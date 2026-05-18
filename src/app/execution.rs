use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event};
use crossterm::terminal::is_raw_mode_enabled;

use crate::commands::{ParsedLine, parse_line};
use crate::history::{HistoryEntry, HistorySource, NoteEntry};
use crate::modes::Mode;
use crate::pty::{BackendShellClosed, PtyBackend, PtyCommandEvent};
use crate::shell_integration::passthrough_key_bytes;
use crate::templates::template_placeholders;

use super::context_prompt::{submit_ai_prompt, submit_ai_prompt_with_context};
use super::{AppState, OutputEntry, private_commands};

pub fn execute_draft(
    state: &mut AppState,
    backend: &mut PtyBackend,
    out: &mut impl Write,
    timeout: Duration,
) -> Result<()> {
    state.cancel_live_completion();
    if state.pending_context.is_some() {
        writeln!(out, "context confirmation is pending; answer Y or n")?;
        state.mode = Mode::Draft;
        return Ok(());
    }
    if state.pending_private_output.is_some() {
        writeln!(
            out,
            "private output export confirmation is pending; answer Y or n"
        )?;
        state.mode = Mode::Draft;
        return Ok(());
    }

    if state.draft.is_empty() && state.mode == Mode::History {
        state.copy_selected_history_to_draft();
    }
    let executing_ai = state.draft.is_empty() && state.mode == Mode::Ai;
    if executing_ai {
        state.copy_selected_ai_to_draft();
    }

    if state.draft.is_empty() {
        return Ok(());
    }

    let command = state.draft.as_str().to_string();
    if state.draft_from_ai_editor {
        let prompt = command.trim();
        if prompt.is_empty() {
            state.clear_draft_for_new_draft();
            return Ok(());
        }
        let ai_line = format!("# {prompt}");
        match parse_line(&ai_line) {
            ParsedLine::AiPrompt(prompt) => submit_ai_prompt(state, prompt, out)?,
            ParsedLine::AiPromptWithContext { prompt, command } => {
                submit_ai_prompt_with_context(state, prompt, command, out, timeout)?;
            }
            _ => unreachable!("AI editor drafts are submitted as AI prompts"),
        }
        state.clear_draft_preserving_mode();
        return Ok(());
    }
    if state.draft_from_template {
        let unresolved = template_placeholders(&command);
        if !unresolved.is_empty() {
            writeln!(
                out,
                "cannot execute unresolved template placeholders: {}",
                unresolved.join(", ")
            )?;
            state.mode = Mode::Draft;
            return Ok(());
        }
    }
    if !state.draft_from_editor {
        match parse_line(&command) {
            ParsedLine::Ordinary(_) => {}
            ParsedLine::EmptyPrivate => {
                writeln!(out, "empty Aish command")?;
                state.clear_draft_for_new_draft();
                return Ok(());
            }
            ParsedLine::Note { tag, text } => {
                state.append_note(NoteEntry {
                    tag,
                    text: text.to_string(),
                })?;
                writeln!(out, "note stored")?;
                state.clear_draft_for_new_draft();
                return Ok(());
            }
            ParsedLine::Private { name, args } => {
                if let Err(err) = private_commands::execute_private_command(state, out, name, args)
                {
                    writeln!(out, "Error: {err}")?;
                    let _ =
                        state.append_event(crate::log::EventLevel::Error, "private command failed");
                    state.clear_draft_for_new_draft();
                }
                return Ok(());
            }
            ParsedLine::AiPrompt(prompt) => {
                submit_ai_prompt(state, prompt, out)?;
                state.clear_draft_preserving_mode();
                return Ok(());
            }
            ParsedLine::AiPromptWithContext { prompt, command } => {
                submit_ai_prompt_with_context(state, prompt, command, out, timeout)?;
                state.clear_draft_preserving_mode();
                return Ok(());
            }
        }
    }

    if !state.draft_from_editor {
        let continuation = backend.input_needs_more_lines(&command)?;
        if continuation.needs_more {
            state.continuation_prompt = continuation.prompt;
            state.draft.insert_str("\n");
            state.mode = Mode::Draft;
            return Ok(());
        }
        state.save_current_draft_if_needed()?;
    }

    state.mode = Mode::CommandRunning;
    let result = match backend
        .run_command_passthrough_with_event_callback(&command, |backend, event| {
            handle_command_running_event(backend, out, event)
        }) {
        Ok(result) => result,
        Err(error) if error.downcast_ref::<BackendShellClosed>().is_some() => {
            state.exit_requested = true;
            state.clear_draft_for_new_draft();
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    record_completed_command(
        state,
        result.command.clone(),
        result.output.clone(),
        result.exit_code,
        executing_ai,
    )?;
    if let Some(cwd) = result.cwd {
        state.current_cwd = Some(PathBuf::from(cwd));
    }
    Ok(())
}

pub(crate) fn record_completed_command(
    state: &mut AppState,
    command: String,
    output: String,
    exit_code: i32,
    executing_ai: bool,
) -> Result<()> {
    state.push_output_entry(OutputEntry {
        command: command.clone(),
        output: output.clone(),
        exit_code,
    });
    if state.regular_history_path.is_some() {
        let entry = HistoryEntry {
            command,
            t: (state.clock)(),
            exit_code: Some(exit_code),
            source: if executing_ai {
                HistorySource::Ai
            } else {
                HistorySource::User
            },
        };
        state.append_regular_history_entry(&entry)?;
        state.regular_history.push(entry);
    }
    state.last_status = Some(exit_code);
    state.continuation_prompt = None;
    if executing_ai {
        state.draft.clear();
        state.selected_draft_index = None;
        state.draft_from_editor = false;
        state.draft_from_ai_editor = false;
        state.draft_from_template = false;
        state.draft_has_paste_preview = false;
    } else {
        state.clear_draft_for_new_draft();
    }
    if executing_ai && exit_code == 0 {
        state.advance_after_ai_success();
    } else if executing_ai {
        state.mode = Mode::Ai;
    }
    Ok(())
}

fn forward_terminal_input_to_backend(backend: &mut PtyBackend) -> Result<bool> {
    if !is_raw_mode_enabled().unwrap_or(false) {
        return Ok(false);
    }

    let mut marker_may_need_reissue = false;
    while event::poll(Duration::from_millis(0))? {
        match event::read()? {
            Event::Key(key) => {
                if matches!(
                    (key.modifiers, key.code),
                    (
                        crossterm::event::KeyModifiers::CONTROL,
                        crossterm::event::KeyCode::Char('c' | 'd')
                    )
                ) {
                    marker_may_need_reissue = true;
                }
                if let Some(bytes) = passthrough_key_bytes(key) {
                    backend.write_raw(&bytes)?;
                }
            }
            Event::Paste(text) => {
                backend.write_raw(&text)?;
            }
            Event::Resize(cols, rows) => {
                backend.resize(crate::pty::pty_size(cols, rows))?;
            }
            _ => {}
        }
    }
    Ok(marker_may_need_reissue)
}

fn handle_command_running_event(
    backend: &mut PtyBackend,
    out: &mut impl Write,
    event: PtyCommandEvent<'_>,
) -> Result<bool> {
    match event {
        PtyCommandEvent::Output(chunk) => {
            write_command_output_bytes(out, chunk)?;
            out.flush()?;
            Ok(false)
        }
        PtyCommandEvent::PollInput | PtyCommandEvent::Idle => {
            forward_terminal_input_to_backend(backend)
        }
    }
}

pub(crate) fn foreground_shell_args(shell: &str, command: &str) -> Vec<String> {
    let shell_name = shell_name(shell);
    match shell_name.as_str() {
        "bash" | "zsh" => vec!["-lc".to_string(), command.to_string()],
        "fish" => vec!["-c".to_string(), command.to_string()],
        _ => vec!["-c".to_string(), command.to_string()],
    }
}

fn shell_name(shell: &str) -> String {
    let name = Path::new(shell.trim())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .trim_start_matches('-')
        .to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

#[cfg(test)]
pub(crate) fn write_command_output(out: &mut impl Write, output: &str) -> Result<()> {
    write_command_output_bytes(out, output.as_bytes())
}

fn write_command_output_bytes(out: &mut impl Write, output: &[u8]) -> Result<()> {
    // PTY output is already terminal protocol. Adding display framing here can
    // corrupt commands like `clear`: ESC[H ESC[2J followed by an Aish-added LF
    // moves the prompt to row 1, leaving a blank first row.
    out.write_all(output)?;
    if output_clears_visible_screen_bytes(output) {
        write!(out, "\x1b[H")?;
    }
    Ok(())
}

fn output_clears_visible_screen_bytes(output: &[u8]) -> bool {
    output_clears_visible_screen(&String::from_utf8_lossy(output))
}

fn output_clears_visible_screen(output: &str) -> bool {
    output.contains("\x1b[2J")
        || output.contains("\x1bc")
        || (output_contains_cursor_home(output) && output.contains("\x1b[J"))
}

fn output_contains_cursor_home(output: &str) -> bool {
    output.contains("\x1b[H") || output.contains("\x1b[;H") || output.contains("\x1b[1;1H")
}
