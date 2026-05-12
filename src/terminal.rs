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

use crate::app::{AppState, execute_draft, save_draft_if_configured};
use crate::completion::{accept_completion, current_token_context, render_completion_candidates};
use crate::editor::resolve_editor_command;
use crate::picker::{PickerAction, PickerRunResult, file_picker_candidates, run_fzf_picker};
use crate::pty::PtyBackend;
use crate::templates::template_placeholder_spans;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Continue,
    Exit,
    ClearScreen,
    HistorySearchPlaceholder,
    ExternalEditor,
    FilePicker,
    AdvancedKeyPlaceholder(&'static str),
    Submit,
    ShowCompletions,
    AcceptCompletion,
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
            Event::Key(key) if handle_key(key, state, backend, out, command_timeout)? => {
                let _ = save_draft_if_configured(state)?;
                break;
            }
            Event::Paste(text) => {
                if apply_paste_to_state(&text, state) == PasteAction::Submit {
                    execute_draft(state, backend, out, command_timeout)?;
                    if state.exit_requested {
                        return Ok(());
                    }
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
        KeyAction::HistorySearchPlaceholder => {
            writeln!(out, "history search is not implemented yet")?;
        }
        KeyAction::ExternalEditor => {
            run_external_editor(state, backend, out, command_timeout)?;
        }
        KeyAction::FilePicker => {
            run_file_picker(state, out)?;
        }
        KeyAction::AdvancedKeyPlaceholder(name) => {
            writeln!(out, "{name} is not implemented yet")?;
        }
        KeyAction::ShowCompletions => {
            write_completion_candidates(state, out)?;
        }
        KeyAction::AcceptCompletion => {
            accept_first_completion(state)?;
        }
        KeyAction::Submit => {
            writeln!(out)?;
            execute_draft(state, backend, out, command_timeout)?;
            if state.exit_requested {
                return Ok(true);
            }
        }
        KeyAction::Continue => {}
    }
    redraw(state, out)?;
    Ok(false)
}

pub fn run_external_editor(
    state: &mut AppState,
    backend: &mut PtyBackend,
    out: &mut impl Write,
    command_timeout: Duration,
) -> Result<()> {
    let Some(command) = resolve_editor_command(&state.editor_config) else {
        writeln!(out, "editor.resolved=unavailable")?;
        return Ok(());
    };
    let Some(temp_root) = state.editor_temp_root.clone() else {
        writeln!(out, "editor temp directory is not configured")?;
        return Ok(());
    };

    let raw_mode_was_enabled = is_raw_mode_enabled()?;
    if raw_mode_was_enabled {
        disable_raw_mode()?;
    }

    let result = state.run_editor_roundtrip(&temp_root, &command);

    if raw_mode_was_enabled {
        enable_raw_mode()?;
    }

    let result = result?;
    if result.exit_code == Some(0) {
        writeln!(out, "editor saved draft")?;
        if state.editor_config.execute_after_save {
            execute_draft(state, backend, out, command_timeout)?;
        }
    } else {
        writeln!(
            out,
            "editor exited without saving draft: status={}",
            result
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string())
        )?;
    }
    Ok(())
}

pub fn run_file_picker(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let root = state
        .current_cwd
        .clone()
        .unwrap_or(std::env::current_dir()?);
    let candidates = file_picker_candidates(&root)?;
    if candidates.is_empty() {
        writeln!(out, "file picker has no candidates")?;
        return Ok(());
    }

    let result = run_external_picker(|| run_fzf_picker(&candidates))?;
    apply_file_picker_result(state, result, out)
}

fn apply_file_picker_result(
    state: &mut AppState,
    result: PickerRunResult,
    out: &mut impl Write,
) -> Result<()> {
    let Some(selected) = result.selected else {
        writeln!(out, "file picker cancelled")?;
        return Ok(());
    };

    if !state.apply_picker_selection(&selected, PickerAction::ReplaceCurrentToken) {
        writeln!(out, "file picker could not update draft")?;
    }
    Ok(())
}

