use std::io::{Result as IoResult, Write, stdout};
use std::panic;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::{
    MoveDown, MoveTo, MoveToColumn, MoveToPreviousLine, RestorePosition, SavePosition,
};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, disable_raw_mode, enable_raw_mode, is_raw_mode_enabled, size,
};

use crate::app::{
    AppState, InlineCompletion, answer_context_confirmation, execute_draft,
    save_draft_if_configured,
};
use crate::completion::{
    CompletionCandidate, accept_completion_with_mode, current_token_context,
    ghost_completion_suffix, limit_candidates, render_completion_candidates_for_width,
    truncate_with_ellipsis,
};
use crate::config::CompletionMode;
use crate::display_width::{visual_line_count, visual_position};
use crate::editor::resolve_editor_command;
use crate::picker::{
    PickerAction, PickerRunResult, env_var_picker_candidates, file_picker_candidates,
    git_branch_picker_candidates, run_fzf_picker, shell_env_var_reference,
};
use crate::pty::{PtyBackend, pty_size};
use crate::shell_integration::passthrough_key_bytes;
use crate::templates::template_placeholder_spans;

const FRONTEND_TICK_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    Continue,
    Exit,
    ClearScreen,
    HistorySearch,
    ExternalEditor,
    FilePicker,
    TemplatePicker,
    GitBranchPicker,
    EnvVarPicker,
    AdvancedKeyPlaceholder(&'static str),
    Submit,
    ConfirmContext(bool),
    CompleteOrShow,
    AcceptCompletion,
    PreviousDraft,
    NextDraft,
    ForwardToBackend(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    Key(KeyEvent),
    Paste(String),
    Resize(u16, u16),
    Tick,
    Ignore,
}

pub struct TerminalGuard;

pub struct CrLfWriter<'a, W: Write> {
    inner: &'a mut W,
    previous_was_cr: bool,
}

impl<'a, W: Write> CrLfWriter<'a, W> {
    fn new(inner: &'a mut W) -> Self {
        Self {
            inner,
            previous_was_cr: false,
        }
    }
}

impl<W: Write> Write for CrLfWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        for &byte in buf {
            if byte == b'\n' && !self.previous_was_cr {
                self.inner.write_all(b"\r")?;
            }
            self.inner.write_all(&[byte])?;
            self.previous_was_cr = byte == b'\r';
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        self.inner.flush()
    }
}

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

fn read_frontend_event(tick_interval: Duration) -> Result<TerminalEvent> {
    if !event::poll(tick_interval)? {
        return Ok(TerminalEvent::Tick);
    }
    Ok(terminal_event_from_crossterm(event::read()?))
}

fn terminal_event_from_crossterm(event: Event) -> TerminalEvent {
    match event {
        Event::Key(key) => TerminalEvent::Key(key),
        Event::Paste(text) => TerminalEvent::Paste(text),
        Event::Resize(cols, rows) => TerminalEvent::Resize(cols, rows),
        _ => TerminalEvent::Ignore,
    }
}

pub fn run(
    state: &mut AppState,
    backend: &mut PtyBackend,
    out: &mut impl Write,
    command_timeout: Duration,
) -> Result<()> {
    install_panic_cleanup();
    let _guard = TerminalGuard::enter()?;
    sync_backend_pty_size(backend)?;
    refresh_live_completion_ui(state)?;
    redraw(state, out)?;

    loop {
        match read_frontend_event(FRONTEND_TICK_INTERVAL)? {
            TerminalEvent::Key(key) => {
                if handle_key(key, state, backend, out, command_timeout)? {
                    persist_draft_and_flush_before_exit(state)?;
                    break;
                }
            }
            TerminalEvent::Paste(text) => {
                if apply_paste_to_state(&text, state) == PasteAction::Submit {
                    let mut display_out = CrLfWriter::new(out);
                    execute_draft(state, backend, &mut display_out, command_timeout)?;
                    if state.exit_requested {
                        persist_draft_and_flush_before_exit(state)?;
                        return Ok(());
                    }
                }
                refresh_live_completion_ui(state)?;
                redraw(state, out)?;
            }
            TerminalEvent::Resize(cols, rows) => {
                backend.resize(pty_size(cols, rows))?;
                redraw(state, out)?;
            }
            TerminalEvent::Tick | TerminalEvent::Ignore => {
                refresh_after_background_events(state, out)?;
            }
        }
    }

    Ok(())
}

fn persist_draft_and_flush_before_exit(state: &AppState) -> Result<()> {
    let _ = save_draft_if_configured(state)?;
    state.flush_encrypted_writes()
}

fn refresh_after_background_events(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let mut should_redraw = false;
    if let Some(candidates) = state.drain_live_completion_events()
        && should_apply_completion_event_update(state)
    {
        should_redraw |=
            replace_completion_ui_from_candidates(state, candidates, terminal_display_width());
    }
    if state.drain_encrypted_write_events() {
        refresh_live_completion_ui(state)?;
        should_redraw = true;
    }
    if should_redraw {
        redraw(state, out)?;
    }
    Ok(())
}

fn should_apply_completion_event_update(state: &AppState) -> bool {
    state.completion_config.enabled
        && state.completion_config.mode() != CompletionMode::Off
        && state.pending_context.is_none()
        && !state.ctrl_x_prefix
        && state.mode == crate::modes::Mode::Draft
        && !state.draft_from_editor
        && !state.draft.is_empty()
        && state.draft.cursor() == state.draft.as_str().len()
}

fn sync_backend_pty_size(backend: &mut PtyBackend) -> Result<()> {
    let (cols, rows) = size()?;
    backend.resize(pty_size(cols, rows))
}

fn handle_key(
    key: KeyEvent,
    state: &mut AppState,
    backend: &mut PtyBackend,
    out: &mut impl Write,
    command_timeout: Duration,
) -> Result<bool> {
    let had_completion_ui = !state.completion_panel.is_empty() || state.completion_inline.is_some();
    let action = apply_key_to_state(key, state);
    let refresh_completion = !matches!(action, KeyAction::CompleteOrShow);
    match action {
        KeyAction::Exit => return Ok(true),
        KeyAction::ClearScreen => {
            clear_screen_for_redraw(state, out)?;
        }
        KeyAction::HistorySearch => {
            let mut display_out = CrLfWriter::new(out);
            run_history_picker(state, &mut display_out)?;
        }
        KeyAction::ExternalEditor => {
            let mut display_out = CrLfWriter::new(out);
            run_external_editor(state, backend, &mut display_out, command_timeout)?;
        }
        KeyAction::FilePicker => {
            let mut display_out = CrLfWriter::new(out);
            run_file_picker(state, &mut display_out)?;
        }
        KeyAction::TemplatePicker => {
            let mut display_out = CrLfWriter::new(out);
            run_template_picker(state, &mut display_out)?;
        }
        KeyAction::GitBranchPicker => {
            let mut display_out = CrLfWriter::new(out);
            run_git_branch_picker(state, &mut display_out)?;
        }
        KeyAction::EnvVarPicker => {
            let mut display_out = CrLfWriter::new(out);
            run_env_var_picker(state, backend, &mut display_out)?;
        }
        KeyAction::AdvancedKeyPlaceholder(name) => {
            invalidate_render_anchor(state);
            let mut display_out = CrLfWriter::new(out);
            writeln!(display_out, "{name} is not implemented yet")?;
        }
        KeyAction::CompleteOrShow => {
            complete_or_show_candidates(state)?;
        }
        KeyAction::AcceptCompletion => {
            accept_first_completion(state)?;
        }
        KeyAction::PreviousDraft => {
            state.move_draft_selection_older()?;
        }
        KeyAction::NextDraft => {
            state.move_draft_selection_newer()?;
        }
        KeyAction::Submit => {
            if had_completion_ui {
                redraw(state, out)?;
            }
            let open_ai_editor = state.mode == crate::modes::Mode::Draft
                && !state.draft_from_editor
                && state
                    .draft
                    .as_str()
                    .strip_prefix("# ")
                    .is_some_and(|prompt| prompt.trim().is_empty());
            if open_ai_editor {
                let mut display_out = CrLfWriter::new(out);
                run_external_editor(state, backend, &mut display_out, command_timeout)?;
            } else {
                move_to_rendered_end(state, out, terminal_display_width())?;
                invalidate_render_anchor(state);
                write!(out, "\r\n")?;
                let mut display_out = CrLfWriter::new(out);
                execute_draft(state, backend, &mut display_out, command_timeout)?;
                if state.exit_requested {
                    return Ok(true);
                }
            }
        }
        KeyAction::ConfirmContext(accepted) => {
            move_to_rendered_end(state, out, terminal_display_width())?;
            invalidate_render_anchor(state);
            write!(out, "\r\n")?;
            let mut display_out = CrLfWriter::new(out);
            answer_context_confirmation(state, accepted, &mut display_out, command_timeout)?;
        }
        KeyAction::ForwardToBackend(bytes) => {
            backend.write_raw(&bytes)?;
            return Ok(false);
        }
        KeyAction::Continue => {}
    }
    if refresh_completion {
        refresh_live_completion_ui(state)?;
    }
    redraw(state, out)?;
    Ok(false)
}

fn clear_screen_for_redraw(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    invalidate_render_anchor(state);
    execute!(
        out,
        MoveTo(0, 0),
        Clear(ClearType::All),
        Clear(ClearType::Purge),
        MoveTo(0, 0)
    )?;
    Ok(())
}

pub fn run_external_editor(
    state: &mut AppState,
    backend: &mut PtyBackend,
    out: &mut impl Write,
    command_timeout: Duration,
) -> Result<()> {
    let Some(command) = resolve_editor_command(&state.editor_config) else {
        invalidate_render_anchor(state);
        writeln!(out, "editor.resolved=unavailable")?;
        return Ok(());
    };
    let Some(temp_root) = state.editor_temp_root.clone() else {
        invalidate_render_anchor(state);
        writeln!(out, "editor temp directory is not configured")?;
        return Ok(());
    };

    let raw_mode_was_enabled = is_raw_mode_enabled()?;
    if raw_mode_was_enabled {
        disable_raw_mode()?;
    }

    let is_ai_prompt_editor = state.should_open_ai_prompt_editor();
    let result = if is_ai_prompt_editor {
        state.run_ai_prompt_editor_roundtrip(&temp_root, &command)
    } else {
        state.run_editor_roundtrip(&temp_root, &command)
    };

    if raw_mode_was_enabled {
        enable_raw_mode()?;
    }

    let result = result?;
    if result.exit_code == Some(0) {
        invalidate_render_anchor(state);
        if state.draft_from_editor {
            writeln!(out, "editor saved draft")?;
        } else {
            writeln!(out, "editor empty; canceled")?;
        }
        if state.editor_config.execute_after_save
            && state.draft_from_editor
            && !state.draft_from_ai_editor
        {
            execute_draft(state, backend, out, command_timeout)?;
        }
    } else {
        invalidate_render_anchor(state);
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
        invalidate_render_anchor(state);
        writeln!(out, "file picker has no candidates")?;
        return Ok(());
    }

    let result = run_external_picker(|| run_fzf_picker(&candidates))?;
    apply_file_picker_result(state, result, out)
}

