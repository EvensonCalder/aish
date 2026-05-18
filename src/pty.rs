use std::fmt;
use std::io::Write;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

mod control;
mod filter;
mod launch;
mod parser;
mod protocol;
mod syntax;
#[cfg(unix)]
mod unix_backend;

use control::{CONTROL_FD, ControlChannel, ControlChannelClosed};
use filter::{PtyOutputFilter, clean_fish_repaint_lines};
pub use launch::resolve_shell;
use launch::{ShellLaunch, shell_command_builder, shell_launch};
use parser::{
    marker_status_is_complete, parse_marker_output, parse_ready_cwd, parse_ready_status_output,
    parse_ready_status_output_with_prompt_separator, start_marker_command,
};
use protocol::{next_marker, ready_marker, start_marker, status_marker_command};
use syntax::input_needs_more_lines;
#[cfg(unix)]
use unix_backend::UnixPtyBackend;

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
    pty: UnixPtyBackend,
    writer: Box<dyn Write + Send>,
    control: Option<ControlChannel>,
    control_pending: Vec<u8>,
    control_started_command: Option<String>,
    initial_cwd: Option<String>,
    shell_program: String,
    integration: ShellIntegration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
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

enum BackendReadEvent {
    Pty(Vec<u8>),
    Control,
    Timeout,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ControlReady {
    exit_code: i32,
    cwd: String,
    started_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ControlEvent {
    Start(String),
    Ready { exit_code: i32, cwd: String },
}

impl PtyBackend {
    pub fn spawn(configured_shell: &str) -> Result<Self> {
        let launch = shell_launch(configured_shell);
        let (control, child_control) = if launch.integration.supports_control_channel() {
            let (control, child_control) = ControlChannel::create()?;
            (Some(control), Some(child_control))
        } else {
            (None, None)
        };

        let command = shell_command_builder(&launch, control.as_ref().map(|_| CONTROL_FD));
        let pty = UnixPtyBackend::spawn(
            command,
            default_pty_size(),
            child_control.as_ref().map(|control| control.raw_fd()),
        )
        .with_context(|| format!("failed to spawn backend shell {}", launch.program))?;
        drop(child_control);

        let writer = pty.clone_writer()?;

        let mut backend = Self {
            pty,
            writer: Box::new(writer),
            control,
            control_pending: Vec::new(),
            control_started_command: None,
            initial_cwd: None,
            shell_program: launch.program.clone(),
            integration: launch.integration,
        };
        backend.initialize_shell(&launch)?;
        Ok(backend)
    }

    fn initialize_shell(&mut self, launch: &ShellLaunch) -> Result<()> {
        self.write_raw(&launch.init_command)?;
        if self.uses_control_channel() {
            let (raw, ready) =
                self.read_until_control_ready(Some(Duration::from_secs(5)), |_, _| Ok(()))?;
            self.initial_cwd = Some(ready.cwd).or_else(|| parse_ready_cwd(&raw));
        } else {
            let mut on_wait = no_wait;
            let raw = self.read_until_ready(Duration::from_secs(5), &mut on_wait)?;
            self.initial_cwd = parse_ready_cwd(&raw);
        }
        let _ = self.drain_for(Duration::from_millis(150));
        let _ = self.drain_control_events();
        Ok(())
    }

    pub fn initial_cwd(&self) -> Option<&str> {
        self.initial_cwd.as_deref()
    }

    pub fn shell_program(&self) -> &str {
        &self.shell_program
    }

    pub fn resize(&mut self, size: PtySize) -> Result<()> {
        self.pty.resize(size)
    }

    pub fn size(&self) -> Result<PtySize> {
        self.pty.size()
    }

    pub fn write_raw(&mut self, text: &str) -> Result<()> {
        self.write_raw_bytes(text.as_bytes())
    }

    pub fn write_raw_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer
            .write_all(bytes)
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
            if self.uses_control_channel() {
                return self.run_command_with_shell_events(command, timeout, &mut on_wait);
            } else if self.shell_events_require_marker_for(command) {
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
            self.shell_event_backend_command(command)
        };
        if self.uses_shell_events() {
            if self.uses_control_channel() {
                return self.run_command_with_shell_events_streaming(
                    command,
                    &backend_command,
                    timeout,
                    on_event,
                );
            } else if self.shell_events_require_marker_for(command) {
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
            ShellIntegration::ZshHooks | ShellIntegration::FishEvents => {
                self.wrap_multiline_shell_event_command(command)
            }
            ShellIntegration::BashPromptCommand | ShellIntegration::MarkerCommand => format!(
                " stty echo; {{\n{command}\n}}; __aish_passthrough_status=$?; stty -echo; __aish_preserve_status \"$__aish_passthrough_status\""
            ),
        }
    }

    fn shell_event_backend_command(&self, command: &str) -> String {
        self.wrap_multiline_shell_event_command(command.trim_end_matches('\n'))
    }

    fn wrap_multiline_shell_event_command(&self, command: &str) -> String {
        if !command.contains('\n') {
            return command.to_string();
        }
        match self.integration {
            ShellIntegration::BashPromptCommand | ShellIntegration::ZshHooks => {
                format!("{{\n{command}\n}}")
            }
            ShellIntegration::FishEvents => format!("begin\n{command}\nend"),
            ShellIntegration::MarkerCommand => command.to_string(),
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

    fn uses_control_channel(&self) -> bool {
        self.control.is_some()
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
            match self.read_next_event(remaining, false)? {
                BackendReadEvent::Pty(chunk) => {
                    data.extend_from_slice(&chunk);
                    on_event(self, PtyReadEvent::Chunk(&chunk))?;
                }
                BackendReadEvent::Timeout | BackendReadEvent::Control => {
                    on_event(self, PtyReadEvent::Idle)?;
                }
                BackendReadEvent::Closed => {
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
        let backend_command = self.shell_event_backend_command(command);
        self.prepare_control_command()?;
        self.write_raw(&backend_command)?;
        if !backend_command.ends_with('\n') {
            self.write_raw("\n")?;
        }

        if self.uses_control_channel() {
            let (raw, ready) = self.read_until_control_ready(Some(timeout), |backend, event| {
                if let PtyReadEvent::Idle = event {
                    let _ = on_wait(backend)?;
                }
                Ok(())
            })?;
            return Ok(self.command_result_from_control(command, &backend_command, raw, ready));
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
        self.prepare_control_command()?;
        self.write_raw(backend_command)?;
        if !backend_command.ends_with('\n') {
            self.write_raw("\n")?;
        }

        if self.uses_control_channel() {
            let mut output_filter = timeout.is_some().then(|| {
                PtyOutputFilter::control_stream(
                    self.integration == ShellIntegration::FishEvents,
                    Some(command.trim_end_matches('\n')),
                )
            });
            let (raw, ready) = self.read_until_control_ready(timeout, |backend, event| {
                match event {
                    PtyReadEvent::Chunk(chunk) => {
                        if let Some(output_filter) = &mut output_filter {
                            let display = output_filter.push(chunk);
                            if !display.is_empty() {
                                let _ = on_event(backend, PtyCommandEvent::Output(&display))?;
                            }
                        } else {
                            let _ = on_event(backend, PtyCommandEvent::Output(chunk))?;
                        }
                        let _ = on_event(backend, PtyCommandEvent::PollInput)?;
                    }
                    PtyReadEvent::Idle => {
                        if let Some(output_filter) = &mut output_filter {
                            let display = output_filter.flush_pending();
                            if !display.is_empty() {
                                let _ = on_event(backend, PtyCommandEvent::Output(&display))?;
                            }
                        }
                        let _ = on_event(backend, PtyCommandEvent::Idle)?;
                    }
                }
                Ok(())
            })?;
            if let Some(output_filter) = &mut output_filter {
                let display = output_filter.flush_pending();
                if !display.is_empty() {
                    let _ = on_event(self, PtyCommandEvent::Output(&display))?;
                }
            }
            return Ok(self.command_result_from_control(command, backend_command, raw, ready));
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

    fn read_until_control_ready<F>(
        &mut self,
        timeout: Option<Duration>,
        mut on_event: F,
    ) -> Result<(String, ControlReady)>
    where
        F: FnMut(&mut Self, PtyReadEvent<'_>) -> Result<()>,
    {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut data = Vec::new();
        loop {
            if let Some(ready) = self.next_control_ready()? {
                self.drain_available_pty_output(&mut data, &mut on_event)?;
                return Ok((String::from_utf8_lossy(&data).into_owned(), ready));
            }
            let now = Instant::now();
            if let Some(deadline) = deadline
                && now >= deadline
            {
                bail!("timed out waiting for backend shell control marker");
            }
            let remaining = deadline
                .map(|deadline| deadline.saturating_duration_since(now))
                .unwrap_or_else(|| Duration::from_millis(10))
                .min(Duration::from_millis(10));
            match self.read_next_event(remaining, true)? {
                BackendReadEvent::Pty(chunk) => {
                    data.extend_from_slice(&chunk);
                    on_event(self, PtyReadEvent::Chunk(&chunk))?;
                }
                BackendReadEvent::Control | BackendReadEvent::Timeout => {
                    on_event(self, PtyReadEvent::Idle)?;
                }
                BackendReadEvent::Closed => {
                    return Err(BackendShellClosed.into());
                }
            }
        }
    }

    fn drain_available_pty_output<F>(&mut self, data: &mut Vec<u8>, on_event: &mut F) -> Result<()>
    where
        F: FnMut(&mut Self, PtyReadEvent<'_>) -> Result<()>,
    {
        loop {
            match self.read_next_event(Duration::from_millis(0), false)? {
                BackendReadEvent::Pty(chunk) => {
                    data.extend_from_slice(&chunk);
                    on_event(self, PtyReadEvent::Chunk(&chunk))?;
                }
                BackendReadEvent::Timeout | BackendReadEvent::Control => return Ok(()),
                BackendReadEvent::Closed => return Err(BackendShellClosed.into()),
            }
        }
    }

    fn prepare_control_command(&mut self) -> Result<()> {
        if self.uses_control_channel() {
            let _ = self.drain_control_events()?;
            self.control_started_command = None;
        }
        Ok(())
    }

    fn next_control_ready(&mut self) -> Result<Option<ControlReady>> {
        for event in self.drain_control_events()? {
            match event {
                ControlEvent::Start(command) => {
                    self.control_started_command = (!command.is_empty()).then_some(command);
                }
                ControlEvent::Ready { exit_code, cwd } => {
                    return Ok(Some(ControlReady {
                        exit_code,
                        cwd,
                        started_command: self.control_started_command.take(),
                    }));
                }
            }
        }
        Ok(None)
    }

    fn drain_control_events(&mut self) -> Result<Vec<ControlEvent>> {
        self.read_control_available()?;

        let mut events = Vec::new();
        while let Some(end) = self.control_pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.control_pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            if let Some(event) = parse_control_event(&line) {
                events.push(event);
            }
        }
        Ok(events)
    }

    fn read_control_available(&mut self) -> Result<()> {
        if let Some(control) = &mut self.control {
            match control.read_available() {
                Ok(chunks) => {
                    for chunk in chunks {
                        self.control_pending.extend_from_slice(&chunk);
                    }
                }
                Err(error) if error.downcast_ref::<ControlChannelClosed>().is_some() => {
                    return Err(BackendShellClosed.into());
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn command_result_from_control(
        &self,
        command: &str,
        backend_command: &str,
        raw: String,
        ready: ControlReady,
    ) -> CommandResult {
        let started_command = if backend_command == command {
            ready
                .started_command
                .or_else(|| Some(command.trim_end_matches('\n').to_string()))
        } else {
            Some(command.trim_end_matches('\n').to_string())
        };
        let output = self.output_from_control_raw(&raw, started_command.as_deref());
        CommandResult {
            command: command.trim_end_matches('\n').to_string(),
            started_command,
            output,
            exit_code: ready.exit_code,
            cwd: Some(ready.cwd),
        }
    }

    fn output_from_control_raw(&self, raw: &str, started_command: Option<&str>) -> String {
        let normalized = normalize_pty_newlines(raw);
        if self.integration != ShellIntegration::FishEvents {
            return normalized.trim_start_matches('\n').to_string();
        }

        let output_ended_with_newline = normalized.ends_with('\n');
        let mut lines: Vec<String> = normalized.lines().map(str::to_string).collect();
        lines = clean_fish_repaint_lines(lines, started_command);
        while lines.first().is_some_and(|line| line.is_empty()) {
            lines.remove(0);
        }
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        if lines.is_empty() {
            String::new()
        } else if output_ended_with_newline {
            format!("{}\n", lines.join("\n"))
        } else {
            lines.join("\n")
        }
    }

    fn drain_for(&mut self, duration: Duration) -> String {
        let deadline = Instant::now() + duration;
        let mut data = Vec::new();
        while Instant::now() < deadline {
            match self.read_next_event(Duration::from_millis(10), self.uses_control_channel()) {
                Ok(BackendReadEvent::Pty(chunk)) => data.extend(chunk),
                Ok(BackendReadEvent::Control | BackendReadEvent::Timeout) => break,
                Ok(BackendReadEvent::Closed) | Err(_) => break,
            }
        }
        String::from_utf8_lossy(&data).into_owned()
    }

    fn read_next_event(
        &mut self,
        timeout: Duration,
        include_control: bool,
    ) -> Result<BackendReadEvent> {
        let pty_fd = self.pty.raw_fd();
        let mut fds = [
            libc::pollfd {
                fd: pty_fd,
                events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
                revents: 0,
            },
            libc::pollfd {
                fd: self
                    .control
                    .as_ref()
                    .filter(|_| include_control)
                    .map(ControlChannel::raw_fd)
                    .unwrap_or(-1),
                events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
                revents: 0,
            },
        ];
        let nfds = if include_control && self.control.is_some() {
            2
        } else {
            1
        };
        let timeout_ms = duration_to_poll_timeout_ms(timeout);
        loop {
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), nfds, timeout_ms) };
            if ready > 0 {
                break;
            }
            if ready == 0 {
                return Ok(BackendReadEvent::Timeout);
            }
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::Interrupted {
                return Err(err).context("failed to poll PTY");
            }
        }

        let control_ready = nfds == 2 && pollfd_has_event(fds[1].revents);
        if pollfd_has_event(fds[0].revents) {
            let event = match self.pty.read_chunk()? {
                Some(chunk) if chunk.is_empty() => Ok(BackendReadEvent::Timeout),
                Some(chunk) => Ok(BackendReadEvent::Pty(chunk)),
                None => Ok(BackendReadEvent::Closed),
            };
            if control_ready {
                self.read_control_available()?;
            }
            return event;
        }

        if control_ready {
            self.read_control_available()?;
            return Ok(BackendReadEvent::Control);
        }

        Ok(BackendReadEvent::Timeout)
    }
}

impl ShellIntegration {
    fn supports_control_channel(self) -> bool {
        matches!(
            self,
            ShellIntegration::BashPromptCommand
                | ShellIntegration::ZshHooks
                | ShellIntegration::FishEvents
        )
    }
}

fn parse_control_event(line: &str) -> Option<ControlEvent> {
    let line = line.trim_matches(['\r', '\n']);
    let start_prefix = format!("{}\t", start_marker());
    if let Some(command) = line.strip_prefix(&start_prefix) {
        return Some(ControlEvent::Start(command.to_string()));
    }

    let ready_prefix = format!("{}\t", ready_marker());
    let rest = line.strip_prefix(&ready_prefix)?;
    let mut parts = rest.splitn(2, '\t');
    let exit_code = parts.next()?.trim().parse::<i32>().ok()?;
    let cwd = parts.next()?.trim_end().to_string();
    (!cwd.is_empty()).then_some(ControlEvent::Ready { exit_code, cwd })
}

fn normalize_pty_newlines(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\r', "\n")
}

fn pollfd_has_event(revents: i16) -> bool {
    revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0
}

fn duration_to_poll_timeout_ms(duration: Duration) -> libc::c_int {
    duration.as_millis().min(libc::c_int::MAX as u128) as libc::c_int
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

fn no_wait(_: &mut PtyBackend) -> Result<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests;
