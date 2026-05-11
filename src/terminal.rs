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
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('d')) if state.draft.is_empty() => return Ok(true),
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            state.draft.delete();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            state.draft.clear();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
            execute!(out, Clear(ClearType::All), MoveToColumn(0))?;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => state.draft.move_start(),
        (KeyModifiers::CONTROL, KeyCode::Char('e')) => state.draft.move_end(),
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => state.draft.delete_to_start(),
        (KeyModifiers::CONTROL, KeyCode::Char('k')) => state.draft.delete_to_end(),
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
            state.draft.delete_previous_word();
        }
        (KeyModifiers::ALT, KeyCode::Char('b') | KeyCode::Left) => {
            state.draft.move_previous_word();
        }
        (KeyModifiers::ALT, KeyCode::Char('f') | KeyCode::Right) => {
            state.draft.move_next_word();
        }
        (_, KeyCode::Left) => {
            state.draft.move_left();
        }
        (_, KeyCode::Right) => {
            state.draft.move_right();
        }
        (_, KeyCode::Backspace) => {
            state.draft.backspace();
        }
        (_, KeyCode::Delete) => {
            state.draft.delete();
        }
        (_, KeyCode::Tab) => state.handle_empty_tab(),
        (_, KeyCode::Enter) => {
            writeln!(out)?;
            execute_draft(state, backend, out, command_timeout)?;
        }
        (_, KeyCode::Char(ch)) => state.draft.insert_char(ch),
        _ => {}
    }
    redraw(state, out)?;
    Ok(false)
}

pub fn redraw(state: &AppState, out: &mut impl Write) -> Result<()> {
    execute!(out, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
    write!(out, "{} {}", state.mode.symbol(), state.draft.as_str())?;
    out.flush()?;
    Ok(())
}
