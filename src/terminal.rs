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

use crate::app::{AppState, answer_context_confirmation, execute_draft, save_draft_if_configured};
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

mod completion_ui;

pub use completion_ui::{
    accept_first_completion, complete_or_show_candidates, complete_or_show_candidates_for_width,
    write_completion_candidates,
};

use completion_ui::{
    live_completion_input_key, refresh_live_completion_ui, refresh_should_defer_completion_display,
    render_inline_completion_suffix, replace_completion_ui_from_candidates,
    write_inline_completion_suffix,
};

#[cfg(test)]
use completion_ui::{refresh_live_completion_ui_for_width, set_completion_ui_from_candidates};

#[cfg(test)]
use crate::{app::InlineCompletion, completion::CompletionCandidate};

const FRONTEND_TICK_INTERVAL: Duration = Duration::from_millis(10);

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
        let event = match read_frontend_event(Duration::from_millis(0))? {
            TerminalEvent::Tick => {
                refresh_after_background_events(state, out)?;
                read_frontend_event(FRONTEND_TICK_INTERVAL)?
            }
            event => event,
        };
        match event {
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
        should_redraw |= replace_completion_ui_from_candidates(
            state,
            candidates,
            terminal_display_width(),
            should_show_completion_panel_for_event_update(state),
        );
    }
    if state.drain_encrypted_write_events() {
        let previous_inline = state.completion_inline.clone();
        let previous_panel = state.completion_panel.clone();
        refresh_live_completion_ui(state)?;
        should_redraw |=
            state.completion_inline != previous_inline || state.completion_panel != previous_panel;
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

fn should_show_completion_panel_for_event_update(state: &AppState) -> bool {
    state.completion_config.mode() != CompletionMode::Off
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
    let previous_draft = state.draft.as_str().to_string();
    let previous_cursor = state.draft.cursor();
    let previous_mode = state.mode;
    let previous_draft_from_editor = state.draft_from_editor;
    let previous_completion_input = live_completion_input_key(state);
    let action = apply_key_to_state(key, state);
    if refresh_should_defer_completion_display(state, &previous_completion_input, &action) {
        state.defer_completion_display(std::time::Instant::now());
    }
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
            state.cancel_live_completion();
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
    let width = terminal_display_width();
    let incremental_snapshot = IncrementalRenderSnapshot {
        had_completion_ui,
        previous_draft: &previous_draft,
        previous_cursor,
        previous_mode,
        previous_draft_from_editor,
        width,
    };
    if can_render_appended_char_incrementally(key, &action, state, incremental_snapshot) {
        write_incremental_appended_char(state, out, key, width)?;
        return Ok(false);
    }
    redraw(state, out)?;
    Ok(false)
}

struct IncrementalRenderSnapshot<'a> {
    had_completion_ui: bool,
    previous_draft: &'a str,
    previous_cursor: usize,
    previous_mode: crate::modes::Mode,
    previous_draft_from_editor: bool,
    width: usize,
}

fn can_render_appended_char_incrementally(
    key: KeyEvent,
    action: &KeyAction,
    state: &AppState,
    snapshot: IncrementalRenderSnapshot<'_>,
) -> bool {
    let KeyCode::Char(ch) = key.code else {
        return false;
    };
    if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
        || !matches!(action, KeyAction::Continue)
        || snapshot.had_completion_ui
        || state.completion_inline.is_some()
        || !state.completion_panel.is_empty()
        || !state.render_anchor_saved
        || snapshot.previous_mode != crate::modes::Mode::Draft
        || state.mode != crate::modes::Mode::Draft
        || snapshot.previous_draft_from_editor
        || state.draft_from_editor
        || snapshot.previous_cursor != snapshot.previous_draft.len()
        || snapshot.previous_draft.contains('\n')
    {
        return false;
    }
    if state.draft.as_str().len() < snapshot.previous_draft.len() {
        return false;
    }
    let appended = &state.draft.as_str()[snapshot.previous_draft.len()..];
    if state.draft.cursor() != state.draft.as_str().len()
        || !state.draft.as_str().starts_with(snapshot.previous_draft)
        || appended != ch.to_string()
    {
        return false;
    }
    let previous_rendered = format!("{}{}", state.prompt_prefix(), snapshot.previous_draft);
    let (previous_row, _) = visual_position(&previous_rendered, snapshot.width);
    let (current_row, _) = terminal_cursor_position_for_width(state, snapshot.width);
    current_row == previous_row
}