fn run_external_picker(run: impl FnOnce() -> Result<PickerRunResult>) -> Result<PickerRunResult> {
    let raw_mode_was_enabled = is_raw_mode_enabled()?;
    if raw_mode_was_enabled {
        disable_raw_mode()?;
    }

    let result = run();

    if raw_mode_was_enabled {
        enable_raw_mode()?;
    }

    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteAction {
    Continue,
    Submit,
}

pub fn apply_paste_to_state(text: &str, state: &mut AppState) -> PasteAction {
    if !text.contains('\n') && !text.contains('\r') {
        state.copy_read_only_selection_to_draft();
        if state.draft.is_empty() {
            state.draft_from_editor = false;
            state.draft_from_template = false;
        }
        state.draft.insert_str(text);
        return PasteAction::Continue;
    }

    match state.paste_config.multiline.as_str() {
        "editor" | "execute" if state.paste_config.confirm_execute => {
            state.replace_draft_from_editor_text(normalize_paste_newlines(text));
            PasteAction::Continue
        }
        "execute" => {
            state.replace_draft_from_editor_text(normalize_paste_newlines(text));
            PasteAction::Submit
        }
        _ => PasteAction::Continue,
    }
}

fn normalize_paste_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn apply_key_to_state(key: KeyEvent, state: &mut AppState) -> KeyAction {
    let is_read_only_mode = matches!(
        state.mode,
        crate::modes::Mode::History | crate::modes::Mode::Ai
    );
    let is_editor_draft = state.mode == crate::modes::Mode::Draft && state.draft_from_editor;
    if state.ctrl_x_prefix {
        state.ctrl_x_prefix = false;
        return match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('e')) => KeyAction::ExternalEditor,
            (KeyModifiers::CONTROL, KeyCode::Char('f')) => KeyAction::FilePicker,
            (KeyModifiers::CONTROL, KeyCode::Char('t')) => {
                KeyAction::AdvancedKeyPlaceholder("template picker")
            }
            (KeyModifiers::CONTROL, KeyCode::Char('b')) => {
                KeyAction::AdvancedKeyPlaceholder("git branch picker")
            }
            (KeyModifiers::CONTROL, KeyCode::Char('v')) => {
                KeyAction::AdvancedKeyPlaceholder("environment variable picker")
            }
            _ => KeyAction::Continue,
        };
    }
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('x')) => {
            state.ctrl_x_prefix = true;
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('d')) if state.draft.is_empty() => KeyAction::Exit,
        (KeyModifiers::CONTROL, KeyCode::Char('d')) if is_editor_draft => KeyAction::Continue,
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            if !delete_template_placeholder_after_cursor(state) {
                state.draft.delete();
            }
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            state.draft.clear();
            state.draft_from_editor = false;
            state.draft_from_template = false;
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => KeyAction::ClearScreen,
        (KeyModifiers::CONTROL, KeyCode::Char('r')) => KeyAction::HistorySearchPlaceholder,
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
            if !is_read_only_mode && !is_editor_draft {
                state.draft.move_start();
            }
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
            if !is_read_only_mode && !is_editor_draft {
                state.draft.move_end();
            }
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u' | 'k' | 'w')) if is_editor_draft => {
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            state.copy_read_only_selection_to_draft();
            state.draft.delete_to_start();
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
            state.copy_read_only_selection_to_draft();
            state.draft.delete_to_end();
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
            state.copy_read_only_selection_to_draft();
            state.draft.delete_previous_word();
            KeyAction::Continue
        }
        (KeyModifiers::ALT, KeyCode::Char('b') | KeyCode::Left) => {
            if !is_read_only_mode && !is_editor_draft {
                state.draft.move_previous_word();
            }
            KeyAction::Continue
        }
        (KeyModifiers::ALT, KeyCode::Char('f') | KeyCode::Right) => {
            if !is_read_only_mode && !is_editor_draft {
                state.draft.move_next_word();
            }
            KeyAction::Continue
        }
        (_, KeyCode::Up) if state.mode == crate::modes::Mode::History => {
            state.move_history_selection_older();
            KeyAction::Continue
        }
        (_, KeyCode::Down) if state.mode == crate::modes::Mode::History => {
            state.move_history_selection_newer();
            KeyAction::Continue
        }
        (_, KeyCode::Up) if state.mode == crate::modes::Mode::Ai => {
            state.move_ai_selection_previous();
            KeyAction::Continue
        }
        (_, KeyCode::Down) if state.mode == crate::modes::Mode::Ai => {
            state.move_ai_selection_next();
            KeyAction::Continue
        }
        (_, KeyCode::Left) => {
            if !is_read_only_mode && !is_editor_draft {
                state.draft.move_left();
            }
            KeyAction::Continue
        }
        (_, KeyCode::Right) => {
            if !is_read_only_mode && !is_editor_draft {
                if state.mode == crate::modes::Mode::Draft
                    && state.draft.cursor() == state.draft.as_str().len()
                    && !state.draft.is_empty()
                {
                    return KeyAction::AcceptCompletion;
                }
                state.draft.move_right();
            }
            KeyAction::Continue
        }
        (_, KeyCode::Backspace | KeyCode::Delete | KeyCode::Char(_)) if is_editor_draft => {
            KeyAction::Continue
        }
        (_, KeyCode::Backspace) => {
            state.copy_read_only_selection_to_draft();
            if !delete_template_placeholder_before_cursor(state) {
                expand_template_draft_if_inside_placeholder(state);
                state.draft.backspace();
            }
            if state.draft.is_empty() {
                state.draft_from_editor = false;
                state.draft_from_template = false;
            }
            KeyAction::Continue
        }
        (_, KeyCode::Delete) => {
            state.copy_read_only_selection_to_draft();
            if !delete_template_placeholder_after_cursor(state) {
                expand_template_draft_if_inside_placeholder(state);
                state.draft.delete();
            }
            if state.draft.is_empty() {
                state.draft_from_editor = false;
                state.draft_from_template = false;
            }
            KeyAction::Continue
        }
        (_, KeyCode::Esc) => {
            state.draft.clear();
            state.draft_from_editor = false;
            state.draft_from_template = false;
            state.mode = crate::modes::Mode::Draft;
            KeyAction::Continue
        }
        (_, KeyCode::Tab) => {
            if !state.draft.is_empty() && state.mode == crate::modes::Mode::Draft {
                return KeyAction::ShowCompletions;
            }
            state.handle_empty_tab();
            KeyAction::Continue
        }
        (_, KeyCode::Enter) => KeyAction::Submit,
        (_, KeyCode::Char(ch)) => {
            state.copy_read_only_selection_to_draft();
            if state.draft.is_empty() {
                state.draft_from_editor = false;
                state.draft_from_template = false;
            }
            expand_template_draft_if_inside_placeholder(state);
            state.draft.insert_char(ch);
            KeyAction::Continue
        }
        _ => KeyAction::Continue,
    }
}

