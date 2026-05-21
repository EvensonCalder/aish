use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::size;

use crate::app::{
    AppState, answer_context_confirmation, answer_private_output_confirmation,
    drain_background_sync_events, execute_draft, queue_due_periodic_sync_if_needed,
    run_exit_sync_if_enabled, save_draft_if_configured, wait_for_background_sync_on_exit,
};
use crate::config::CompletionMode;
use crate::display_width::{visual_line_count, visual_position};
use crate::pty::{PtyBackend, pty_size};

mod completion_ui;
mod external;
mod input;
mod io;
mod render;

pub use completion_ui::{
    accept_first_completion, complete_or_show_candidates, complete_or_show_candidates_for_width,
    write_completion_candidates,
};
pub use external::{
    run_env_var_picker, run_external_editor, run_file_picker, run_git_branch_picker,
    run_history_picker, run_template_picker,
};
pub use input::{KeyAction, PasteAction, apply_key_to_state, apply_paste_to_state};
pub use io::{CrLfWriter, TerminalEvent, TerminalGuard, install_panic_cleanup};

use completion_ui::{
    live_completion_input_key, refresh_live_completion_ui, refresh_should_defer_completion_display,
    replace_completion_ui_from_candidates,
};
use io::read_frontend_event;
pub use render::redraw;
use render::{
    clear_screen_for_redraw, completion_panel_content_start_col, invalidate_render_anchor,
    move_to_rendered_end, terminal_cursor_position_for_width, terminal_display_width,
};

#[cfg(test)]
use completion_ui::{
    refresh_live_completion_ui_for_width, render_inline_completion_suffix,
    set_completion_ui_from_candidates,
};
#[cfg(test)]
use crossterm::event::Event;
#[cfg(test)]
use render::{redraw_for_size, redraw_for_width};
#[cfg(test)]
use {
    crate::picker::PickerRunResult,
    external::{
        apply_env_var_picker_result, apply_file_picker_result, apply_git_branch_picker_result,
        apply_history_picker_result, apply_template_picker_result,
    },
};
#[cfg(test)]
use {input::normalize_paste_newlines, io::terminal_event_from_crossterm};

#[cfg(test)]
use crate::{app::InlineCompletion, completion::CompletionCandidate};

const FRONTEND_TICK_INTERVAL: Duration = Duration::from_millis(10);

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
                    persist_draft_and_flush_before_exit(state, out)?;
                    break;
                }
            }
            TerminalEvent::Paste(text) => {
                if apply_paste_to_state(&text, state) == PasteAction::Submit {
                    execute_draft(state, backend, out, command_timeout)?;
                    if state.exit_requested {
                        persist_draft_and_flush_before_exit(state, out)?;
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

fn persist_draft_and_flush_before_exit(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let _ = save_draft_if_configured(state)?;
    {
        let mut display_out = CrLfWriter::new(out);
        for message in wait_for_background_sync_on_exit(state)? {
            writeln!(display_out, "{message}")?;
        }
        run_exit_sync_if_enabled(state, &mut display_out)?;
        display_out.flush()?;
    }
    state.flush_encrypted_writes()
}

fn refresh_after_background_events(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let mut should_redraw = false;
    let mut sync_messages = Vec::new();
    {
        let mut sync_start_output = Vec::new();
        queue_due_periodic_sync_if_needed(state, &mut sync_start_output)?;
        sync_messages.extend(sync_output_lines(sync_start_output));
    }
    let sync_drain = drain_background_sync_events(state)?;
    if sync_drain.completed {
        let previous_inline = state.completion_inline.clone();
        let previous_panel = state.completion_panel.clone();
        refresh_live_completion_ui(state)?;
        should_redraw |=
            state.completion_inline != previous_inline || state.completion_panel != previous_panel;
    }
    sync_messages.extend(sync_drain.messages);
    if !sync_messages.is_empty() {
        write_background_messages(state, out, &sync_messages)?;
        should_redraw = true;
    }
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
    if state.drain_startup_unlock_event()? {
        refresh_live_completion_ui(state)?;
        should_redraw = true;
    }
    if should_redraw {
        redraw(state, out)?;
    }
    Ok(())
}

fn sync_output_lines(output: Vec<u8>) -> Vec<String> {
    String::from_utf8_lossy(&output)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn write_background_messages(
    state: &mut AppState,
    out: &mut impl Write,
    messages: &[String],
) -> Result<()> {
    move_to_rendered_end(state, out, terminal_display_width())?;
    invalidate_render_anchor(state);
    for message in messages {
        write!(out, "\r\n{message}")?;
    }
    write!(out, "\r\n")?;
    Ok(())
}

fn should_apply_completion_event_update(state: &AppState) -> bool {
    state.completion_config.enabled
        && state.completion_config.mode() != CompletionMode::Off
        && state.pending_context.is_none()
        && state.pending_private_output.is_none()
        && !state.has_pending_key_prefix()
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
        KeyAction::Exit => {
            render_ctrl_d_exit(state, out, had_completion_ui)?;
            return Ok(true);
        }
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
                execute_draft(state, backend, out, command_timeout)?;
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
        KeyAction::ConfirmPrivateOutput(accepted) => {
            move_to_rendered_end(state, out, terminal_display_width())?;
            invalidate_render_anchor(state);
            write!(out, "\r\n")?;
            let mut display_out = CrLfWriter::new(out);
            answer_private_output_confirmation(state, accepted, &mut display_out, command_timeout)?;
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

fn render_ctrl_d_exit(
    state: &mut AppState,
    out: &mut impl Write,
    had_completion_ui: bool,
) -> Result<()> {
    if had_completion_ui {
        redraw(state, out)?;
    }
    move_to_rendered_end(state, out, terminal_display_width())?;
    invalidate_render_anchor(state);
    write!(out, "\r\nexit\r\n")?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests;
