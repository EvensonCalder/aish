use std::fmt;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use portable_pty::{MasterPty, NativePtySystem, PtySize, PtySystem};

mod filter;
mod launch;
mod parser;
mod protocol;
mod syntax;

use filter::PtyOutputFilter;
pub use launch::resolve_shell;
use launch::{ShellLaunch, shell_command_builder, shell_launch};
use parser::{
    marker_status_is_complete, parse_marker_output, parse_ready_cwd, parse_ready_status_output,
    parse_ready_status_output_with_prompt_separator, start_marker_command,
};
use protocol::{next_marker, ready_marker, start_marker, status_marker_command};
use syntax::input_needs_more_lines;

#[cfg(test)]
use std::env;

#[cfg(test)]
use parser::{HookCommandResult, clean_marker_echo};
#[cfg(test)]
use protocol::{READY_MARKER, START_MARKER};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub command: String,
    pub started_command: Option<String>,
    pub output: String,
    pub exit_code: i32,
    pub cwd: Option<String>,
}

pub struct PtyBackend {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    initial_cwd: Option<String>,
    shell_program: String,
    integration: ShellIntegration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuationCheck {
    pub needs_more: bool,
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellIntegration {
    MarkerCommand,
    BashPromptCommand,
    ZshHooks,
    FishEvents,
}

enum PtyReadTarget<'a> {
    Marker { marker: &'a str },
    Ready,
}

impl PtyReadTarget<'_> {
    fn is_complete(&self, data: &[u8]) -> bool {
        let current = String::from_utf8_lossy(data);
        match self {
            Self::Marker { marker } => marker_status_is_complete(&current, marker),
            Self::Ready => parse_ready_cwd(&current).is_some(),
        }
    }

    fn timeout_message(&self) -> &'static str {
        match self {
            Self::Marker { .. } => "timed out waiting for backend shell prompt marker",
            Self::Ready => "timed out waiting for backend shell ready marker",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtyCommandEvent<'a> {
    Output(&'a [u8]),
    PollInput,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendShellClosed;

impl fmt::Display for BackendShellClosed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("backend shell PTY closed")
    }
}

impl std::error::Error for BackendShellClosed {}

enum PtyReadEvent<'a> {
    Chunk(&'a [u8]),
    Idle,
}

impl PtyBackend {
    pub fn spawn(configured_shell: &str) -> Result<Self> {
        let launch = shell_launch(configured_shell);
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(default_pty_size())
            .context("failed to create PTY")?;

        let command = shell_command_builder(&launch);

        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("failed to spawn backend shell {}", launch.program))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .context("failed to clone PTY reader")?;
        let writer = pair
            .master
            .take_writer()
            .context("failed to open PTY writer")?;
        drop(pair.slave);

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0_u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let mut backend = Self {
            master: pair.master,
            writer,
            output: rx,
            child,
            initial_cwd: None,
            shell_program: launch.program.clone(),
            integration: launch.integration,
        };
        backend.initialize_shell(&launch)?;
        Ok(backend)
    }

    fn initialize_shell(&mut self, launch: &ShellLaunch) -> Result<()> {
        self.write_raw(&launch.init_command)?;
        let mut on_wait = no_wait;
        let raw = self.read_until_ready(Duration::from_secs(5), &mut on_wait)?;
        self.initial_cwd = parse_ready_cwd(&raw);
        let _ = self.drain_for(Duration::from_millis(150));
        Ok(())
    }

    pub fn initial_cwd(&self) -> Option<&str> {
        self.initial_cwd.as_deref()
    }

    pub fn shell_program(&self) -> &str {
        &self.shell_program
    }

    pub fn resize(&mut self, size: PtySize) -> Result<()> {
        self.master.resize(size).context("failed to resize PTY")
    }

    pub fn size(&self) -> Result<PtySize> {
        self.master.get_size().context("failed to read PTY size")
    }

    pub fn write_raw(&mut self, text: &str) -> Result<()> {
        self.writer
            .write_all(text.as_bytes())
            .context("failed to write to PTY")?;
        self.writer.flush().context("failed to flush PTY")?;
        Ok(())
    }

    pub fn input_needs_more_lines(&self, input: &str) -> Result<ContinuationCheck> {
        input_needs_more_lines(&self.shell_program, input)
    }

    pub fn run_command(&mut self, command: &str, timeout: Duration) -> Result<CommandResult> {
        self.run_command_with_wait_callback(command, timeout, no_wait)
    }

    pub fn run_command_with_wait_callback<F>(
        &mut self,
        command: &str,
        timeout: Duration,
        mut on_wait: F,
    ) -> Result<CommandResult>
    where
        F: FnMut(&mut Self) -> Result<bool>,
    {
        if self.uses_shell_events() {
            if self.shell_events_require_marker_for(command) {
                return self.run_command_with_marker(command, timeout, &mut on_wait);
            }
            return self.run_command_with_shell_events(command, timeout, &mut on_wait);
        }

        self.run_command_with_marker(command, timeout, &mut on_wait)
    }

    pub fn run_command_streaming_with_wait_callback<F, G>(
        &mut self,
        command: &str,
        timeout: Duration,
        mut on_wait: F,
        mut on_output: G,
    ) -> Result<CommandResult>
    where
        F: FnMut(&mut Self) -> Result<bool>,
        G: FnMut(&[u8]) -> Result<()>,
    {
        self.run_command_with_event_callback(command, timeout, |backend, event| match event {
            PtyCommandEvent::Output(chunk) => {
                on_output(chunk)?;
                Ok(false)
            }
            PtyCommandEvent::PollInput | PtyCommandEvent::Idle => on_wait(backend),
        })
    }

    pub fn run_command_with_event_callback<F>(
        &mut self,
        command: &str,
        timeout: Duration,
        mut on_event: F,
    ) -> Result<CommandResult>
    where
        F: FnMut(&mut Self, PtyCommandEvent<'_>) -> Result<bool>,
    {
        self.run_command_with_event_callback_inner(command, Some(timeout), &mut on_event)
    }

    pub fn run_command_passthrough_with_event_callback<F>(
        &mut self,
        command: &str,
        mut on_event: F,
    ) -> Result<CommandResult>
    where
        F: FnMut(&mut Self, PtyCommandEvent<'_>) -> Result<bool>,
    {
        self.run_command_with_event_callback_inner(command, None, &mut on_event)
    }

    fn run_command_with_event_callback_inner<F>(
        &mut self,
        command: &str,
        timeout: Option<Duration>,
        on_event: &mut F,
    ) -> Result<CommandResult>
    where
        F: FnMut(&mut Self, PtyCommandEvent<'_>) -> Result<bool>,
    {
        let backend_command = if timeout.is_none() {
            self.passthrough_backend_command(command)
        } else {
            command.to_string()
        };
        if self.uses_shell_events() {
            if self.shell_events_require_marker_for(command) {
                return self.run_command_with_marker_events(
                    command,
                    &backend_command,
                    timeout,
                    on_event,
                );
            }
            return self.run_command_with_shell_events_streaming(
                command,
                &backend_command,
                timeout,
                on_event,
            );
        }

        self.run_command_with_marker_events(command, &backend_command, timeout, on_event)
    }

    fn passthrough_backend_command(&self, command: &str) -> String {
        let command = command.trim_end_matches('\n');
        match self.integration {
            ShellIntegration::ZshHooks | ShellIntegration::FishEvents => command.to_string(),
            ShellIntegration::BashPromptCommand | ShellIntegration::MarkerCommand => format!(
                " stty echo; {{\n{command}\n}}; __aish_passthrough_status=$?; stty -echo; __aish_preserve_status \"$__aish_passthrough_status\""
            ),
        }
    }

    fn uses_shell_events(&self) -> bool {
        matches!(
            self.integration,
            ShellIntegration::BashPromptCommand
                | ShellIntegration::ZshHooks
                | ShellIntegration::FishEvents
        )
    }

    fn shell_events_require_marker_for(&self, command: &str) -> bool {
        command.contains('\n')
            && matches!(
                self.integration,
                ShellIntegration::BashPromptCommand | ShellIntegration::ZshHooks
            )
    }

    fn run_command_with_marker<F>(
        &mut self,
        command: &str,
        timeout: Duration,
        on_wait: &mut F,
    ) -> Result<CommandResult>
    where
        F: FnMut(&mut Self) -> Result<bool>,
    {
        let _ = self.drain_for(Duration::from_millis(25));
        let marker = next_marker();
        let start_command = start_marker_command(command);
        let marker_command = status_marker_command(&marker);
        if !command.contains('\n') {
            self.write_raw(&start_command)?;
        }
        self.write_raw(command)?;
        if !command.ends_with('\n') {
            self.write_raw("\n")?;
        }
        self.write_raw(&marker_command)?;

        let raw = self.read_until_marker(&marker, &marker_command, timeout, on_wait)?;
        let (output, exit_code, cwd, started_command) = parse_marker_output(&raw, &marker)?;
        let command_text = command.trim_end_matches('\n').to_string();
        Ok(CommandResult {
            command: command_text.clone(),
            started_command: started_command.or(Some(command_text)),
            output,
            exit_code,
            cwd,
        })
    }

    fn run_command_with_marker_events<F>(
        &mut self,
        command: &str,
        backend_command: &str,
        timeout: Option<Duration>,
        on_event: &mut F,
    ) -> Result<CommandResult>
    where
        F: FnMut(&mut Self, PtyCommandEvent<'_>) -> Result<bool>,
    {
        let _ = self.drain_for(Duration::from_millis(25));
        let marker = next_marker();
        let start_command = start_marker_command(command);
        let marker_command = status_marker_command(&marker);
        if !command.contains('\n') {
            self.write_raw(&start_command)?;
        }
        self.write_raw(backend_command)?;
        if !backend_command.ends_with('\n') {
            self.write_raw("\n")?;
        }
        self.write_raw(&marker_command)?;

        let raw = self.read_until_marker_streaming(&marker, &marker_command, timeout, on_event)?;
        let (output, exit_code, cwd, started_command) = parse_marker_output(&raw, &marker)?;
        let command_text = command.trim_end_matches('\n').to_string();
        Ok(CommandResult {
            command: command_text.clone(),
            started_command: started_command.or(Some(command_text)),
            output,
            exit_code,
            cwd,
        })
    }

    fn read_until_marker<F>(
        &mut self,
        marker: &str,
        recovery_marker_command: &str,
        timeout: Duration,
        on_wait: &mut F,
    ) -> Result<String>
    where
        F: FnMut(&mut Self) -> Result<bool>,
    {
        self.read_pty_until(
            PtyReadTarget::Marker { marker },
            Some(timeout),
            |backend, event| {
                if let PtyReadEvent::Idle = event
                    && on_wait(backend)?
                {
                    std::thread::sleep(Duration::from_millis(100));
                    backend.write_raw("\n")?;
                    backend.write_raw(recovery_marker_command)?;
                }
                Ok(())
            },
        )
    }

    fn read_until_marker_streaming<F>(
        &mut self,
        marker: &str,
        recovery_marker_command: &str,
        timeout: Option<Duration>,
        on_event: &mut F,
    ) -> Result<String>
    where
        F: FnMut(&mut Self, PtyCommandEvent<'_>) -> Result<bool>,
    {
        let mut output_filter = PtyOutputFilter::marker(marker);
        let mut marker_needs_reissue = false;
        self.read_pty_until(
            PtyReadTarget::Marker { marker },
            timeout,
            |backend, event| {
                match event {
                    PtyReadEvent::Chunk(chunk) => {
                        let display = output_filter.push(chunk);
                        if !display.is_empty() {
                            marker_needs_reissue |=
                                on_event(backend, PtyCommandEvent::Output(&display))?;
                        }
                        marker_needs_reissue |= on_event(backend, PtyCommandEvent::PollInput)?;
                    }
                    PtyReadEvent::Idle => {
                        let display = output_filter.flush_pending();
                        if !display.is_empty() {
                            marker_needs_reissue |=
                                on_event(backend, PtyCommandEvent::Output(&display))?;
                        }
                        marker_needs_reissue |= on_event(backend, PtyCommandEvent::Idle)?;
                        if marker_needs_reissue {
                            std::thread::sleep(Duration::from_millis(100));
                            backend.write_raw("\n")?;
                            backend.write_raw(recovery_marker_command)?;
                            marker_needs_reissue = false;
                        }
                    }
                }
                Ok(())
            },
        )
    }

    fn read_pty_until<F>(
        &mut self,
        target: PtyReadTarget<'_>,
        timeout: Option<Duration>,
        mut on_event: F,
    ) -> Result<String>
    where
        F: FnMut(&mut Self, PtyReadEvent<'_>) -> Result<()>,
    {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut data = Vec::new();
        loop {
            if target.is_complete(&data) {
                return Ok(String::from_utf8_lossy(&data).into_owned());
            }
            let now = Instant::now();
            if let Some(deadline) = deadline
                && now >= deadline
            {
                bail!(target.timeout_message());
            }
            let remaining = deadline
                .map(|deadline| deadline.saturating_duration_since(now))
                .unwrap_or_else(|| Duration::from_millis(50))
                .min(Duration::from_millis(50));
            match self.output.recv_timeout(remaining) {
                Ok(chunk) => {
                    data.extend_from_slice(&chunk);
                    on_event(self, PtyReadEvent::Chunk(&chunk))?;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    on_event(self, PtyReadEvent::Idle)?;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(BackendShellClosed.into());
                }
            }
        }
    }

    fn run_command_with_shell_events<F>(
        &mut self,
        command: &str,
        timeout: Duration,
        on_wait: &mut F,
    ) -> Result<CommandResult>
    where
        F: FnMut(&mut Self) -> Result<bool>,
    {
        let _ = self.drain_for(Duration::from_millis(25));
        self.write_raw(command)?;
        if !command.ends_with('\n') {
            self.write_raw("\n")?;
        }

        let raw = self.read_until_ready(timeout, on_wait)?;
        let parsed = if self.integration == ShellIntegration::BashPromptCommand {
            parse_ready_status_output_with_prompt_separator(&raw, false)?
        } else {
            parse_ready_status_output(&raw, self.integration == ShellIntegration::FishEvents)?
        };
        Ok(CommandResult {
            command: command.trim_end_matches('\n').to_string(),
            started_command: parsed
                .started_command
                .or_else(|| Some(command.trim_end_matches('\n').to_string())),
            output: parsed.output,
            exit_code: parsed.exit_code,
            cwd: Some(parsed.cwd),
        })
    }

    fn run_command_with_shell_events_streaming<F>(
        &mut self,
        command: &str,
        backend_command: &str,
        timeout: Option<Duration>,
        on_event: &mut F,
    ) -> Result<CommandResult>
    where
        F: FnMut(&mut Self, PtyCommandEvent<'_>) -> Result<bool>,
    {
        let _ = self.drain_for(Duration::from_millis(25));
        self.write_raw(backend_command)?;
        if !backend_command.ends_with('\n') {
            self.write_raw("\n")?;
        }

        let raw = self.read_until_ready_streaming(timeout, on_event)?;
        let parsed = if self.integration == ShellIntegration::BashPromptCommand {
            parse_ready_status_output_with_prompt_separator(&raw, false)?
        } else {
            parse_ready_status_output(&raw, self.integration == ShellIntegration::FishEvents)?
        };
        Ok(CommandResult {
            command: command.trim_end_matches('\n').to_string(),
            started_command: if backend_command == command {
                parsed
                    .started_command
                    .or_else(|| Some(command.trim_end_matches('\n').to_string()))
            } else {
                Some(command.trim_end_matches('\n').to_string())
            },
            output: parsed.output,
            exit_code: parsed.exit_code,
            cwd: Some(parsed.cwd),
        })
    }

    fn read_until_ready<F>(&mut self, timeout: Duration, on_wait: &mut F) -> Result<String>
    where
        F: FnMut(&mut Self) -> Result<bool>,
    {
        self.read_pty_until(PtyReadTarget::Ready, Some(timeout), |backend, event| {
            if let PtyReadEvent::Idle = event {
                let _ = on_wait(backend)?;
            }
            Ok(())
        })
    }

    fn read_until_ready_streaming<F>(
        &mut self,
        timeout: Option<Duration>,
        on_event: &mut F,
    ) -> Result<String>
    where
        F: FnMut(&mut Self, PtyCommandEvent<'_>) -> Result<bool>,
    {
        let mut output_filter =
            PtyOutputFilter::shell_events(self.integration == ShellIntegration::FishEvents);
        self.read_pty_until(PtyReadTarget::Ready, timeout, |backend, event| {
            match event {
                PtyReadEvent::Chunk(chunk) => {
                    let display = output_filter.push(chunk);
                    if !display.is_empty() {
                        let _ = on_event(backend, PtyCommandEvent::Output(&display))?;
                    }
                    let _ = on_event(backend, PtyCommandEvent::PollInput)?;
                }
                PtyReadEvent::Idle => {
                    let display = output_filter.flush_pending();
                    if !display.is_empty() {
                        let _ = on_event(backend, PtyCommandEvent::Output(&display))?;
                    }
                    let _ = on_event(backend, PtyCommandEvent::Idle)?;
                }
            }
            Ok(())
        })
    }

    fn drain_for(&mut self, duration: Duration) -> String {
        let deadline = Instant::now() + duration;
        let mut data = Vec::new();
        while Instant::now() < deadline {
            match self.output.recv_timeout(Duration::from_millis(10)) {
                Ok(chunk) => data.extend(chunk),
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&data).into_owned()
    }
}

pub fn pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn default_pty_size() -> PtySize {
    pty_size(
        std::env::var("COLUMNS")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|cols| *cols > 0)
            .unwrap_or(80),
        std::env::var("LINES")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|rows| *rows > 0)
            .unwrap_or(24),
    )
}

impl Drop for PtyBackend {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn no_wait(_: &mut PtyBackend) -> Result<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests;