fn delete_template_placeholder_before_cursor(state: &mut AppState) -> bool {
    if !state.draft_from_template {
        return false;
    }
    let cursor = state.draft.cursor();
    for span in template_placeholder_spans(state.draft.as_str()) {
        if span.end == cursor {
            return state.draft.drain_range(span.start, span.end);
        }
    }
    false
}

fn delete_template_placeholder_after_cursor(state: &mut AppState) -> bool {
    if !state.draft_from_template {
        return false;
    }
    let cursor = state.draft.cursor();
    for span in template_placeholder_spans(state.draft.as_str()) {
        if span.start == cursor {
            return state.draft.drain_range(span.start, span.end);
        }
    }
    false
}

fn expand_template_draft_if_inside_placeholder(state: &mut AppState) {
    if !state.draft_from_template {
        return;
    }
    let cursor = state.draft.cursor();
    if template_placeholder_spans(state.draft.as_str())
        .into_iter()
        .any(|span| span.start < cursor && cursor < span.end)
    {
        state.draft_from_template = false;
    }
}

pub fn redraw(state: &AppState, out: &mut impl Write) -> Result<()> {
    execute!(out, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
    write!(out, "{}", state.render_prompt_line())?;
    execute!(out, MoveToColumn(state.terminal_cursor_column()))?;
    out.flush()?;
    Ok(())
}

pub fn write_completion_candidates(state: &AppState, out: &mut impl Write) -> Result<()> {
    let candidates = state.completion_candidates()?;
    if candidates.is_empty() {
        writeln!(out, "no completions")?;
        return Ok(());
    }
    for line in render_completion_candidates(&candidates) {
        writeln!(out, "{line}")?;
    }
    Ok(())
}

pub fn accept_first_completion(state: &mut AppState) -> Result<bool> {
    let Some(candidate) = state.completion_candidates()?.into_iter().next() else {
        return Ok(false);
    };
    let token = current_token_context(state.draft.as_str(), state.draft.cursor());
    let accepted = accept_completion(state.draft.as_str(), &token, &candidate);
    if state.draft.replace(accepted.line, accepted.cursor) {
        state.draft_from_template = false;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EditorConfig;
    use crate::modes::Mode;
    use std::path::Path;

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
    }

    #[test]
    fn non_empty_tab_requests_completion_display_without_editing_draft() {
        let mut state = AppState::default();
        state.draft.insert_str("git");

        assert_eq!(
            apply_key_to_state(key(KeyCode::Tab), &mut state),
            KeyAction::ShowCompletions
        );

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git");
    }

    #[test]
    fn write_completion_candidates_prints_labeled_rows() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates/templates.jsonl");
        crate::templates::append_template(
            &template_path,
            &crate::templates::TemplateEntry {
                name: "git-save".to_string(),
                body: "git add . && git commit".to_string(),
            },
        )
        .unwrap();
        let mut state = AppState {
            template_store_path: Some(template_path),
            ..AppState::default()
        };
        state.draft.insert_str("git");
        let mut output = Vec::new();

        write_completion_candidates(&state, &mut output).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("template\tgit-save"));
    }

    #[test]
    fn right_at_end_requests_completion_accept_without_editing_immediately() {
        let mut state = AppState::default();
        state.draft.insert_str("git");

        assert_eq!(
            apply_key_to_state(key(KeyCode::Right), &mut state),
            KeyAction::AcceptCompletion
        );

        assert_eq!(state.draft.as_str(), "git");
    }

    #[test]
    fn right_inside_line_keeps_cursor_movement_behavior() {
        let mut state = AppState::default();
        state.draft.insert_str("git status");
        state.draft.move_start();

        assert_eq!(
            apply_key_to_state(key(KeyCode::Right), &mut state),
            KeyAction::Continue
        );

        assert_eq!(state.draft.cursor(), 1);
    }

    #[test]
    fn accept_first_completion_replaces_current_token() {
        let mut state = AppState {
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "git status".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            ..AppState::default()
        };
        state.draft.insert_str("git sta");

        assert!(accept_first_completion(&mut state).unwrap());

        assert_eq!(state.draft.as_str(), "git status");
        assert_eq!(state.draft.cursor(), "git status".len());
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

    #[test]
    fn esc_clears_draft_and_returns_to_draft_mode() {
        let mut state = AppState {
            mode: Mode::History,
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "git status".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            selected_history_index: Some(0),
            ..AppState::default()
        };
        state.draft.insert_str("partial");

        assert_eq!(
            apply_key_to_state(key(KeyCode::Esc), &mut state),
            KeyAction::Continue
        );

        assert_eq!(state.mode, Mode::Draft);
        assert!(state.draft.is_empty());
        assert_eq!(state.selected_history_index, Some(0));
    }

    #[test]
    fn ctrl_r_returns_history_search_placeholder_without_editing_draft() {
        let mut state = AppState::default();
        state.draft.insert_str("git status");

        assert_eq!(
            apply_key_to_state(ctrl('r'), &mut state),
            KeyAction::HistorySearchPlaceholder
        );

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git status");
    }

    #[test]
    fn ctrl_x_prefix_resolves_editor_chord_to_launch_action() {
        let mut state = AppState::default();
        state.draft.insert_str("git status");

        assert_eq!(
            apply_key_to_state(ctrl('x'), &mut state),
            KeyAction::Continue
        );
        assert!(state.ctrl_x_prefix);
        assert_eq!(
            apply_key_to_state(ctrl('e'), &mut state),
            KeyAction::ExternalEditor
        );

        assert!(!state.ctrl_x_prefix);
        assert_eq!(state.draft.as_str(), "git status");
    }

    #[test]
    fn ctrl_x_prefix_resolves_file_picker_chord_to_launch_action() {
        let mut state = AppState::default();
        state.draft.insert_str("cat old.txt");

        apply_key_to_state(ctrl('x'), &mut state);

        assert_eq!(
            apply_key_to_state(ctrl('f'), &mut state),
            KeyAction::FilePicker
        );
        assert!(!state.ctrl_x_prefix);
        assert_eq!(state.draft.as_str(), "cat old.txt");
    }

    #[test]
    fn ctrl_x_prefix_resolves_other_advanced_chords_to_placeholders() {
        for (ch, name) in [
            ('t', "template picker"),
            ('b', "git branch picker"),
            ('v', "environment variable picker"),
        ] {
            let mut state = AppState::default();
            state.draft.insert_str("git status");

            apply_key_to_state(ctrl('x'), &mut state);

            assert_eq!(
                apply_key_to_state(ctrl(ch), &mut state),
                KeyAction::AdvancedKeyPlaceholder(name)
            );
            assert!(!state.ctrl_x_prefix);
            assert_eq!(state.draft.as_str(), "git status");
        }
    }

    #[test]
    fn apply_file_picker_result_replaces_current_token() {
        let mut state = AppState::default();
        state.draft.insert_str("cat old.txt");
        state.draft.move_left();
        state.draft.move_left();
        state.draft.move_left();
        let mut output = Vec::new();

        apply_file_picker_result(
            &mut state,
            PickerRunResult {
                selected: Some("new file.txt".to_string()),
                exit_code: Some(0),
            },
            &mut output,
        )
        .unwrap();

        assert_eq!(state.draft.as_str(), "cat 'new file.txt'");
        assert!(String::from_utf8(output).unwrap().is_empty());
    }

    #[test]
    fn apply_file_picker_result_reports_cancel_without_editing() {
        let mut state = AppState::default();
        state.draft.insert_str("cat old.txt");
        let mut output = Vec::new();

        apply_file_picker_result(
            &mut state,
            PickerRunResult {
                selected: None,
                exit_code: Some(130),
            },
            &mut output,
        )
        .unwrap();

        assert_eq!(state.draft.as_str(), "cat old.txt");
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "file picker cancelled\n"
        );
    }

    #[test]
    fn ctrl_x_prefix_cancels_on_unknown_chord_without_editing_draft() {
        let mut state = AppState::default();
        state.draft.insert_str("git status");

        apply_key_to_state(ctrl('x'), &mut state);
        assert_eq!(
            apply_key_to_state(ctrl('q'), &mut state),
            KeyAction::Continue
        );

        assert!(!state.ctrl_x_prefix);
        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git status");
    }

    #[test]
    fn run_external_editor_replaces_draft_after_success() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("fake-editor.sh");
        std::fs::write(&script, "#!/bin/sh\nprintf 'echo edited' > \"$1\"\n").unwrap();
        make_executable(&script);
        let mut state = AppState {
            editor_config: EditorConfig {
                command: vec![script.display().to_string()],
                execute_after_save: false,
            },
            editor_temp_root: Some(temp.path().join("editor")),
            ..AppState::default()
        };
        state.draft.insert_str("old draft");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        run_external_editor(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        assert_eq!(state.draft.as_str(), "echo edited");
        assert_eq!(state.draft.cursor(), "echo edited".len());
        assert_eq!(String::from_utf8(output).unwrap(), "editor saved draft\n");
    }

    #[test]
    fn run_external_editor_keeps_draft_after_editor_failure() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("fake-editor.sh");
        std::fs::write(&script, "#!/bin/sh\nprintf changed > \"$1\"\nexit 4\n").unwrap();
        make_executable(&script);
        let mut state = AppState {
            editor_config: EditorConfig {
                command: vec![script.display().to_string()],
                execute_after_save: false,
            },
            editor_temp_root: Some(temp.path().join("editor")),
            ..AppState::default()
        };
        state.draft.insert_str("old draft");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        run_external_editor(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        assert_eq!(state.draft.as_str(), "old draft");
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "editor exited without saving draft: status=4\n"
        );
    }

    #[test]
    fn run_external_editor_reports_missing_editor() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState {
            editor_config: EditorConfig {
                command: vec!["/definitely/missing/aish-editor".to_string()],
                execute_after_save: false,
            },
            editor_temp_root: Some(temp.path().join("editor")),
            ..AppState::default()
        };
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        let error = run_external_editor(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap_err();

        assert!(error.to_string().contains("failed to run editor command"));
        assert!(state.draft.is_empty());
    }

    #[test]
    fn run_external_editor_executes_after_save_when_configured() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("fake-editor.sh");
        let marker = temp.path().join("auto-ran");
        std::fs::write(
            &script,
            format!("#!/bin/sh\nprintf 'touch {}' > \"$1\"\n", marker.display()),
        )
        .unwrap();
        make_executable(&script);
        let mut state = AppState {
            editor_config: EditorConfig {
                command: vec![script.display().to_string()],
                execute_after_save: true,
            },
            editor_temp_root: Some(temp.path().join("editor")),
            ..AppState::default()
        };
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        run_external_editor(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        assert!(marker.exists());
        assert_eq!(state.last_status, Some(0));
        assert!(state.draft.is_empty());
        assert_eq!(String::from_utf8(output).unwrap(), "editor saved draft\n");
    }

    #[test]
    fn editor_draft_ignores_inline_editing_keys() {
        let mut state = AppState::default();
        state.draft.insert_str("echo one\necho two");
        state.draft_from_editor = true;

        apply_key_to_state(key(KeyCode::Char('x')), &mut state);
        apply_key_to_state(key(KeyCode::Backspace), &mut state);
        apply_key_to_state(ctrl('u'), &mut state);
        apply_key_to_state(key(KeyCode::Left), &mut state);

        assert_eq!(state.draft.as_str(), "echo one\necho two");
        assert!(state.draft_from_editor);
    }

    #[test]
    fn template_draft_backspace_deletes_placeholder_from_outside() {
        let mut state = AppState {
            draft_from_template: true,
            ..AppState::default()
        };
        state.draft.insert_str("echo {name} now");
        state.draft.move_left();
        state.draft.move_left();
        state.draft.move_left();
        state.draft.move_left();

        apply_key_to_state(key(KeyCode::Backspace), &mut state);

        assert_eq!(state.draft.as_str(), "echo  now");
        assert_eq!(state.draft.cursor(), 5);
        assert!(state.draft_from_template);
    }

    #[test]
    fn template_draft_delete_deletes_placeholder_from_outside() {
        let mut state = AppState {
            draft_from_template: true,
            ..AppState::default()
        };
        state.draft.insert_str("echo {name} now");
        state.draft.move_start();
        for _ in 0..5 {
            state.draft.move_right();
        }

        apply_key_to_state(key(KeyCode::Delete), &mut state);

        assert_eq!(state.draft.as_str(), "echo  now");
        assert_eq!(state.draft.cursor(), 5);
        assert!(state.draft_from_template);
    }

    #[test]
    fn template_draft_edit_inside_placeholder_expands_to_plain_draft() {
        let mut state = AppState {
            draft_from_template: true,
            ..AppState::default()
        };
        state.draft.insert_str("echo {name}");
        state.draft.move_start();
        for _ in 0..7 {
            state.draft.move_right();
        }

        apply_key_to_state(key(KeyCode::Char('X')), &mut state);

        assert_eq!(state.draft.as_str(), "echo {nXame}");
        assert!(!state.draft_from_template);
    }

    #[test]
    fn normalize_paste_newlines_canonicalizes_crlf_and_cr() {
        assert_eq!(
            normalize_paste_newlines("one\r\ntwo\rthree"),
            "one\ntwo\nthree"
        );
    }

    #[test]
    fn single_line_paste_inserts_into_draft() {
        let mut state = AppState::default();
        state.draft.insert_str("git ");

        assert_eq!(
            apply_paste_to_state("status", &mut state),
            PasteAction::Continue
        );

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git status");
        assert!(!state.draft_from_editor);
    }

    #[test]
    fn single_line_paste_copies_history_selection_first() {
        let mut state = AppState {
            mode: Mode::History,
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "git statu".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            selected_history_index: Some(0),
            ..AppState::default()
        };

        assert_eq!(apply_paste_to_state("s", &mut state), PasteAction::Continue);

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git status");
        assert!(!state.draft_from_editor);
    }

    #[test]
    fn multiline_paste_creates_opaque_editor_draft() {
        let mut state = AppState::default();

        assert_eq!(
            apply_paste_to_state("echo one\r\necho two", &mut state),
            PasteAction::Continue
        );

        assert_eq!(state.mode, Mode::Draft);
        assert!(state.draft_from_editor);
        assert_eq!(state.draft.as_str(), "echo one\necho two");
        assert!(
            state
                .render_prompt_line()
                .contains("[editor draft: 2 line(s)")
        );
    }

    #[test]
    fn multiline_paste_discard_config_ignores_content() {
        let mut state = AppState {
            paste_config: crate::config::PasteConfig {
                multiline: "discard".to_string(),
                confirm_execute: true,
            },
            ..AppState::default()
        };
        state.draft.insert_str("existing");

        assert_eq!(
            apply_paste_to_state("echo one\necho two", &mut state),
            PasteAction::Continue
        );

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "existing");
        assert!(!state.draft_from_editor);
    }

    #[test]
    fn multiline_paste_execute_with_confirm_creates_editor_draft() {
        let mut state = AppState {
            paste_config: crate::config::PasteConfig {
                multiline: "execute".to_string(),
                confirm_execute: true,
            },
            ..AppState::default()
        };

        assert_eq!(
            apply_paste_to_state("echo one\necho two", &mut state),
            PasteAction::Continue
        );

        assert!(state.draft_from_editor);
        assert_eq!(state.draft.as_str(), "echo one\necho two");
    }

    #[test]
    fn multiline_paste_execute_without_confirm_requests_submit() {
        let mut state = AppState {
            paste_config: crate::config::PasteConfig {
                multiline: "execute".to_string(),
                confirm_execute: false,
            },
            ..AppState::default()
        };

        assert_eq!(
            apply_paste_to_state("echo one\necho two", &mut state),
            PasteAction::Submit
        );

        assert!(state.draft_from_editor);
        assert_eq!(state.draft.as_str(), "echo one\necho two");
    }

    #[test]
    fn history_mode_up_down_browses_without_editing_draft() {
        let mut state = AppState {
            mode: Mode::History,
            regular_history: vec![
                crate::history::HistoryEntry {
                    t: 1,
                    command: "one".to_string(),
                    exit_code: Some(0),
                    source: crate::history::HistorySource::User,
                },
                crate::history::HistoryEntry {
                    t: 2,
                    command: "two".to_string(),
                    exit_code: Some(0),
                    source: crate::history::HistorySource::User,
                },
            ],
            selected_history_index: Some(0),
            ..AppState::default()
        };

        apply_key_to_state(key(KeyCode::Up), &mut state);
        assert_eq!(state.mode, Mode::History);
        assert_eq!(state.selected_history_command(), Some("one"));
        assert!(state.draft.is_empty());

        apply_key_to_state(key(KeyCode::Down), &mut state);
        assert_eq!(state.selected_history_command(), Some("two"));
        assert!(state.draft.is_empty());
    }

    #[test]
    fn history_mode_typing_copies_selection_to_draft_then_edits() {
        let mut state = AppState {
            mode: Mode::History,
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "git statu".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            selected_history_index: Some(0),
            ..AppState::default()
        };

        apply_key_to_state(key(KeyCode::Char('s')), &mut state);

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git status");
    }

    #[test]
    fn history_mode_cursor_movement_does_not_copy_to_draft() {
        let mut state = AppState {
            mode: Mode::History,
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "git status".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            selected_history_index: Some(0),
            ..AppState::default()
        };

        apply_key_to_state(key(KeyCode::Left), &mut state);
        apply_key_to_state(key(KeyCode::Right), &mut state);
        apply_key_to_state(ctrl('a'), &mut state);
        apply_key_to_state(ctrl('e'), &mut state);

        assert_eq!(state.mode, Mode::History);
        assert!(state.draft.is_empty());
        assert_eq!(state.selected_history_command(), Some("git status"));
    }

    #[test]
    fn ai_mode_up_down_browses_without_editing_draft() {
        let mut state = AppState {
            mode: Mode::Ai,
            ai_sessions: vec![crate::history::AiSession {
                id: "a_1".to_string(),
                t: 1,
                prompt: "commands".to_string(),
                ctx: false,
                model: "test".to_string(),
                items: vec![
                    crate::history::AiItem {
                        kind: crate::history::AiItemKind::Command,
                        text: "one".to_string(),
                        name: None,
                    },
                    crate::history::AiItem {
                        kind: crate::history::AiItemKind::Command,
                        text: "two".to_string(),
                        name: None,
                    },
                ],
            }],
            ai_command_indices: vec![
                crate::history::AiCommandIndex {
                    session_index: 0,
                    item_index: 0,
                },
                crate::history::AiCommandIndex {
                    session_index: 0,
                    item_index: 1,
                },
            ],
            selected_ai_index: Some(0),
            ..AppState::default()
        };

        apply_key_to_state(key(KeyCode::Down), &mut state);
        assert_eq!(state.mode, Mode::Ai);
        assert_eq!(state.selected_ai_command(), Some("two"));
        assert!(state.draft.is_empty());

        apply_key_to_state(key(KeyCode::Up), &mut state);
        assert_eq!(state.selected_ai_command(), Some("one"));
        assert!(state.draft.is_empty());
    }

    #[test]
    fn ai_mode_typing_copies_selection_to_draft_then_edits() {
        let mut state = AppState {
            mode: Mode::Ai,
            ai_sessions: vec![crate::history::AiSession {
                id: "a_1".to_string(),
                t: 1,
                prompt: "commands".to_string(),
                ctx: false,
                model: "test".to_string(),
                items: vec![crate::history::AiItem {
                    kind: crate::history::AiItemKind::Command,
                    text: "git statu".to_string(),
                    name: None,
                }],
            }],
            ai_command_indices: vec![crate::history::AiCommandIndex {
                session_index: 0,
                item_index: 0,
            }],
            selected_ai_index: Some(0),
            ..AppState::default()
        };

        apply_key_to_state(key(KeyCode::Char('s')), &mut state);

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git status");
    }

    #[test]
    fn ai_mode_cursor_movement_does_not_copy_to_draft() {
        let mut state = AppState {
            mode: Mode::Ai,
            ai_sessions: vec![crate::history::AiSession {
                id: "a_1".to_string(),
                t: 1,
                prompt: "commands".to_string(),
                ctx: false,
                model: "test".to_string(),
                items: vec![crate::history::AiItem {
                    kind: crate::history::AiItemKind::Command,
                    text: "git status".to_string(),
                    name: None,
                }],
            }],
            ai_command_indices: vec![crate::history::AiCommandIndex {
                session_index: 0,
                item_index: 0,
            }],
            selected_ai_index: Some(0),
            ..AppState::default()
        };

        apply_key_to_state(key(KeyCode::Left), &mut state);
        apply_key_to_state(key(KeyCode::Right), &mut state);
        apply_key_to_state(ctrl('a'), &mut state);
        apply_key_to_state(ctrl('e'), &mut state);

        assert_eq!(state.mode, Mode::Ai);
        assert!(state.draft.is_empty());
        assert_eq!(state.selected_ai_command(), Some("git status"));
    }

    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(path, permissions).unwrap();
        }
    }
}
