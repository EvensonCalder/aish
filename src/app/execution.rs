use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
#[cfg(not(unix))]
use crossterm::event::{self, Event};
use crossterm::terminal::{is_raw_mode_enabled, size};

use crate::commands::{ParsedLine, parse_line};
use crate::history::{HistoryEntry, HistorySource, NoteEntry};
use crate::modes::Mode;
use crate::pty::{BackendShellClosed, PtyBackend, PtyCommandEvent, pty_size};
#[cfg(not(unix))]
use crate::shell_integration::passthrough_key_bytes;
use crate::templates::template_placeholders;

use super::context_prompt::{submit_ai_prompt, submit_ai_prompt_with_context};
use super::{AppState, OutputEntry, private_commands};

struct DisplayWriter<'a, W: Write> {
    inner: &'a mut W,
    convert_lf: bool,
    previous_was_cr: bool,
}

impl<'a, W: Write> DisplayWriter<'a, W> {
    fn new(inner: &'a mut W) -> Self {
        Self {
            inner,
            convert_lf: is_raw_mode_enabled().unwrap_or(false),
            previous_was_cr: false,
        }
    }
}

impl<W: Write> Write for DisplayWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for &byte in buf {
            if self.convert_lf && byte == b'\n' && !self.previous_was_cr {
                self.inner.write_all(b"\r")?;
            }
            self.inner.write_all(&[byte])?;
            self.previous_was_cr = byte == b'\r';
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub fn execute_draft(
    state: &mut AppState,
    backend: &mut PtyBackend,
    out: &mut impl Write,
    timeout: Duration,
) -> Result<()> {
    state.cancel_live_completion();
    if state.pending_context.is_some() {
        let mut display_out = DisplayWriter::new(out);
        writeln!(
            display_out,
            "context confirmation is pending; answer Y or n"
        )?;
        state.mode = Mode::Draft;
        return Ok(());
    }
    if state.pending_private_output.is_some() {
        let mut display_out = DisplayWriter::new(out);
        writeln!(
            display_out,
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
        let mut display_out = DisplayWriter::new(out);
        match parse_line(&ai_line) {
            ParsedLine::AiPrompt(prompt) => submit_ai_prompt(state, prompt, &mut display_out)?,
            ParsedLine::AiPromptWithContext { prompt, command } => {
                submit_ai_prompt_with_context(state, prompt, command, &mut display_out, timeout)?;
            }
            _ => unreachable!("AI editor drafts are submitted as AI prompts"),
        }
        state.clear_draft_preserving_mode();
        return Ok(());
    }
    if state.draft_from_template {
        let unresolved = template_placeholders(&command);
        if !unresolved.is_empty() {
            let mut display_out = DisplayWriter::new(out);
            writeln!(
                display_out,
                "cannot execute unresolved template placeholders: {}",
                unresolved.join(", ")
            )?;
            state.mode = Mode::Draft;
            return Ok(());
        }
    }
    match parse_line(&command) {
        ParsedLine::Ordinary(_) => {}
        ParsedLine::EmptyPrivate => {
            let mut display_out = DisplayWriter::new(out);
            writeln!(display_out, "empty Aish command")?;
            state.clear_draft_for_new_draft();
            return Ok(());
        }
        ParsedLine::Note { tag, text } => {
            state.append_note(NoteEntry {
                tag,
                text: text.to_string(),
            })?;
            let mut display_out = DisplayWriter::new(out);
            writeln!(display_out, "note stored")?;
            state.clear_draft_for_new_draft();
            return Ok(());
        }
        ParsedLine::Private { name, args } => {
            let mut display_out = DisplayWriter::new(out);
            if let Err(err) =
                private_commands::execute_private_command(state, &mut display_out, name, args)
            {
                writeln!(display_out, "Error: {err}")?;
                let _ = state.append_event(crate::log::EventLevel::Error, "private command failed");
                state.clear_draft_for_new_draft();
            }
            return Ok(());
        }
        ParsedLine::AiPrompt(prompt) => {
            let mut display_out = DisplayWriter::new(out);
            submit_ai_prompt(state, prompt, &mut display_out)?;
            state.clear_draft_preserving_mode();
            return Ok(());
        }
        ParsedLine::AiPromptWithContext { prompt, command } => {
            let mut display_out = DisplayWriter::new(out);
            submit_ai_prompt_with_context(state, prompt, command, &mut display_out, timeout)?;
            state.clear_draft_preserving_mode();
            return Ok(());
        }
    }
    if is_plain_exit_command(&command) {
        state.exit_requested = true;
        state.clear_draft_for_new_draft();
        return Ok(());
    }

    let continuation = backend.input_needs_more_lines(&command)?;
    if continuation.needs_more {
        state.continuation_prompt = continuation.prompt;
        state.draft.insert_str("\n");
        state.mode = Mode::Draft;
        return Ok(());
    }
    state.save_current_draft_if_needed()?;

    state.mode = Mode::CommandRunning;
    let mut bridge = ForegroundPtyBridge::enter(backend)?;
    let result = match backend
        .run_command_passthrough_with_event_callback(&command, |backend, event| {
            handle_command_running_event(&mut bridge, backend, out, event)
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

fn is_plain_exit_command(command: &str) -> bool {
    command.trim() == "exit"
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

struct ForegroundPtyBridge {
    last_size: Option<(u16, u16)>,
    #[cfg(unix)]
    stdin: Option<UnixStdinBridge>,
}

impl ForegroundPtyBridge {
    fn enter(backend: &mut PtyBackend) -> Result<Self> {
        let mut bridge = Self {
            last_size: None,
            #[cfg(unix)]
            stdin: UnixStdinBridge::enter()?,
        };
        bridge.sync_size(backend)?;
        Ok(bridge)
    }

    fn pump(&mut self, backend: &mut PtyBackend) -> Result<bool> {
        self.forward_stdin(backend)?;
        self.sync_size(backend)?;
        Ok(false)
    }

    fn sync_size(&mut self, backend: &mut PtyBackend) -> Result<()> {
        let current = size()?;
        if self.last_size != Some(current) {
            backend.resize(pty_size(current.0, current.1))?;
            self.last_size = Some(current);
        }
        Ok(())
    }

    #[cfg(unix)]
    fn forward_stdin(&mut self, backend: &mut PtyBackend) -> Result<()> {
        if let Some(stdin) = &mut self.stdin {
            stdin.forward_available(backend)?;
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn forward_stdin(&mut self, backend: &mut PtyBackend) -> Result<()> {
        if !is_raw_mode_enabled().unwrap_or(false) {
            return Ok(());
        }

        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(key) => {
                    if let Some(bytes) = passthrough_key_bytes(key) {
                        backend.write_raw(&bytes)?;
                    }
                }
                Event::Paste(text) => {
                    backend.write_raw(&text)?;
                }
                Event::Resize(cols, rows) => {
                    backend.resize(pty_size(cols, rows))?;
                    self.last_size = Some((cols, rows));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
struct UnixStdinBridge {
    fd: libc::c_int,
    original_flags: libc::c_int,
}

#[cfg(unix)]
impl UnixStdinBridge {
    fn enter() -> Result<Option<Self>> {
        if !is_raw_mode_enabled().unwrap_or(false) {
            return Ok(None);
        }
        let fd = libc::STDIN_FILENO;
        let original_flags = fcntl_getfl(fd)?;
        fcntl_setfl(fd, original_flags | libc::O_NONBLOCK)?;
        Ok(Some(Self { fd, original_flags }))
    }

    fn forward_available(&mut self, backend: &mut PtyBackend) -> Result<()> {
        let mut buf = [0_u8; 4096];
        loop {
            let read =
                unsafe { libc::read(self.fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len()) };
            if read > 0 {
                backend.write_raw_bytes(&buf[..read as usize])?;
                continue;
            }
            if read == 0 {
                return Ok(());
            }
            let err = std::io::Error::last_os_error();
            match err.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted => return Ok(()),
                _ => return Err(err.into()),
            }
        }
    }
}

#[cfg(unix)]
impl Drop for UnixStdinBridge {
    fn drop(&mut self) {
        let _ = fcntl_setfl(self.fd, self.original_flags);
    }
}

#[cfg(unix)]
fn fcntl_getfl(fd: libc::c_int) -> Result<libc::c_int> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(flags)
}

#[cfg(unix)]
fn fcntl_setfl(fd: libc::c_int, flags: libc::c_int) -> Result<()> {
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn handle_command_running_event(
    bridge: &mut ForegroundPtyBridge,
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
        PtyCommandEvent::PollInput | PtyCommandEvent::Idle => bridge.pump(backend),
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
    out.write_all(output)?;
    Ok(())
}