pub fn run_history_picker(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let candidates = state.history_picker_candidates();
    if candidates.is_empty() {
        invalidate_render_anchor(state);
        writeln!(out, "history search has no candidates")?;
        return Ok(());
    }

    let result = run_external_picker(|| run_fzf_picker(&candidates))?;
    apply_history_picker_result(state, result, out)
}

pub fn run_template_picker(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let candidates = state.template_picker_candidates()?;
    if candidates.is_empty() {
        invalidate_render_anchor(state);
        writeln!(out, "template picker has no candidates")?;
        return Ok(());
    }

    let result = run_external_picker(|| run_fzf_picker(&candidates))?;
    apply_template_picker_result(state, result, out)
}

pub fn run_git_branch_picker(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let root = state
        .current_cwd
        .clone()
        .unwrap_or(std::env::current_dir()?);
    let candidates = git_branch_picker_candidates(&root)?;
    if candidates.is_empty() {
        invalidate_render_anchor(state);
        writeln!(out, "git branch picker has no candidates")?;
        return Ok(());
    }

    let result = run_external_picker(|| run_fzf_picker(&candidates))?;
    apply_git_branch_picker_result(state, result, out)
}

pub fn run_env_var_picker(
    state: &mut AppState,
    backend: &mut PtyBackend,
    out: &mut impl Write,
) -> Result<()> {
    let candidates = env_var_picker_candidates_from_backend(backend);
    if candidates.is_empty() {
        invalidate_render_anchor(state);
        writeln!(out, "environment variable picker has no candidates")?;
        return Ok(());
    }

    let result = run_external_picker(|| run_fzf_picker(&candidates))?;
    apply_env_var_picker_result(state, result, out)
}

fn env_var_picker_candidates_from_backend(backend: &mut PtyBackend) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(result) = backend.run_command(" env", Duration::from_secs(2)) {
        names.extend(
            result
                .output
                .lines()
                .filter_map(|line| line.split_once('=').map(|(name, _)| name.to_string())),
        );
    }
    names.extend(env_var_picker_candidates());
    crate::picker::env_var_picker_candidates_from_names(names)
}

fn apply_env_var_picker_result(
    state: &mut AppState,
    result: PickerRunResult,
    out: &mut impl Write,
) -> Result<()> {
    let Some(selected) = result.selected else {
        invalidate_render_anchor(state);
        writeln!(out)?;
        writeln!(out, "environment variable picker cancelled")?;
        return Ok(());
    };
    let Some(reference) = shell_env_var_reference(&selected) else {
        invalidate_render_anchor(state);
        writeln!(
            out,
            "environment variable picker rejected invalid name: {selected}"
        )?;
        return Ok(());
    };

    if !state.apply_raw_picker_selection(&reference, PickerAction::ReplaceCurrentToken) {
        invalidate_render_anchor(state);
        writeln!(out, "environment variable picker could not update draft")?;
    }
    Ok(())
}

fn apply_git_branch_picker_result(
    state: &mut AppState,
    result: PickerRunResult,
    out: &mut impl Write,
) -> Result<()> {
    let Some(selected) = result.selected else {
        invalidate_render_anchor(state);
        writeln!(out)?;
        writeln!(out, "git branch picker cancelled")?;
        return Ok(());
    };

    if !state.apply_picker_selection(&selected, PickerAction::ReplaceCurrentToken) {
        invalidate_render_anchor(state);
        writeln!(out, "git branch picker could not update draft")?;
    }
    Ok(())
}

fn apply_template_picker_result(
    state: &mut AppState,
    result: PickerRunResult,
    out: &mut impl Write,
) -> Result<()> {
    let Some(selected) = result.selected else {
        invalidate_render_anchor(state);
        writeln!(out)?;
        writeln!(out, "template picker cancelled")?;
        return Ok(());
    };

    let id = selected
        .split_whitespace()
        .next()
        .unwrap_or(selected.as_str());
    if state.replace_draft_from_template_picker(&selected)? {
        invalidate_render_anchor(state);
        writeln!(out, "template copied to draft: {id}")?;
    } else {
        invalidate_render_anchor(state);
        writeln!(out, "template not found: {id}")?;
    }
    Ok(())
}

fn apply_history_picker_result(
    state: &mut AppState,
    result: PickerRunResult,
    out: &mut impl Write,
) -> Result<()> {
    let Some(selected) = result.selected else {
        invalidate_render_anchor(state);
        writeln!(out)?;
        writeln!(out, "history search cancelled")?;
        return Ok(());
    };

    state.replace_draft_from_history_picker(selected);
    Ok(())
}

fn apply_file_picker_result(
    state: &mut AppState,
    result: PickerRunResult,
    out: &mut impl Write,
) -> Result<()> {
    let Some(selected) = result.selected else {
        invalidate_render_anchor(state);
        writeln!(out)?;
        writeln!(out, "file picker cancelled")?;
        return Ok(());
    };

    if !state.apply_picker_selection(&selected, PickerAction::ReplaceCurrentToken) {
        invalidate_render_anchor(state);
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
    let text = normalize_paste_newlines(text);
    if !text.contains('\n') {
        state.copy_read_only_selection_to_draft();
        if state.draft.is_empty() {
            state.draft_from_editor = false;
            state.draft_from_template = false;
        }
        state.draft.insert_str(&text);
        return PasteAction::Continue;
    }

    match state.paste_config.multiline.as_str() {
        "editor" | "execute" if state.paste_config.confirm_execute => {
            state.replace_draft_from_editor_text(text);
            PasteAction::Continue
        }
        "execute" => {
            state.replace_draft_from_editor_text(text);
            PasteAction::Submit
        }
        _ => PasteAction::Continue,
    }
}

fn normalize_paste_newlines(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim_end_matches('\n')
        .to_string()
}

pub fn apply_key_to_state(key: KeyEvent, state: &mut AppState) -> KeyAction {
    if matches!(
        state.mode,
        crate::modes::Mode::Passthrough | crate::modes::Mode::UnlockPassthrough
    ) {
        return passthrough_key_bytes(key)
            .map(KeyAction::ForwardToBackend)
            .unwrap_or(KeyAction::Continue);
    }

    if !matches!(key.code, KeyCode::Tab | KeyCode::Right) {
        state.clear_completion_ui();
    }

    if state.pending_context.is_some() {
        return match (key.modifiers, key.code) {
            (_, KeyCode::Enter) => KeyAction::ConfirmContext(true),
            (_, KeyCode::Char('y' | 'Y')) => KeyAction::ConfirmContext(true),
            (_, KeyCode::Char('n' | 'N')) => KeyAction::ConfirmContext(false),
            (_, KeyCode::Esc) => KeyAction::ConfirmContext(false),
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => KeyAction::ConfirmContext(false),
            _ => KeyAction::Continue,
        };
    }

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
            (KeyModifiers::CONTROL, KeyCode::Char('t')) => KeyAction::TemplatePicker,
            (KeyModifiers::CONTROL, KeyCode::Char('b')) => KeyAction::GitBranchPicker,
            (KeyModifiers::CONTROL, KeyCode::Char('v')) => KeyAction::EnvVarPicker,
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
            if state.draft.is_empty() {
                state.selected_draft_index = None;
            }
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            state.clear_draft_for_new_draft();
            KeyAction::Continue
        }
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => KeyAction::ClearScreen,
        (KeyModifiers::CONTROL, KeyCode::Char('r')) => KeyAction::HistorySearch,
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
        (_, KeyCode::Up) if state.mode == crate::modes::Mode::Draft => KeyAction::PreviousDraft,
        (_, KeyCode::Down) if state.mode == crate::modes::Mode::Ai => {
            state.move_ai_selection_next();
            KeyAction::Continue
        }
        (_, KeyCode::Down) => {
            if state.mode == crate::modes::Mode::Draft && !is_editor_draft {
                KeyAction::NextDraft
            } else {
                KeyAction::Continue
            }
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
                    && state.completion_config.mode() != CompletionMode::Off
                    && state.completion_inline.is_some()
                {
                    return KeyAction::AcceptCompletion;
                }
                state.clear_completion_ui();
                state.draft.move_right();
            }
            KeyAction::Continue
        }
        (_, KeyCode::Backspace | KeyCode::Delete | KeyCode::Char(_)) if is_editor_draft => {
            KeyAction::Continue
        }
        (modifiers, KeyCode::Backspace) if modifiers.contains(KeyModifiers::ALT) => {
            state.copy_read_only_selection_to_draft();
            if !delete_template_placeholder_before_cursor(state) {
                expand_template_draft_if_inside_placeholder(state);
                state.draft.delete_previous_word();
            }
            clear_draft_metadata_if_empty(state);
            KeyAction::Continue
        }
        (modifiers, KeyCode::Delete | KeyCode::Char('d'))
            if modifiers.contains(KeyModifiers::ALT) =>
        {
            state.copy_read_only_selection_to_draft();
            if !delete_template_placeholder_after_cursor(state) {
                expand_template_draft_if_inside_placeholder(state);
                state.draft.delete_next_word();
            }
            clear_draft_metadata_if_empty(state);
            KeyAction::Continue
        }
        (_, KeyCode::Backspace) => {
            state.copy_read_only_selection_to_draft();
            if !delete_template_placeholder_before_cursor(state) {
                expand_template_draft_if_inside_placeholder(state);
                state.draft.backspace();
            }
            clear_draft_metadata_if_empty(state);
            KeyAction::Continue
        }
        (_, KeyCode::Delete) => {
            state.copy_read_only_selection_to_draft();
            if !delete_template_placeholder_after_cursor(state) {
                expand_template_draft_if_inside_placeholder(state);
                state.draft.delete();
            }
            clear_draft_metadata_if_empty(state);
            KeyAction::Continue
        }
        (_, KeyCode::Esc) => {
            state.clear_draft_for_new_draft();
            KeyAction::Continue
        }
        (_, KeyCode::Tab) => {
            if !state.draft.is_empty() && state.mode == crate::modes::Mode::Draft {
                if state.completion_config.mode() != CompletionMode::Off {
                    return KeyAction::CompleteOrShow;
                }
                state.clear_completion_ui();
                return KeyAction::Continue;
            }
            state.clear_completion_ui();
            state.handle_empty_tab();
            KeyAction::Continue
        }
        (_, KeyCode::Enter) => KeyAction::Submit,
        (_, KeyCode::Char(ch)) => {
            state.copy_read_only_selection_to_draft();
            if state.draft.is_empty() {
                state.draft_from_editor = false;
                state.draft_from_ai_editor = false;
                state.draft_from_template = false;
            }
            expand_template_draft_if_inside_placeholder(state);
            state.draft.insert_char(ch);
            KeyAction::Continue
        }
        _ => KeyAction::Continue,
    }
}

