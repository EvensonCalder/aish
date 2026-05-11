use std::io::{Write, stdout};
use std::panic;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::MoveToColumn;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, disable_raw_mode, enable_raw_mode, is_raw_mode_enabled,
};

use crate::app::{AppState, execute_draft};
use crate::pty::PtyBackend;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Continue,
    Exit,
    ClearScreen,
    Submit,
}

pub struct TerminalGuard;

impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), crossterm::event::EnableBracketedPaste)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), crossterm::event::DisableBracketedPaste);
        if is_raw_mode_enabled().unwrap_or(false) {
            let _ = disable_raw_mode();
        }
    }
}

pub fn install_panic_cleanup() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), crossterm::event::DisableBracketedPaste);
        let _ = disable_raw_mode();
        previous(info);
    }));
}

pub fn run(
    state: &mut AppState,
    backend: &mut PtyBackend,
    out: &mut impl Write,
    command_timeout: Duration,
) -> Result<()> {
    install_panic_cleanup();
    let _guard = TerminalGuard::enter()?;
    redraw(state, out)?;

    loop {
        match event::read()? {
            Event::Key(key) if handle_key(key, state, backend, out, command_timeout)? => break,
            Event::Paste(text) => {
                if !text.contains('\n') && !text.contains('\r') {
                    state.draft.insert_str(&text);
                }
                redraw(state, out)?;
            }
            _ => {}
        }
    }

    Ok(())
}

fn handle_key(
    key: KeyEvent,
    state: &mut AppState,
    backend: &mut PtyBackend,
    out: &mut impl Write,
    command_timeout: Duration,
) -> Result<bool> {
    match apply_key_to_state(key, state) {
        KeyAction::Exit => return Ok(true),
        KeyAction::ClearScreen => {
            execute!(out, Clear(ClearType::All), MoveToColumn(0))?;
        }
        KeyAction::Submit => {
            writeln!(out)?;
            execute_draft(state, backend, out, command_timeout)?;
        }
        KeyAction::Continue => {}
    }
    redraw(state, out)?;
    Ok(false)
}

pub fn apply_key_to_state(key: KeyEvent, state: &mut AppState) -> KeyAction {
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('d')) if state.draft.is_empty() => KeyAction::Exit,
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            state.draft.delete();
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            state.draft.clear();
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => KeyAction::ClearScreen,
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
            state.draft.move_start();
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
            state.draft.move_end();
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            state.draft.delete_to_start();
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
            state.draft.delete_to_end();
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
            state.draft.delete_previous_word();
            KeyAction::Continue
        }
        (KeyModifiers::ALT, KeyCode::Char('b') | KeyCode::Left) => {
            state.draft.move_previous_word();
            KeyAction::Continue
        }
        (KeyModifiers::ALT, KeyCode::Char('f') | KeyCode::Right) => {
            state.draft.move_next_word();
            KeyAction::Continue
        }
        (_, KeyCode::Left) => {
            state.draft.move_left();
            KeyAction::Continue
        }
        (_, KeyCode::Right) => {
            state.draft.move_right();
            KeyAction::Continue
        }
        (_, KeyCode::Backspace) => {
            state.draft.backspace();
            KeyAction::Continue
        }
        (_, KeyCode::Delete) => {
            state.draft.delete();
            KeyAction::Continue
        }
        (_, KeyCode::Tab) => {
            state.handle_empty_tab();
            KeyAction::Continue
        }
        (_, KeyCode::Enter) => KeyAction::Submit,
        (_, KeyCode::Char(ch)) => {
            state.draft.insert_char(ch);
            KeyAction::Continue
        }
        _ => KeyAction::Continue,
    }
}

pub fn redraw(state: &AppState, out: &mut impl Write) -> Result<()> {
    execute!(out, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
    write!(out, "{}", state.render_prompt_line())?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modes::Mode;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    fn alt(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::ALT)
    }

    #[test]
    fn printable_keys_edit_draft_at_cursor() {
        let mut state = AppState::default();
        apply_key_to_state(key(KeyCode::Char('a')), &mut state);
        apply_key_to_state(key(KeyCode::Char('c')), &mut state);
        apply_key_to_state(key(KeyCode::Left), &mut state);
        apply_key_to_state(key(KeyCode::Char('b')), &mut state);

        assert_eq!(state.draft.as_str(), "abc");
        assert_eq!(state.draft.cursor(), 2);
    }

    #[test]
    fn control_navigation_and_deletion_update_draft() {
        let mut state = AppState::default();
        state.draft.insert_str("cargo test --all");

        assert_eq!(
            apply_key_to_state(ctrl('a'), &mut state),
            KeyAction::Continue
        );
        assert_eq!(state.draft.cursor(), 0);
        apply_key_to_state(ctrl('e'), &mut state);
        assert_eq!(state.draft.cursor(), state.draft.as_str().len());
        apply_key_to_state(ctrl('w'), &mut state);
        assert_eq!(state.draft.as_str(), "cargo test ");
        apply_key_to_state(ctrl('u'), &mut state);
        assert_eq!(state.draft.as_str(), "");
    }

    #[test]
    fn alt_word_navigation_moves_by_tokens() {
        let mut state = AppState::default();
        state.draft.insert_str("git commit message");

        apply_key_to_state(ctrl('a'), &mut state);
        apply_key_to_state(alt('f'), &mut state);
        assert_eq!(state.draft.cursor(), 4);
        apply_key_to_state(alt('f'), &mut state);
        assert_eq!(state.draft.cursor(), 11);
        apply_key_to_state(alt('b'), &mut state);
        assert_eq!(state.draft.cursor(), 4);
    }

    #[test]
    fn tab_switches_mode_only_for_empty_draft() {
        let mut state = AppState::default();
        apply_key_to_state(key(KeyCode::Tab), &mut state);
        assert_eq!(state.mode, Mode::History);

        state.draft.insert_str("git");
        apply_key_to_state(key(KeyCode::Tab), &mut state);
        assert_eq!(state.mode, Mode::History);
    }

    #[test]
    fn enter_and_empty_ctrl_d_return_actions() {
        let mut state = AppState::default();
        assert_eq!(
            apply_key_to_state(key(KeyCode::Enter), &mut state),
            KeyAction::Submit
        );
        assert_eq!(apply_key_to_state(ctrl('d'), &mut state), KeyAction::Exit);

        state.draft.insert_str("x");
        assert_eq!(
            apply_key_to_state(ctrl('d'), &mut state),
            KeyAction::Continue
        );
    }
}