fn write_incremental_appended_char(
    state: &mut AppState,
    out: &mut impl Write,
    key: KeyEvent,
    width: usize,
) -> Result<()> {
    let KeyCode::Char(ch) = key.code else {
        return Ok(());
    };
    write!(out, "{ch}")?;
    let rendered = state.rendered_text();
    state.last_rendered_lines = visual_line_count(&rendered, width);
    state.last_rendered_cursor_row = terminal_cursor_position_for_width(state, width).0;
    out.flush()?;
    Ok(())
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
            state.draft_from_ai_editor = false;
            state.draft_from_template = false;
            state.draft_has_paste_preview = false;
        }
        state.draft.insert_str(&text);
        return PasteAction::Continue;
    }

    match state.paste_config.multiline.as_str() {
        "editor" | "execute" if state.paste_config.confirm_execute => {
            state.replace_draft_from_paste_text(text);
            PasteAction::Continue
        }
        "execute" => {
            state.replace_draft_from_paste_text(text);
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
                    && (state.completion_inline.is_some()
                        || state
                            .cached_live_completion_candidates_with_max_results(1)
                            .is_some_and(|candidates| !candidates.is_empty()))
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
                state.draft_has_paste_preview = false;
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
        state.draft_has_paste_preview = false;
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
    let (width, height) = terminal_display_size();
    redraw_for_size(state, out, width, height)
}

#[cfg(test)]
fn redraw_for_width(state: &mut AppState, out: &mut impl Write, width: usize) -> Result<()> {
    redraw_for_size(state, out, width, terminal_display_height())
}

fn redraw_for_size(
    state: &mut AppState,
    out: &mut impl Write,
    width: usize,
    height: usize,
) -> Result<()> {
    let rendered = state.rendered_text();
    let inline_suffix = render_inline_completion_suffix(state, width);
    let rendered_with_inline =
        full_rendered_text_for_width(&rendered, inline_suffix.as_deref(), &[]);
    let prompt_lines = visual_line_count(&rendered_with_inline, width);
    let visible_panel_len = state
        .completion_panel
        .len()
        .min(height.saturating_sub(prompt_lines));
    let visible_panel = &state.completion_panel[..visible_panel_len];
    let full_render =
        full_rendered_text_for_width(&rendered, inline_suffix.as_deref(), visible_panel);
    let render_lines = visual_line_count(&full_render, width).max(1);

    move_to_rendered_start(state, out)?;
    reserve_render_area(out, render_lines, height)?;
    execute!(
        out,
        MoveToColumn(0),
        Clear(ClearType::FromCursorDown),
        SavePosition
    )?;
    write!(out, "{}", rendered.replace('\n', "\r\n"))?;
    if let Some(suffix) = &inline_suffix {
        write_inline_completion_suffix(out, suffix)?;
    }
    if !visible_panel.is_empty() {
        for line in visible_panel {
            write!(out, "\r\n{line}")?;
        }
    }
    let final_row = visual_line_count(&full_render, width).saturating_sub(1);
    let (cursor_row, cursor_col) = terminal_cursor_position_for_width(state, width);
    move_to_rendered_position(out, cursor_row, cursor_col)?;
    state.last_rendered_lines = final_row + 1;
    state.last_rendered_cursor_row = cursor_row;
    state.render_anchor_saved = true;
    out.flush()?;
    Ok(())
}

fn terminal_display_width() -> usize {
    terminal_display_size().0
}

#[cfg(test)]
fn terminal_display_height() -> usize {
    terminal_display_size().1
}

fn terminal_display_size() -> (usize, usize) {
    match size() {
        Ok((columns, rows)) => (
            if columns > 0 { columns as usize } else { 80 },
            if rows > 0 { rows as usize } else { 24 },
        ),
        _ => (80, 24),
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
    if state.last_rendered_cursor_row > 0 {
        execute!(
            out,
            MoveToPreviousLine(state.last_rendered_cursor_row as u16)
        )?;
    }
    execute!(out, MoveToColumn(0))?;
    Ok(())
}

fn reserve_render_area(out: &mut impl Write, render_lines: usize, height: usize) -> Result<()> {
    let reserve_rows = render_lines
        .saturating_sub(1)
        .min(height.saturating_sub(1))
        .min(u16::MAX as usize);
    if reserve_rows == 0 {
        return Ok(());
    }
    for _ in 0..reserve_rows {
        write!(out, "\r\n")?;
    }
    execute!(
        out,
        MoveToPreviousLine(reserve_rows as u16),
        MoveToColumn(0)
    )?;
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
    panel: &[String],
) -> String {
    let mut full = String::from(rendered);
    if let Some(suffix) = inline_suffix {
        full.push_str(suffix);
    }
    for line in panel {
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

#[cfg(test)]
mod tests;