fn clear_draft_metadata_if_empty(state: &mut AppState) {
    if state.draft.is_empty() {
        state.selected_draft_index = None;
        state.draft_from_editor = false;
        state.draft_from_ai_editor = false;
        state.draft_from_template = false;
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

pub fn redraw(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    redraw_for_width(state, out, terminal_display_width())
}

fn redraw_for_width(state: &mut AppState, out: &mut impl Write, width: usize) -> Result<()> {
    move_to_rendered_start(state, out)?;
    execute!(
        out,
        MoveToColumn(0),
        Clear(ClearType::FromCursorDown),
        SavePosition
    )?;
    let rendered = state.rendered_text();
    write!(out, "{}", rendered.replace('\n', "\r\n"))?;
    let inline_suffix = render_inline_completion_suffix(state, width);
    if let Some(suffix) = &inline_suffix {
        write_inline_completion_suffix(out, suffix)?;
    }
    if !state.completion_panel.is_empty() {
        for line in &state.completion_panel {
            write!(out, "\r\n{line}")?;
        }
    }
    let full_render = full_rendered_text_for_width(&rendered, inline_suffix.as_deref(), state);
    let final_row = visual_line_count(&full_render, width).saturating_sub(1);
    let (cursor_row, cursor_col) = terminal_cursor_position_for_width(state, width);
    move_to_rendered_position(out, cursor_row, cursor_col)?;
    state.last_rendered_lines = final_row + 1;
    state.last_rendered_cursor_row = cursor_row;
    state.render_anchor_saved = true;
    out.flush()?;
    Ok(())
}

pub fn write_completion_candidates(state: &AppState, out: &mut impl Write) -> Result<()> {
    let candidates = state
        .cached_live_completion_candidates_with_max_results(state.completion_config.max_results)
        .map(Ok)
        .unwrap_or_else(|| state.completion_panel_candidates())?;
    if candidates.is_empty() {
        return Ok(());
    }
    let token = current_token_context(state.draft.as_str(), state.draft.cursor());
    let width = terminal_display_width();
    let content_start_col = completion_panel_content_start_col(state, width);
    for line in render_completion_candidates_for_width(
        &candidates,
        state.draft.as_str(),
        &token,
        content_start_col,
        width,
    ) {
        writeln!(out, "{line}")?;
    }
    Ok(())
}

pub fn complete_or_show_candidates(state: &mut AppState) -> Result<()> {
    complete_or_show_candidates_for_width(state, terminal_display_width())
}

pub fn complete_or_show_candidates_for_width(state: &mut AppState, width: usize) -> Result<()> {
    match state.completion_config.mode() {
        CompletionMode::Auto => complete_or_show_auto_candidates_for_width(state, width),
        CompletionMode::Tab => complete_or_show_tab_candidates_for_width(state, width),
        CompletionMode::Off => {
            state.clear_completion_ui();
            Ok(())
        }
    }
}

fn complete_or_show_auto_candidates_for_width(state: &mut AppState, width: usize) -> Result<()> {
    {
        if accept_inline_completion(state)? {
            refresh_live_completion_ui_for_width(state, width)?;
            return Ok(());
        }
        let had_panel_without_inline =
            state.completion_inline.is_none() && !state.completion_panel.is_empty();
        let candidates = state.live_completion_candidates_with_max_results(usize::MAX)?;
        if candidates.is_empty() {
            state.clear_completion_ui();
            return Ok(());
        }
        if had_panel_without_inline {
            let Some(candidate) = candidates.into_iter().next() else {
                return Ok(());
            };
            accept_completion_candidate(state, candidate)?;
            refresh_live_completion_ui_for_width(state, width)?;
            return Ok(());
        }
        set_completion_ui_from_candidates(state, candidates, width);
        return Ok(());
    }
}

fn complete_or_show_tab_candidates_for_width(state: &mut AppState, width: usize) -> Result<()> {
    if accept_visible_completion(state)? {
        refresh_live_completion_ui_for_width(state, width)?;
        return Ok(());
    }
    state.clear_completion_ui();
    let candidates = state.start_live_completion_request(usize::MAX)?;
    if !candidates.is_empty() {
        set_completion_ui_from_candidates(state, candidates, width);
    }
    Ok(())
}

fn refresh_live_completion_ui(state: &mut AppState) -> Result<()> {
    refresh_live_completion_ui_for_width(state, terminal_display_width())
}

fn refresh_live_completion_ui_for_width(state: &mut AppState, width: usize) -> Result<()> {
    state.clear_completion_ui();
    if !should_refresh_live_completion(state) {
        return Ok(());
    }
    let candidates = state.start_live_completion_request(usize::MAX)?;
    if !candidates.is_empty() {
        set_completion_ui_from_candidates(state, candidates, width);
    }
    Ok(())
}

fn should_refresh_live_completion(state: &AppState) -> bool {
    state.completion_config.enabled
        && state.completion_config.mode() == CompletionMode::Auto
        && state.pending_context.is_none()
        && !state.ctrl_x_prefix
        && state.mode == crate::modes::Mode::Draft
        && !state.draft_from_editor
        && !state.draft.is_empty()
        && state.draft.cursor() == state.draft.as_str().len()
}

fn accept_visible_completion(state: &mut AppState) -> Result<bool> {
    if state.completion_inline.is_none() && state.completion_panel.is_empty() {
        return Ok(false);
    }
    if accept_inline_completion(state)? {
        return Ok(true);
    }
    let candidates = state
        .cached_live_completion_candidates_with_max_results(usize::MAX)
        .map(Ok)
        .unwrap_or_else(|| state.completion_candidates())?;
    let Some(candidate) = candidates.into_iter().next() else {
        state.clear_completion_ui();
        return Ok(false);
    };
    accept_completion_candidate(state, candidate)
}

fn set_completion_ui_from_candidates(
    state: &mut AppState,
    candidates: Vec<CompletionCandidate>,
    width: usize,
) {
    let token = current_token_context(state.draft.as_str(), state.draft.cursor());
    let inline_index = candidates
        .iter()
        .position(|candidate| inline_completion_from_candidate(&token, candidate).is_some());
    state.completion_inline = inline_index.and_then(|index| {
        candidates
            .get(index)
            .and_then(|candidate| inline_completion_from_candidate(&token, candidate))
    });
    let panel_candidates: Vec<_> = if let Some(inline_index) = inline_index {
        candidates
            .into_iter()
            .enumerate()
            .filter_map(|(index, candidate)| (index != inline_index).then_some(candidate))
            .collect()
    } else {
        candidates
    };
    let panel_candidates = panel_candidates
        .into_iter()
        .filter(|candidate| candidate.replacement != token.text)
        .collect();
    let panel_candidates = limit_candidates(panel_candidates, state.completion_config.max_results);
    let content_start_col = completion_panel_content_start_col(state, width);
    state.completion_panel = render_completion_candidates_for_width(
        &panel_candidates,
        state.draft.as_str(),
        &token,
        content_start_col,
        width,
    );
}

fn replace_completion_ui_from_candidates(
    state: &mut AppState,
    candidates: Vec<CompletionCandidate>,
    width: usize,
) -> bool {
    let previous_inline = state.completion_inline.clone();
    let previous_panel = state.completion_panel.clone();
    state.clear_completion_ui();
    if !candidates.is_empty() {
        set_completion_ui_from_candidates(state, candidates, width);
    }
    state.completion_inline != previous_inline || state.completion_panel != previous_panel
}

fn inline_completion_from_candidate(
    token: &crate::completion::TokenContext,
    candidate: &CompletionCandidate,
) -> Option<InlineCompletion> {
    ghost_completion_suffix(token, candidate).map(|suffix| InlineCompletion {
        candidate: candidate.clone(),
        suffix,
    })
}

fn accept_inline_completion(state: &mut AppState) -> Result<bool> {
    if !state.completion_config.enabled {
        state.clear_completion_ui();
        return Ok(false);
    }
    let Some(inline) = state.completion_inline.clone() else {
        return Ok(false);
    };
    state.clear_completion_ui();
    accept_completion_candidate(state, inline.candidate)
}

fn render_inline_completion_suffix(state: &AppState, width: usize) -> Option<String> {
    if state.mode != crate::modes::Mode::Draft
        || state.draft_from_editor
        || state.draft.cursor() != state.draft.as_str().len()
    {
        return None;
    }
    let suffix = &state.completion_inline.as_ref()?.suffix;
    let (_, cursor_col) = terminal_cursor_position_for_width(state, width);
    let remaining = width.saturating_sub(cursor_col as usize);
    let suffix = truncate_with_ellipsis(suffix, remaining);
    (!suffix.is_empty()).then_some(suffix)
}

fn write_inline_completion_suffix(out: &mut impl Write, suffix: &str) -> Result<()> {
    if std::env::var_os("NO_COLOR").is_some() {
        write!(out, "{suffix}")?;
    } else {
        write!(out, "\x1b[2m{suffix}\x1b[0m")?;
    }
    Ok(())
}

fn terminal_display_width() -> usize {
    match size() {
        Ok((columns, _)) if columns > 0 => columns as usize,
        _ => 80,
    }
}

fn completion_panel_content_start_col(state: &AppState, width: usize) -> usize {
    let prefix = if state.draft.as_str()[..state.draft.cursor()].contains('\n') {
        state
            .continuation_prompt
            .as_deref()
            .unwrap_or(".. ")
            .to_string()
    } else {
        state.prompt_prefix()
    };
    visual_position(&prefix, width).1 as usize
}

fn move_to_rendered_start(state: &AppState, out: &mut impl Write) -> Result<()> {
    if state.render_anchor_saved {
        execute!(out, RestorePosition)?;
    } else if state.last_rendered_cursor_row > 0 {
        execute!(
            out,
            MoveToPreviousLine(state.last_rendered_cursor_row as u16)
        )?;
    }
    execute!(out, MoveToColumn(0))?;
    Ok(())
}

fn move_to_rendered_position(out: &mut impl Write, row: usize, col: u16) -> Result<()> {
    execute!(out, RestorePosition)?;
    if row > 0 {
        execute!(out, MoveDown(row.min(u16::MAX as usize) as u16))?;
    }
    execute!(out, MoveToColumn(col))?;
    Ok(())
}

fn move_to_rendered_end(state: &AppState, out: &mut impl Write, width: usize) -> Result<()> {
    move_to_rendered_start(state, out)?;
    let rendered = state.rendered_text();
    let (end_row, end_col) = visual_position(&rendered, width);
    if end_row > 0 {
        execute!(out, MoveDown(end_row as u16))?;
    }
    execute!(out, MoveToColumn(end_col))?;
    Ok(())
}

fn invalidate_render_anchor(state: &mut AppState) {
    state.last_rendered_lines = 0;
    state.last_rendered_cursor_row = 0;
    state.render_anchor_saved = false;
}

fn full_rendered_text_for_width(
    rendered: &str,
    inline_suffix: Option<&str>,
    state: &AppState,
) -> String {
    let mut full = String::from(rendered);
    if let Some(suffix) = inline_suffix {
        full.push_str(suffix);
    }
    for line in &state.completion_panel {
        full.push('\n');
        full.push_str(line);
    }
    full
}

fn terminal_cursor_position_for_width(state: &AppState, width: usize) -> (usize, u16) {
    let rendered_before_cursor = rendered_text_before_cursor(state);
    let (row, col) = visual_position(&rendered_before_cursor, width);
    (row, col)
}

fn rendered_text_before_cursor(state: &AppState) -> String {
    if let Some(pending) = &state.pending_context {
        let marker = if pending.dangerous {
            "[dangerous context confirmation: Y/n]"
        } else {
            "[context confirmation: Y/n]"
        };
        return format!("{}{}", state.prompt_prefix(), marker);
    }
    match state.mode {
        crate::modes::Mode::History => format!(
            "{}{}",
            state.prompt_prefix(),
            state.selected_history_command().unwrap_or("")
        ),
        crate::modes::Mode::Ai => format!(
            "{}{}",
            state.prompt_prefix(),
            state.selected_ai_command().unwrap_or("")
        ),
        crate::modes::Mode::Draft if state.draft_from_editor => {
            format!(
                "{}{}",
                state.prompt_prefix(),
                state.editor_draft_summary_for_terminal()
            )
        }
        _ => {
            let before_cursor = &state.draft.as_str()[..state.draft.cursor()];
            if before_cursor.contains('\n') {
                render_multiline_for_terminal(
                    &state.prompt_prefix(),
                    state.continuation_prompt.as_deref().unwrap_or(".. "),
                    before_cursor,
                )
            } else {
                format!("{}{}", state.prompt_prefix(), before_cursor)
            }
        }
    }
}

fn render_multiline_for_terminal(
    prompt_prefix: &str,
    continuation_prefix: &str,
    text: &str,
) -> String {
    let mut lines = text.split('\n');
    let mut rendered = String::from(prompt_prefix);
    rendered.push_str(lines.next().unwrap_or_default());
    for line in lines {
        rendered.push('\n');
        rendered.push_str(continuation_prefix);
        rendered.push_str(line);
    }
    rendered
}

pub fn accept_first_completion(state: &mut AppState) -> Result<bool> {
    if accept_inline_completion(state)? {
        return Ok(true);
    }
    let candidates = if state.completion_config.inline {
        state.live_completion_candidates_with_max_results(usize::MAX)?
    } else {
        state.completion_candidates()?
    };
    let Some(candidate) = candidates.into_iter().next() else {
        return Ok(false);
    };
    accept_completion_candidate(state, candidate)
}

fn accept_completion_candidate(
    state: &mut AppState,
    candidate: crate::completion::CompletionCandidate,
) -> Result<bool> {
    let token = current_token_context(state.draft.as_str(), state.draft.cursor());
    let accepted = accept_completion_with_mode(
        state.draft.as_str(),
        &token,
        &candidate,
        state.completion_config.tab_accept,
    );
    if state.draft.replace(accepted.line, accepted.cursor) {
        state.draft_from_template = matches!(
            candidate.source,
            crate::completion::CompletionSource::Template
                | crate::completion::CompletionSource::TemplateTypo
                | crate::completion::CompletionSource::TemplatePlaceholder
        );
        state.clear_completion_ui();
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CompletionConfig, CompletionMode, CompletionTabAccept, EditorConfig};
    use crate::encrypted_writer::EncryptedWriteQueue;
    use crate::history::{DraftEntry, HistoryEntry, HistorySource};
    use crate::modes::Mode;
    use std::collections::HashMap;
    use std::path::Path;
    use std::time::Instant;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    fn alt(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::ALT)
    }

    fn alt_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
    }

    fn wait_for_inline_suffix(state: &mut AppState, suffix: &str) {
        let mut output = Vec::new();
        for _ in 0..50 {
            refresh_after_background_events(state, &mut output).unwrap();
            if state
                .completion_inline
                .as_ref()
                .is_some_and(|inline| inline.suffix == suffix)
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "missing inline suffix {suffix:?}; inline was {:?}, panel was {:?}",
            state.completion_inline, state.completion_panel
        );
    }

    #[cfg(unix)]
    fn write_copying_fake_gpg(temp: &tempfile::TempDir) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let fake_gpg = temp.path().join("copy-gpg");
        std::fs::write(
            &fake_gpg,
            "#!/bin/sh\nmode=encrypt\nout=\"\"\ninput=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --decrypt) mode=decrypt ;;\n    --output) shift; out=\"$1\" ;;\n    --recipient|--trust-model) shift ;;\n    --batch|--yes|--no-tty|--encrypt|always) ;;\n    *) input=\"$1\" ;;\n  esac\n  shift\ndone\nif [ \"$mode\" = decrypt ]; then\n  cat \"$input\"\nelse\n  cp \"$input\" \"$out\"\nfi\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake_gpg, std::fs::Permissions::from_mode(0o755)).unwrap();
        fake_gpg
    }

    #[cfg(unix)]
    fn write_blocking_fake_gpg(
        temp: &tempfile::TempDir,
        started_path: &Path,
        release_path: &Path,
    ) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let fake_gpg = temp.path().join("blocking-gpg");
        std::fs::write(
            &fake_gpg,
            format!(
                "#!/bin/sh\nmode=encrypt\nout=\"\"\ninput=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --decrypt) mode=decrypt ;;\n    --output) shift; out=\"$1\" ;;\n    --recipient|--trust-model) shift ;;\n    --batch|--yes|--no-tty|--encrypt|always) ;;\n    *) input=\"$1\" ;;\n  esac\n  shift\ndone\nif [ \"$mode\" = decrypt ]; then\n  cat \"$input\"\nelse\n  : > '{}'\n  while [ ! -f '{}' ]; do sleep 0.02; done\n  cp \"$input\" \"$out\"\nfi\n",
                started_path.display(),
                release_path.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake_gpg, std::fs::Permissions::from_mode(0o755)).unwrap();
        fake_gpg
    }

    #[test]
    fn terminal_event_source_maps_crossterm_events() {
        assert_eq!(
            terminal_event_from_crossterm(Event::Key(key(KeyCode::Char('x')))),
            TerminalEvent::Key(key(KeyCode::Char('x')))
        );
        assert_eq!(
            terminal_event_from_crossterm(Event::Paste("echo ok".to_string())),
            TerminalEvent::Paste("echo ok".to_string())
        );
        assert_eq!(
            terminal_event_from_crossterm(Event::Resize(100, 30)),
            TerminalEvent::Resize(100, 30)
        );
        assert_eq!(
            terminal_event_from_crossterm(Event::FocusGained),
            TerminalEvent::Ignore
        );
    }

    #[test]
    #[cfg(unix)]
    fn encrypted_write_completion_event_refreshes_live_completion() {
        let temp = tempfile::tempdir().unwrap();
        let fake_gpg = write_copying_fake_gpg(&temp);
        let history_path = temp.path().join("history/regular.jsonl");
        let mut cache = HashMap::new();
        cache.insert(history_path.clone(), Vec::new());
        let entry = HistoryEntry {
            t: 1,
            command: "git status".to_string(),
            exit_code: Some(0),
            source: HistorySource::User,
        };
        let mut state = AppState {
            regular_history_path: Some(history_path.clone()),
            regular_history: vec![entry.clone()],
            encryption_config: crate::config::EncryptionConfig {
                enabled: true,
                key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
                recipient: String::new(),
            },
            encrypted_writer: Some(EncryptedWriteQueue::start(
                fake_gpg.display().to_string(),
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
                cache,
            )),
            ..AppState::default()
        };
        state.draft.insert_str("git");
        state
            .encrypted_writer
            .as_ref()
            .unwrap()
            .enqueue_append_jsonl(&history_path, &entry)
            .unwrap();
        state.flush_encrypted_writes().unwrap();
        assert!(state.completion_inline.is_none());

        let mut output = Vec::new();
        for _ in 0..50 {
            refresh_after_background_events(&mut state, &mut output).unwrap();
            if state
                .completion_inline
                .as_ref()
                .is_some_and(|inline| inline.suffix == " status")
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(state.completion_inline.as_ref().unwrap().suffix, " status");
    }

    #[test]
    #[cfg(unix)]
    fn exit_persistence_boundary_flushes_pending_encrypted_draft() {
        let temp = tempfile::tempdir().unwrap();
        let started_path = temp.path().join("gpg-started");
        let release_path = temp.path().join("release-gpg");
        let fake_gpg = write_blocking_fake_gpg(&temp, &started_path, &release_path);
        let draft_path = temp.path().join("history/draft.jsonl");
        let mut cache = HashMap::new();
        cache.insert(draft_path.clone(), Vec::new());
        let mut state = AppState {
            draft_history_path: Some(draft_path.clone()),
            draft_persist: true,
            encryption_config: crate::config::EncryptionConfig {
                enabled: true,
                key_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
                recipient: String::new(),
            },
            encrypted_writer: Some(EncryptedWriteQueue::start(
                fake_gpg.display().to_string(),
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
                cache,
            )),
            ..AppState::default()
        };
        state.draft.insert_str("echo pending-draft");
        let release_for_thread = release_path.clone();
        let started_for_thread = started_path.clone();
        let releaser = std::thread::spawn(move || {
            for _ in 0..200 {
                if started_for_thread.exists() {
                    std::thread::sleep(Duration::from_millis(120));
                    std::fs::write(&release_for_thread, b"go\n").unwrap();
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("fake gpg did not start");
        });

        let started = Instant::now();
        persist_draft_and_flush_before_exit(&state).unwrap();
        let elapsed = started.elapsed();
        releaser.join().unwrap();

        assert!(
            elapsed >= Duration::from_millis(100),
            "exit returned before pending encrypted write was released"
        );
        let loaded = crate::encryption::load_encrypted_jsonl::<DraftEntry>(
            fake_gpg.display().to_string(),
            &draft_path,
        )
        .unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].text, "echo pending-draft");
    }

    fn fixed_clock() -> i64 {
        42
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
    fn alt_word_deletion_removes_words_around_cursor() {
        let mut state = AppState::default();
        state.draft.insert_str("cargo test --all");

        apply_key_to_state(ctrl('a'), &mut state);
        apply_key_to_state(alt_key(KeyCode::Delete), &mut state);
        assert_eq!(state.draft.as_str(), " test --all");
        assert_eq!(state.draft.cursor(), 0);

        apply_key_to_state(alt('d'), &mut state);
        assert_eq!(state.draft.as_str(), " --all");
        assert_eq!(state.draft.cursor(), 0);

        apply_key_to_state(ctrl('e'), &mut state);
        apply_key_to_state(alt_key(KeyCode::Backspace), &mut state);
        assert_eq!(state.draft.as_str(), " ");
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
            KeyAction::CompleteOrShow
        );

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git");
    }

    #[test]
    fn down_on_non_empty_new_draft_saves_and_opens_blank_draft() {
        let temp = tempfile::tempdir().unwrap();
        let draft_path = temp.path().join("draft.jsonl");
        let mut state = AppState {
            draft_history_path: Some(draft_path.clone()),
            clock: fixed_clock,
            ..AppState::default()
        };
        state.draft.insert_str("echo saved-draft");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        assert!(
            !handle_key(
                key(KeyCode::Down),
                &mut state,
                &mut backend,
                &mut output,
                Duration::from_secs(5),
            )
            .unwrap()
        );

        assert_eq!(state.mode, Mode::Draft);
        assert!(state.draft.is_empty());
        assert!(!state.draft_from_editor);
        assert!(!state.draft_from_template);
        assert_eq!(state.selected_draft_index, None);
        let loaded = crate::history::load_jsonl::<crate::history::DraftEntry>(&draft_path).unwrap();
        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].t, fixed_clock());
        assert_eq!(loaded.items[0].text, "echo saved-draft");
        assert_eq!(state.draft_history, loaded.items);
    }

    #[test]
    fn up_on_blank_draft_restores_newest_saved_draft() {
        let mut state = AppState {
            draft_history: vec![
                crate::history::DraftEntry {
                    t: 1,
                    text: "echo older-draft".to_string(),
                },
                crate::history::DraftEntry {
                    t: 2,
                    text: "echo newest-draft".to_string(),
                },
            ],
            ..AppState::default()
        };
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        handle_key(
            key(KeyCode::Up),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "echo newest-draft");
        assert_eq!(state.selected_draft_index, Some(1));
        assert!(!state.draft_from_editor);
        assert!(!state.draft_from_template);
    }

    #[test]
    fn up_and_down_browse_multiple_saved_drafts() {
        let mut state = AppState {
            draft_history: vec![
                crate::history::DraftEntry {
                    t: 1,
                    text: "echo first-draft".to_string(),
                },
                crate::history::DraftEntry {
                    t: 2,
                    text: "echo second-draft".to_string(),
                },
                crate::history::DraftEntry {
                    t: 3,
                    text: "echo third-draft".to_string(),
                },
            ],
            ..AppState::default()
        };
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        for (key_code, expected_draft, expected_index) in [
            (KeyCode::Up, "echo third-draft", Some(2)),
            (KeyCode::Up, "echo second-draft", Some(1)),
            (KeyCode::Up, "echo first-draft", Some(0)),
            (KeyCode::Up, "echo first-draft", Some(0)),
            (KeyCode::Down, "echo second-draft", Some(1)),
            (KeyCode::Down, "echo third-draft", Some(2)),
        ] {
            handle_key(
                key(key_code),
                &mut state,
                &mut backend,
                &mut output,
                Duration::from_secs(5),
            )
            .unwrap();
            assert_eq!(state.draft.as_str(), expected_draft);
            assert_eq!(state.selected_draft_index, expected_index);
        }

        handle_key(
            key(KeyCode::Down),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        assert!(state.draft.is_empty());
        assert_eq!(state.selected_draft_index, None);
    }

    #[test]
    fn draft_navigation_is_disabled_when_draft_persistence_is_disabled() {
        let mut state = AppState {
            draft_persist: false,
            draft_history: vec![crate::history::DraftEntry {
                t: 1,
                text: "echo saved-draft".to_string(),
            }],
            ..AppState::default()
        };
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        handle_key(
            key(KeyCode::Up),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        assert!(state.draft.is_empty());
        assert_eq!(state.selected_draft_index, None);
    }

    #[test]
    fn editing_saved_draft_then_navigating_newer_saves_as_new_draft() {
        let temp = tempfile::tempdir().unwrap();
        let draft_path = temp.path().join("draft.jsonl");
        let mut state = AppState {
            draft_history_path: Some(draft_path.clone()),
            draft_history: vec![
                crate::history::DraftEntry {
                    t: 1,
                    text: "echo older-draft".to_string(),
                },
                crate::history::DraftEntry {
                    t: 2,
                    text: "echo newer-draft".to_string(),
                },
            ],
            selected_draft_index: Some(0),
            clock: fixed_clock,
            ..AppState::default()
        };
        state.draft.insert_str("echo older-draft edited");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        handle_key(
            key(KeyCode::Down),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        assert!(state.draft.is_empty());
        assert_eq!(state.selected_draft_index, None);
        assert_eq!(state.draft_history.len(), 3);
        assert_eq!(state.draft_history[2].t, fixed_clock());
        assert_eq!(state.draft_history[2].text, "echo older-draft edited");
        let loaded = crate::history::load_jsonl::<crate::history::DraftEntry>(&draft_path).unwrap();
        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].text, "echo older-draft edited");
    }

    #[test]
    fn tab_with_inline_enabled_shows_single_completion_before_accepting() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState {
            current_cwd: Some(temp.path().to_path_buf()),
            ..AppState::default()
        };
        std::fs::write(temp.path().join("single.txt"), "").unwrap();
        state.draft.insert_str("cat si");

        complete_or_show_candidates(&mut state).unwrap();

        assert_eq!(state.draft.as_str(), "cat si");
        assert_eq!(state.completion_inline.as_ref().unwrap().suffix, "ngle.txt");
        assert!(state.completion_panel.is_empty());

        complete_or_show_candidates(&mut state).unwrap();

        assert_eq!(state.draft.as_str(), "cat single.txt");
        assert!(state.completion_inline.is_none());
        assert!(state.completion_panel.is_empty());
    }

    #[test]
    fn typed_input_shows_live_inline_completion_without_tab() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState {
            current_cwd: Some(temp.path().to_path_buf()),
            ..AppState::default()
        };
        std::fs::write(temp.path().join("single.txt"), "").unwrap();
        state.draft.insert_str("cat s");

        apply_key_to_state(key(KeyCode::Char('i')), &mut state);
        refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

        assert_eq!(state.draft.as_str(), "cat si");
        assert_eq!(state.completion_inline.as_ref().unwrap().suffix, "ngle.txt");
        assert!(state.completion_panel.is_empty());
    }

    #[test]
    fn async_history_completion_updates_live_ui_after_request() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("local-file"), "").unwrap();
        let mut state = AppState {
            current_cwd: Some(temp.path().to_path_buf()),
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "git status --short".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            ..AppState::default()
        };
        state.draft.insert_str("git ");

        refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
        assert!(state.completion_inline.is_none());
        assert!(state.completion_panel.is_empty());

        let mut output = Vec::new();
        for _ in 0..50 {
            refresh_after_background_events(&mut state, &mut output).unwrap();
            if state
                .completion_inline
                .as_ref()
                .is_some_and(|inline| inline.suffix == "status --short")
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(
            state.completion_inline.as_ref().unwrap().suffix,
            "status --short"
        );
        assert!(state.completion_panel.is_empty());
    }

    #[test]
    fn tab_mode_async_history_completion_updates_after_explicit_tab() {
        let mut completion_config = CompletionConfig::default();
        completion_config.set_mode(CompletionMode::Tab);
        let mut state = AppState {
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "git status --short".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            completion_config,
            ..AppState::default()
        };
        state.draft.insert_str("git ");

        complete_or_show_candidates(&mut state).unwrap();
        assert!(state.completion_inline.is_none());
        assert!(state.completion_panel.is_empty());

        let mut output = Vec::new();
        for _ in 0..50 {
            refresh_after_background_events(&mut state, &mut output).unwrap();
            if state
                .completion_inline
                .as_ref()
                .is_some_and(|inline| inline.suffix == "status --short")
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(
            state.completion_inline.as_ref().unwrap().suffix,
            "status --short"
        );

        complete_or_show_candidates(&mut state).unwrap();

        assert_eq!(state.draft.as_str(), "git status --short");
        assert!(state.completion_inline.is_none());
        assert!(state.completion_panel.is_empty());
    }

    #[test]
    fn stale_async_completion_events_are_ignored_after_input_changes() {
        let mut state = AppState {
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "git status --short".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            ..AppState::default()
        };
        state.draft.insert_str("git ");
        refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

        state.draft.clear();
        state.draft.insert_str("# ");
        refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

        let mut output = Vec::new();
        for _ in 0..20 {
            refresh_after_background_events(&mut state, &mut output).unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(state.completion_inline.is_none());
        assert!(state.completion_panel.is_empty());
    }

    #[test]
    fn no_match_completion_leaves_completion_ui_empty() {
        let mut state = AppState {
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "git status".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            completion_config: CompletionConfig {
                inline: false,
                ..CompletionConfig::default()
            },
            ..AppState::default()
        };
        state.draft.insert_str("zzzzzz-no-match");

        refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

        assert!(state.completion_inline.is_none());
        assert!(state.completion_panel.is_empty());

        complete_or_show_candidates(&mut state).unwrap();

        assert_eq!(state.draft.as_str(), "zzzzzz-no-match");
        assert!(state.completion_inline.is_none());
        assert!(state.completion_panel.is_empty());
    }

    #[test]
    fn live_inline_completion_shows_remaining_candidates_as_panel_hints() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState {
            current_cwd: Some(temp.path().to_path_buf()),
            ..AppState::default()
        };
        std::fs::write(temp.path().join("one.txt"), "").unwrap();
        std::fs::write(temp.path().join("only.log"), "").unwrap();
        state.draft.insert_str("cat o");

        refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

        assert_eq!(state.completion_inline.as_ref().unwrap().suffix, "ne.txt");
        assert_eq!(
            state.completion_panel,
            vec!["file cat only.log".to_string()]
        );
    }

    #[test]
    fn inline_completion_uses_first_prefix_candidate_when_earlier_candidate_is_panel_only() {
        let mut state = AppState::default();
        state.draft.insert_str("cat al");
        let candidates = vec![
            CompletionCandidate {
                display: "beta-alpha".to_string(),
                replacement: "beta-alpha".to_string(),
                is_dir: false,
                source: crate::completion::CompletionSource::History,
            },
            CompletionCandidate {
                display: "alpha.txt".to_string(),
                replacement: "alpha.txt".to_string(),
                is_dir: false,
                source: crate::completion::CompletionSource::Path,
            },
        ];

        set_completion_ui_from_candidates(&mut state, candidates, 80);

        assert_eq!(state.completion_inline.as_ref().unwrap().suffix, "pha.txt");
        assert_eq!(
            state.completion_panel,
            vec!["history cat beta-alpha".to_string()]
        );
    }

    #[test]
    fn first_tab_accepts_live_inline_completion() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState {
            current_cwd: Some(temp.path().to_path_buf()),
            ..AppState::default()
        };
        std::fs::write(temp.path().join("single.txt"), "").unwrap();
        state.draft.insert_str("cat si");
        refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

        complete_or_show_candidates(&mut state).unwrap();

        assert_eq!(state.draft.as_str(), "cat single.txt");
        assert!(state.completion_inline.is_none());
        assert!(state.completion_panel.is_empty());
    }

    #[test]
    fn live_inline_completion_respects_inline_disabled_config() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState {
            current_cwd: Some(temp.path().to_path_buf()),
            completion_config: CompletionConfig {
                inline: false,
                ..CompletionConfig::default()
            },
            ..AppState::default()
        };
        std::fs::write(temp.path().join("single.txt"), "").unwrap();
        state.draft.insert_str("cat si");

        refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

        assert!(state.completion_inline.is_none());
        assert!(state.completion_panel.is_empty());
    }

    #[test]
    fn completion_enabled_false_disables_live_and_stale_inline_acceptance() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState {
            current_cwd: Some(temp.path().to_path_buf()),
            ..AppState::default()
        };
        std::fs::write(temp.path().join("single.txt"), "").unwrap();
        state.draft.insert_str("cat si");
        refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
        assert!(state.completion_inline.is_some());

        state.completion_config.enabled = false;
        assert!(!accept_first_completion(&mut state).unwrap());
        assert_eq!(state.draft.as_str(), "cat si");
        assert!(state.completion_inline.is_none());
        assert!(state.completion_panel.is_empty());

        refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
        complete_or_show_candidates(&mut state).unwrap();
        assert_eq!(state.draft.as_str(), "cat si");
        assert!(state.completion_inline.is_none());
        assert!(state.completion_panel.is_empty());
    }

    #[test]
    fn completion_fuzzy_false_keeps_structural_history_completion() {
        let mut state = AppState {
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "git status --short".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            completion_config: CompletionConfig {
                fuzzy: false,
                ..CompletionConfig::default()
            },
            ..AppState::default()
        };
        state.draft.insert_str("git ");

        refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
        wait_for_inline_suffix(&mut state, "status --short");

        assert_eq!(
            state.completion_inline.as_ref().unwrap().suffix,
            "status --short"
        );
    }

    #[test]
    fn tab_mode_first_tab_shows_candidates_second_tab_accepts_first_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState {
            current_cwd: Some(temp.path().to_path_buf()),
            completion_config: CompletionConfig {
                inline: false,
                ..CompletionConfig::default()
            },
            ..AppState::default()
        };
        std::fs::write(temp.path().join("single.txt"), "").unwrap();
        state.draft.insert_str("cat si");

        complete_or_show_candidates(&mut state).unwrap();

        assert_eq!(state.draft.as_str(), "cat si");
        assert_eq!(state.completion_inline.as_ref().unwrap().suffix, "ngle.txt");
        assert!(state.completion_panel.is_empty());

        complete_or_show_candidates(&mut state).unwrap();

        assert_eq!(state.draft.as_str(), "cat single.txt");
        assert!(state.completion_inline.is_none());
        assert!(state.completion_panel.is_empty());
    }

    #[test]
    fn tab_shows_multiple_completion_candidates_below_prompt() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState {
            current_cwd: Some(temp.path().to_path_buf()),
            ..AppState::default()
        };
        std::fs::write(temp.path().join("one.txt"), "").unwrap();
        std::fs::write(temp.path().join("only.log"), "").unwrap();
        state.draft.insert_str("cat o");

        complete_or_show_candidates(&mut state).unwrap();

        assert_eq!(
            state.completion_panel,
            vec!["file cat only.log".to_string()]
        );
        assert_eq!(state.draft.as_str(), "cat o");
    }

    #[test]
    fn tab_display_respects_completion_max_results() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = AppState {
            current_cwd: Some(temp.path().to_path_buf()),
            completion_config: CompletionConfig {
                max_results: 1,
                ..CompletionConfig::default()
            },
            ..AppState::default()
        };
        std::fs::write(temp.path().join("one.txt"), "").unwrap();
        std::fs::write(temp.path().join("only.log"), "").unwrap();
        state.draft.insert_str("cat o");

        complete_or_show_candidates(&mut state).unwrap();

        assert_eq!(state.completion_panel.len(), 1);
        assert!(state.completion_panel[0].starts_with("file "));
    }

    #[test]
    fn redraw_renders_completion_panel_below_prompt_and_restores_cursor() {
        let mut state = AppState::default();
        state.draft.insert_str("git");
        state.completion_panel = vec!["exec\tgit".to_string(), "exec\tgit-shell".to_string()];
        let mut output = Vec::new();

        redraw(&mut state, &mut output).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("> git\r\nexec\tgit\r\nexec\tgit-shell"));
        assert!(output.contains("\u{1b}7"), "output was {output:?}");
        assert!(output.contains("\u{1b}8"), "output was {output:?}");
        assert!(output.ends_with("\u{1b}[6G"), "output was {output:?}");
    }

    #[test]
    fn redraw_positions_cursor_from_anchor_at_wrap_boundary() {
        let mut state = AppState::default();
        state.draft.insert_str("ab");
        let mut output = Vec::new();

        redraw_for_width(&mut state, &mut output, 4).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("> ab"), "output was {output:?}");
        assert!(
            output.ends_with("\u{1b}8\u{1b}[1B\u{1b}[1G"),
            "output was {output:?}"
        );
    }

    #[test]
    fn redraw_positions_cursor_from_anchor_at_cjk_wrap_boundary() {
        let mut state = AppState::default();
        state.draft.insert_str("a中b");
        let mut output = Vec::new();

        redraw_for_width(&mut state, &mut output, 6).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("> a中b"), "output was {output:?}");
        assert!(
            output.ends_with("\u{1b}8\u{1b}[1B\u{1b}[1G"),
            "output was {output:?}"
        );
    }

    #[test]
    fn redraw_renders_inline_completion_suffix_without_moving_cursor() {
        let mut state = AppState::default();
        state.draft.insert_str("cat Car");
        state.completion_inline = Some(InlineCompletion {
            candidate: crate::completion::CompletionCandidate {
                display: "Cargo.toml".to_string(),
                replacement: "Cargo.toml".to_string(),
                is_dir: false,
                source: crate::completion::CompletionSource::Path,
            },
            suffix: "go.toml".to_string(),
        });
        let mut output = Vec::new();

        redraw(&mut state, &mut output).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("> cat Car"), "output was {output:?}");
        assert!(output.contains("go.toml"), "output was {output:?}");
        assert!(output.ends_with("\u{1b}[10G"), "output was {output:?}");
    }

    #[test]
    fn inline_completion_suffix_elides_to_terminal_width() {
        let mut state = AppState::default();
        state.draft.insert_str("cat very");
        state.completion_inline = Some(InlineCompletion {
            candidate: crate::completion::CompletionCandidate {
                display: "very-long-target.txt".to_string(),
                replacement: "very-long-target.txt".to_string(),
                is_dir: false,
                source: crate::completion::CompletionSource::Path,
            },
            suffix: "-long-target.txt".to_string(),
        });

        assert_eq!(
            render_inline_completion_suffix(&state, "> cat very-l...".len()),
            Some("-l...".to_string())
        );
    }

    #[test]
    fn redraw_positions_cursor_on_multiline_draft_last_line() {
        let mut state = AppState::default();
        state.draft.insert_str("echo \"\n123");
        let mut output = Vec::new();

        redraw(&mut state, &mut output).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains("> echo \"\r\n.. 123"),
            "output was {output:?}"
        );
        assert!(output.ends_with("\u{1b}[7G"), "output was {output:?}");
    }

    #[test]
    fn editing_after_completion_panel_clears_panel() {
        let mut state = AppState {
            completion_panel: vec!["exec\tgit".to_string()],
            completion_inline: Some(InlineCompletion {
                candidate: crate::completion::CompletionCandidate {
                    display: "git status".to_string(),
                    replacement: "git status".to_string(),
                    is_dir: false,
                    source: crate::completion::CompletionSource::History,
                },
                suffix: " status".to_string(),
            }),
            ..AppState::default()
        };
        state.draft.insert_str("git");

        apply_key_to_state(key(KeyCode::Char('x')), &mut state);

        assert!(state.completion_panel.is_empty());
        assert!(state.completion_inline.is_none());
    }

    #[test]
    fn ctrl_c_clears_multiline_draft_and_returns_to_empty_prompt() {
        let mut state = AppState {
            selected_draft_index: Some(0),
            ..AppState::default()
        };
        state.draft.insert_str("echo \"\n123");

        apply_key_to_state(ctrl('c'), &mut state);

        assert!(state.draft.is_empty());
        assert_eq!(state.selected_draft_index, None);
        assert_eq!(state.mode, crate::modes::Mode::Draft);
        assert_eq!(state.render_prompt_line(), "> ");
    }

    #[test]
    fn ctrl_c_from_history_mode_returns_to_draft_prompt() {
        let mut state = AppState {
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "echo older".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            mode: crate::modes::Mode::History,
            selected_history_index: Some(0),
            ..AppState::default()
        };

        apply_key_to_state(ctrl('c'), &mut state);

        assert_eq!(state.mode, crate::modes::Mode::Draft);
        assert!(state.draft.is_empty());
        assert_eq!(state.render_prompt_line(), "> ");
    }

    #[test]
    fn write_completion_candidates_prints_labeled_rows() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates/templates.jsonl");
        crate::templates::append_template(
            &template_path,
            &crate::templates::TemplateEntry::new("git add . && git commit"),
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
        assert!(output.contains("template git add . && git commit"));
    }

    #[test]
    fn right_at_end_requests_visible_completion_accept_without_editing_immediately() {
        let mut state = AppState::default();
        state.draft.insert_str("git");
        state.completion_inline = Some(InlineCompletion {
            candidate: CompletionCandidate {
                display: "git status".to_string(),
                replacement: "git status".to_string(),
                is_dir: false,
                source: crate::completion::CompletionSource::History,
            },
            suffix: " status".to_string(),
        });

        assert_eq!(
            apply_key_to_state(key(KeyCode::Right), &mut state),
            KeyAction::AcceptCompletion
        );

        assert_eq!(state.draft.as_str(), "git");
    }

    #[test]
    fn right_at_end_without_visible_completion_keeps_cursor_behavior() {
        let mut state = AppState::default();
        state.draft.insert_str("git");

        assert_eq!(
            apply_key_to_state(key(KeyCode::Right), &mut state),
            KeyAction::Continue
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
            completion_config: CompletionConfig {
                inline: false,
                ..CompletionConfig::default()
            },
            ..AppState::default()
        };
        state.draft.insert_str("git sta");

        assert!(accept_first_completion(&mut state).unwrap());

        assert_eq!(state.draft.as_str(), "git status");
        assert_eq!(state.draft.cursor(), "git status".len());
    }

    #[test]
    fn tab_accept_word_mode_accepts_only_next_word_from_inline_suggestion() {
        let mut state = AppState {
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "kubectl apply -f deployment.yaml".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            completion_config: CompletionConfig {
                tab_accept: CompletionTabAccept::Word,
                ..CompletionConfig::default()
            },
            ..AppState::default()
        };
        state.draft.insert_str("kub");

        complete_or_show_candidates(&mut state).unwrap();
        wait_for_inline_suffix(&mut state, "ectl apply -f deployment.yaml");
        complete_or_show_candidates(&mut state).unwrap();

        assert_eq!(state.draft.as_str(), "kubectl");
        assert_eq!(state.draft.cursor(), "kubectl".len());
    }

    #[test]
    fn right_accepts_inline_completion_when_available() {
        let mut state = AppState {
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "git status".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            ..AppState::default()
        };
        state.draft.insert_str("git");
        complete_or_show_candidates(&mut state).unwrap();
        let mut output = Vec::new();
        for _ in 0..50 {
            refresh_after_background_events(&mut state, &mut output).unwrap();
            if state
                .completion_inline
                .as_ref()
                .is_some_and(|inline| inline.suffix == " status")
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(accept_first_completion(&mut state).unwrap());

        assert_eq!(state.draft.as_str(), "git status");
        assert!(state.completion_inline.is_none());
    }

    #[test]
    fn live_template_placeholder_name_completion_accepts_raw_placeholder() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates.jsonl");
        crate::templates::append_template(
            &template_path,
            &crate::templates::TemplateEntry::new("echo {something}"),
        )
        .unwrap();
        let mut state = AppState {
            template_store_path: Some(template_path),
            ..AppState::default()
        };
        state.draft.insert_str("echo something");

        refresh_live_completion_ui_for_width(&mut state, 80).unwrap();

        assert!(state.completion_inline.is_none());
        assert!(
            state
                .completion_panel
                .iter()
                .any(|row| row.contains("{something}"))
        );

        complete_or_show_candidates_for_width(&mut state, 80).unwrap();

        assert_eq!(state.draft.as_str(), "echo {something}");
        assert!(state.draft_from_template);
        assert!(state.completion_inline.is_none());
        assert!(state.completion_panel.is_empty());
    }

    #[test]
    fn right_accepts_structural_template_completion_as_protected_template_draft() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates.jsonl");
        crate::templates::append_template(
            &template_path,
            &crate::templates::TemplateEntry::new("echo {something}"),
        )
        .unwrap();
        let mut state = AppState {
            template_store_path: Some(template_path),
            ..AppState::default()
        };
        state.draft.insert_str("echo something");

        assert!(accept_first_completion(&mut state).unwrap());

        assert_eq!(state.draft.as_str(), "echo {something}");
        assert!(state.draft_from_template);
    }

    #[test]
    fn crlf_writer_normalizes_lf_without_double_converting_crlf() {
        let mut output = Vec::new();
        {
            let mut writer = CrLfWriter::new(&mut output);
            write!(writer, "one\ntwo\r\nthree\r").unwrap();
            write!(writer, "\nfour").unwrap();
        }

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "one\r\ntwo\r\nthree\r\nfour"
        );
    }

    #[test]
    fn panic_cleanup_hook_can_be_installed_without_panicking() {
        install_panic_cleanup();
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
    fn submit_moves_cursor_to_prompt_line_end_before_newline() {
        let mut state = AppState::default();
        state.draft.insert_str("echo hello");
        state.draft.move_start();
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        handle_key(
            key(KeyCode::Enter),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains("\u{1b}[13G\r\nhello"),
            "output was {output:?}"
        );
    }

    #[test]
    fn submit_redraws_without_inline_ghost_suffix() {
        let mut state = AppState {
            regular_history: vec![crate::history::HistoryEntry {
                t: 1,
                command: "echo inline-history seeded".to_string(),
                exit_code: Some(0),
                source: crate::history::HistorySource::User,
            }],
            completion_config: CompletionConfig {
                tab_accept: CompletionTabAccept::Word,
                ..CompletionConfig::default()
            },
            ..AppState::default()
        };
        state.draft.insert_str("echo in");
        refresh_live_completion_ui_for_width(&mut state, 80).unwrap();
        wait_for_inline_suffix(&mut state, "line-history seeded");
        complete_or_show_candidates(&mut state).unwrap();
        wait_for_inline_suffix(&mut state, " seeded");
        assert_eq!(state.draft.as_str(), "echo inline-history");
        assert_eq!(state.completion_inline.as_ref().unwrap().suffix, " seeded");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        handle_key(
            key(KeyCode::Enter),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains("> echo inline-history"),
            "output was {output:?}"
        );
        assert!(
            output.contains("\r\ninline-history"),
            "output was {output:?}"
        );
        assert!(
            !output.contains("echo inline-history seeded\r\ninline-history"),
            "output was {output:?}"
        );
    }

    #[test]
    fn submit_normalizes_multiline_shell_output_for_raw_terminal() {
        let mut state = AppState::default();
        state.draft.insert_str("printf 'one\\ntwo\\n'");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        handle_key(
            key(KeyCode::Enter),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("one\r\ntwo"), "output was {output:?}");
        assert!(!output.contains("one\ntwo\n"), "output was {output:?}");
        assert!(
            !output.contains("one\r\ntwo\r\n\r\n"),
            "output was {output:?}"
        );
    }

    #[test]
    fn submit_output_stays_visible_above_redrawn_prompt() {
        let mut state = AppState::default();
        state.draft.insert_str("echo hello");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        handle_key(
            key(KeyCode::Enter),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let rendered = String::from_utf8(output).unwrap();
        let screen = TestScreen::from_output(&rendered);
        let prompt_row = screen
            .rows
            .iter()
            .position(|row| row.iter().collect::<String>() == "> ")
            .expect("redrawn prompt row");
        assert!(prompt_row > 0, "screen was {:?}", screen.lines());
        assert_eq!(screen.line(prompt_row - 1), "hello");
    }

    #[test]
    fn submit_after_completion_panel_keeps_output_adjacent_to_command_line() {
        let mut state = AppState::default();
        state.draft.insert_str("echo hello");
        state.completion_panel = vec![
            "exec\techo".to_string(),
            "exec\techoctl".to_string(),
            "exec\techoed".to_string(),
        ];
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        redraw(&mut state, &mut output).unwrap();
        handle_key(
            key(KeyCode::Enter),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let rendered = String::from_utf8(output).unwrap();
        let screen = TestScreen::from_output(&rendered);
        assert_eq!(screen.line(0), "> echo hello");
        assert_eq!(screen.line(1), "hello");
        assert_eq!(screen.line(2), "> ");
    }

    #[test]
    fn pending_context_confirmation_keys_return_confirmation_actions() {
        let mut state = AppState {
            pending_context: Some(crate::app::PendingContextPrompt {
                prompt: "explain".to_string(),
                command: "printf context".to_string(),
                dangerous: false,
            }),
            ..AppState::default()
        };

        assert_eq!(
            apply_key_to_state(key(KeyCode::Char('y')), &mut state),
            KeyAction::ConfirmContext(true)
        );
        assert_eq!(
            apply_key_to_state(key(KeyCode::Enter), &mut state),
            KeyAction::ConfirmContext(true)
        );
        assert_eq!(
            apply_key_to_state(key(KeyCode::Char('n')), &mut state),
            KeyAction::ConfirmContext(false)
        );
        assert_eq!(
            apply_key_to_state(key(KeyCode::Esc), &mut state),
            KeyAction::ConfirmContext(false)
        );
        assert_eq!(
            apply_key_to_state(ctrl('c'), &mut state),
            KeyAction::ConfirmContext(false)
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
            selected_draft_index: Some(0),
            ..AppState::default()
        };
        state.draft.insert_str("partial");

        assert_eq!(
            apply_key_to_state(key(KeyCode::Esc), &mut state),
            KeyAction::Continue
        );

        assert_eq!(state.mode, Mode::Draft);
        assert!(state.draft.is_empty());
        assert_eq!(state.selected_draft_index, None);
        assert_eq!(state.selected_history_index, Some(0));
    }

    #[test]
    fn ctrl_r_returns_history_search_action_without_editing_draft() {
        let mut state = AppState::default();
        state.draft.insert_str("git status");

        assert_eq!(
            apply_key_to_state(ctrl('r'), &mut state),
            KeyAction::HistorySearch
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
    fn ctrl_x_prefix_resolves_template_picker_chord_to_launch_action() {
        let mut state = AppState::default();
        state.draft.insert_str("git status");

        apply_key_to_state(ctrl('x'), &mut state);

        assert_eq!(
            apply_key_to_state(ctrl('t'), &mut state),
            KeyAction::TemplatePicker
        );
        assert!(!state.ctrl_x_prefix);
        assert_eq!(state.draft.as_str(), "git status");
    }

    #[test]
    fn ctrl_x_prefix_resolves_git_branch_picker_chord_to_launch_action() {
        let mut state = AppState::default();
        state.draft.insert_str("git checkout main");

        apply_key_to_state(ctrl('x'), &mut state);

        assert_eq!(
            apply_key_to_state(ctrl('b'), &mut state),
            KeyAction::GitBranchPicker
        );
        assert!(!state.ctrl_x_prefix);
        assert_eq!(state.draft.as_str(), "git checkout main");
    }

    #[test]
    fn ctrl_x_prefix_resolves_env_var_picker_chord_to_launch_action() {
        let mut state = AppState::default();
        state.draft.insert_str("git status");

        apply_key_to_state(ctrl('x'), &mut state);

        assert_eq!(
            apply_key_to_state(ctrl('v'), &mut state),
            KeyAction::EnvVarPicker
        );
        assert!(!state.ctrl_x_prefix);
        assert_eq!(state.draft.as_str(), "git status");
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
            "\nfile picker cancelled\n"
        );
    }

    #[test]
    fn apply_history_picker_result_replaces_draft_without_shell_quoting() {
        let mut state = AppState::default();
        state.draft.insert_str("partial");
        let mut output = Vec::new();

        apply_history_picker_result(
            &mut state,
            PickerRunResult {
                selected: Some("git commit -m 'hello world'".to_string()),
                exit_code: Some(0),
            },
            &mut output,
        )
        .unwrap();

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "git commit -m 'hello world'");
        assert!(String::from_utf8(output).unwrap().is_empty());
    }

    #[test]
    fn apply_history_picker_result_reports_cancel_without_editing() {
        let mut state = AppState::default();
        state.draft.insert_str("partial");
        let mut output = Vec::new();

        apply_history_picker_result(
            &mut state,
            PickerRunResult {
                selected: None,
                exit_code: Some(130),
            },
            &mut output,
        )
        .unwrap();

        assert_eq!(state.draft.as_str(), "partial");
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\nhistory search cancelled\n"
        );
    }

    #[test]
    fn apply_template_picker_result_copies_template_to_protected_draft() {
        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("templates.jsonl");
        crate::templates::append_template(
            &template_path,
            &crate::templates::TemplateEntry::new("rsync {from} {to}"),
        )
        .unwrap();
        let mut state = AppState {
            template_store_path: Some(template_path),
            ..AppState::default()
        };
        let mut output = Vec::new();

        apply_template_picker_result(
            &mut state,
            PickerRunResult {
                selected: Some(crate::templates::template_id("rsync {from} {to}")),
                exit_code: Some(0),
            },
            &mut output,
        )
        .unwrap();

        assert_eq!(state.draft.as_str(), "rsync {from} {to}");
        assert!(state.draft_from_template);
        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!(
                "template copied to draft: {}\n",
                crate::templates::template_id("rsync {from} {to}")
            )
        );
    }

    #[test]
    fn apply_template_picker_result_reports_cancel_without_editing() {
        let mut state = AppState::default();
        state.draft.insert_str("partial");
        let mut output = Vec::new();

        apply_template_picker_result(
            &mut state,
            PickerRunResult {
                selected: None,
                exit_code: Some(130),
            },
            &mut output,
        )
        .unwrap();

        assert_eq!(state.draft.as_str(), "partial");
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\ntemplate picker cancelled\n"
        );
    }

    #[test]
    fn apply_git_branch_picker_result_replaces_current_token() {
        let mut state = AppState::default();
        state.draft.insert_str("git checkout old");
        state.draft.move_left();
        state.draft.move_left();
        let mut output = Vec::new();

        apply_git_branch_picker_result(
            &mut state,
            PickerRunResult {
                selected: Some("feature/new branch".to_string()),
                exit_code: Some(0),
            },
            &mut output,
        )
        .unwrap();

        assert_eq!(state.draft.as_str(), "git checkout 'feature/new branch'");
        assert!(String::from_utf8(output).unwrap().is_empty());
    }

    #[test]
    fn apply_git_branch_picker_result_reports_cancel_without_editing() {
        let mut state = AppState::default();
        state.draft.insert_str("git checkout old");
        let mut output = Vec::new();

        apply_git_branch_picker_result(
            &mut state,
            PickerRunResult {
                selected: None,
                exit_code: Some(130),
            },
            &mut output,
        )
        .unwrap();

        assert_eq!(state.draft.as_str(), "git checkout old");
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\ngit branch picker cancelled\n"
        );
    }

    #[test]
    fn apply_env_var_picker_result_replaces_current_token_with_reference() {
        let mut state = AppState::default();
        state.draft.insert_str("echo OLD");
        state.draft.move_left();
        state.draft.move_left();
        let mut output = Vec::new();

        apply_env_var_picker_result(
            &mut state,
            PickerRunResult {
                selected: Some("AISH_TEST_VAR".to_string()),
                exit_code: Some(0),
            },
            &mut output,
        )
        .unwrap();

        assert_eq!(state.draft.as_str(), "echo $AISH_TEST_VAR");
        assert!(String::from_utf8(output).unwrap().is_empty());
    }

    #[test]
    fn apply_env_var_picker_result_rejects_invalid_names_without_editing() {
        let mut state = AppState::default();
        state.draft.insert_str("echo OLD");
        let mut output = Vec::new();

        apply_env_var_picker_result(
            &mut state,
            PickerRunResult {
                selected: Some("BAD-NAME".to_string()),
                exit_code: Some(0),
            },
            &mut output,
        )
        .unwrap();

        assert_eq!(state.draft.as_str(), "echo OLD");
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "environment variable picker rejected invalid name: BAD-NAME\n"
        );
    }

    #[test]
    fn apply_env_var_picker_result_reports_cancel_without_editing() {
        let mut state = AppState::default();
        state.draft.insert_str("echo OLD");
        let mut output = Vec::new();

        apply_env_var_picker_result(
            &mut state,
            PickerRunResult {
                selected: None,
                exit_code: Some(130),
            },
            &mut output,
        )
        .unwrap();

        assert_eq!(state.draft.as_str(), "echo OLD");
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\nenvironment variable picker cancelled\n"
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
    fn clear_screen_moves_to_top_left_before_redraw() {
        let mut state = AppState {
            last_rendered_lines: 3,
            ..AppState::default()
        };
        let mut output = Vec::new();

        clear_screen_for_redraw(&mut state, &mut output).unwrap();

        let rendered = String::from_utf8(output).unwrap();
        assert!(!rendered.starts_with("\r\n"));
        assert!(!rendered.starts_with('\n'));
        assert!(rendered.contains("\x1b[2J"));
        assert!(rendered.contains("\x1b[3J"));
        assert_eq!(state.last_rendered_lines, 0);
    }

    #[test]
    fn ctrl_l_redraw_does_not_emit_leading_blank_line() {
        let mut state = AppState::default();
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        handle_key(
            ctrl('l'),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let rendered = String::from_utf8(output).unwrap();
        let screen = TestScreen::from_output(&rendered);
        assert_eq!(screen.line(0), "> ");
        assert_eq!(screen.first_non_empty_line(), Some(0));
    }

    #[test]
    fn clear_like_command_output_redraws_prompt_on_first_screen_line() {
        let mut state = AppState::default();
        state.draft.insert_str("printf '\\033[H\\033[2J'");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        handle_key(
            key(KeyCode::Enter),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        let rendered = String::from_utf8(output).unwrap();
        let screen = TestScreen::from_output(&rendered);
        assert_eq!(screen.line(0), "> ");
        assert_eq!(screen.first_non_empty_line(), Some(0));
    }

    struct TestScreen {
        rows: Vec<Vec<char>>,
        row: usize,
        col: usize,
        saved_position: Option<(usize, usize)>,
    }

    impl TestScreen {
        fn from_output(output: &str) -> Self {
            let mut screen = Self {
                rows: vec![Vec::new(); 8],
                row: 0,
                col: 0,
                saved_position: None,
            };
            let chars: Vec<char> = output.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                match chars[i] {
                    '\x1b' if chars.get(i + 1) == Some(&'[') => {
                        i = screen.apply_csi(&chars, i + 2);
                    }
                    '\x1b' if chars.get(i + 1) == Some(&'7') => {
                        screen.saved_position = Some((screen.row, screen.col));
                        i += 2;
                    }
                    '\x1b' if chars.get(i + 1) == Some(&'8') => {
                        if let Some((row, col)) = screen.saved_position {
                            screen.row = row;
                            screen.col = col;
                            screen.ensure_row();
                        }
                        i += 2;
                    }
                    '\r' => {
                        screen.col = 0;
                        i += 1;
                    }
                    '\n' => {
                        screen.row += 1;
                        screen.ensure_row();
                        i += 1;
                    }
                    ch => {
                        screen.put(ch);
                        i += 1;
                    }
                }
            }
            screen
        }

        fn apply_csi(&mut self, chars: &[char], mut i: usize) -> usize {
            let start = i;
            while i < chars.len() && !chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            if i >= chars.len() {
                return i;
            }
            let params: String = chars[start..i].iter().collect();
            match chars[i] {
                'A' => {
                    let amount = csi_amount(&params);
                    self.row = self.row.saturating_sub(amount);
                }
                'B' => {
                    self.row += csi_amount(&params);
                    self.ensure_row();
                }
                'F' => {
                    let amount = csi_amount(&params);
                    self.row = self.row.saturating_sub(amount);
                    self.col = 0;
                }
                'H' => {
                    self.row = 0;
                    self.col = 0;
                }
                'J' => {
                    self.clear_for_j(&params);
                }
                'K' => {
                    self.clear_for_k(&params);
                }
                'G' => {
                    self.col = params.parse::<usize>().unwrap_or(1).saturating_sub(1);
                }
                _ => {}
            }
            i + 1
        }

        fn put(&mut self, ch: char) {
            self.ensure_row();
            if self.rows[self.row].len() <= self.col {
                self.rows[self.row].resize(self.col + 1, ' ');
            }
            self.rows[self.row][self.col] = ch;
            self.col += 1;
        }

        fn ensure_row(&mut self) {
            if self.rows.len() <= self.row {
                self.rows.resize_with(self.row + 1, Vec::new);
            }
        }

        fn clear_for_j(&mut self, params: &str) {
            match params {
                "" | "0" => {
                    self.clear_for_k("0");
                    for row in self.row + 1..self.rows.len() {
                        self.rows[row].clear();
                    }
                }
                "1" => {
                    for row in 0..self.row {
                        self.rows[row].clear();
                    }
                    self.clear_for_k("1");
                }
                "2" | "3" => {
                    self.rows = vec![Vec::new(); 8];
                    self.row = 0;
                    self.col = 0;
                }
                _ => {}
            }
        }

        fn clear_for_k(&mut self, params: &str) {
            self.ensure_row();
            let line = &mut self.rows[self.row];
            match params {
                "" | "0" => line.truncate(self.col.min(line.len())),
                "1" => {
                    let end = self.col.saturating_add(1).min(line.len());
                    for ch in line.iter_mut().take(end) {
                        *ch = ' ';
                    }
                }
                "2" => line.clear(),
                _ => {}
            }
        }

        fn line(&self, row: usize) -> String {
            self.rows
                .get(row)
                .map(|line| line.iter().collect::<String>())
                .unwrap_or_default()
        }

        fn first_non_empty_line(&self) -> Option<usize> {
            self.rows.iter().position(|line| !line.is_empty())
        }

        fn lines(&self) -> Vec<String> {
            self.rows
                .iter()
                .map(|line| line.iter().collect::<String>())
                .collect()
        }
    }

    fn csi_amount(params: &str) -> usize {
        params.parse::<usize>().unwrap_or(1).max(1)
    }

    #[test]
    fn passthrough_mode_forwards_keys_without_interpreting_app_actions() {
        let mut state = AppState {
            mode: Mode::Passthrough,
            ..AppState::default()
        };
        state.draft.insert_str("keep");

        assert_eq!(
            apply_key_to_state(ctrl('c'), &mut state),
            KeyAction::ForwardToBackend("\x03".to_string())
        );
        assert_eq!(state.mode, Mode::Passthrough);
        assert_eq!(state.draft.as_str(), "keep");

        assert_eq!(
            apply_key_to_state(key(KeyCode::Char('x')), &mut state),
            KeyAction::ForwardToBackend("x".to_string())
        );
        assert_eq!(state.draft.as_str(), "keep");
    }

    #[test]
    fn passthrough_mode_forwards_navigation_escape_sequences() {
        let mut state = AppState {
            mode: Mode::UnlockPassthrough,
            ..AppState::default()
        };

        assert_eq!(
            apply_key_to_state(key(KeyCode::Up), &mut state),
            KeyAction::ForwardToBackend("\x1b[A".to_string())
        );
        assert_eq!(
            apply_key_to_state(key(KeyCode::Delete), &mut state),
            KeyAction::ForwardToBackend("\x1b[3~".to_string())
        );
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
    fn run_external_editor_on_ai_prompt_creates_sendable_ai_editor_draft() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("fake-editor.sh");
        let captured = temp.path().join("captured.txt");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncat \"$1\" > '{}'\nprintf 'line one\\nline two' > \"$1\"\n",
                captured.display()
            ),
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
        state.draft.insert_str("# explain this");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        run_external_editor(
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        assert_eq!(std::fs::read_to_string(captured).unwrap(), "explain this");
        assert_eq!(state.draft.as_str(), "line one\nline two");
        assert!(state.draft_from_editor);
        assert!(state.draft_from_ai_editor);
        assert_eq!(
            state.render_prompt_line(),
            "> [ai prompt: 2 lines, 17 bytes; Enter send, Ctrl-X Ctrl-E edit]"
        );
        assert_eq!(state.last_status, None);
        assert_eq!(String::from_utf8(output).unwrap(), "editor saved draft\n");
    }

    #[test]
    fn enter_on_empty_hash_space_opens_ai_prompt_editor() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("fake-editor.sh");
        std::fs::write(&script, "#!/bin/sh\nprintf 'multi\\nAI' > \"$1\"\n").unwrap();
        make_executable(&script);
        let mut state = AppState {
            editor_config: EditorConfig {
                command: vec![script.display().to_string()],
                execute_after_save: false,
            },
            editor_temp_root: Some(temp.path().join("editor")),
            ..AppState::default()
        };
        state.draft.insert_str("# ");
        let mut backend = PtyBackend::spawn("/bin/bash").unwrap();
        let mut output = Vec::new();

        handle_key(
            key(KeyCode::Enter),
            &mut state,
            &mut backend,
            &mut output,
            Duration::from_secs(5),
        )
        .unwrap();

        assert_eq!(state.mode, Mode::Draft);
        assert_eq!(state.draft.as_str(), "multi\nAI");
        assert!(state.draft_from_editor);
        assert!(state.draft_from_ai_editor);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("editor saved draft\r\n"));
        assert!(output.contains("[ai prompt: 2 lines, 8 bytes; Enter send"));
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
        assert_eq!(normalize_paste_newlines("one\r\n"), "one");
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
        assert!(state.render_prompt_line().contains("[draft: 2 lines"));
    }

    #[test]
    fn pasted_single_line_with_trailing_newline_inserts_without_review() {
        let mut state = AppState::default();

        assert_eq!(
            apply_paste_to_state("echo pasted\n", &mut state),
            PasteAction::Continue
        );

        assert_eq!(state.draft.as_str(), "echo pasted");
        assert!(!state.draft_from_editor);
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
