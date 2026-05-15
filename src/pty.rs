use std::env;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};

const MARKER_PREFIX: &str = "__AISH_STATUS__";
const READY_MARKER: &str = "__AISH_READY__";
const START_MARKER: &str = "__AISH_START__";
static NEXT_MARKER_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub command: String,
    pub started_command: Option<String>,
    pub output: String,
    pub exit_code: i32,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HookCommandResult {
    output: String,
    exit_code: i32,
    cwd: String,
    started_command: Option<String>,
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
        if ends_with_shell_line_continuation(input) {
            return Ok(ContinuationCheck {
                needs_more: true,
                prompt: Some("> ".to_string()),
            });
        }

        let shell_name = Path::new(&self.shell_program)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let mut command = ProcessCommand::new(&self.shell_program);
        match shell_name {
            "bash" => {
                command.args(["--noprofile", "--norc", "-n"]);
            }
            "zsh" => {
                command.args(["-f", "-n"]);
            }
            _ => {
                return Ok(ContinuationCheck {
                    needs_more: false,
                    prompt: None,
                });
            }
        }

        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to syntax-check input with {}", self.shell_program))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input.as_bytes())
                .context("failed to write input to shell syntax check")?;
            stdin
                .write_all(b"\n")
                .context("failed to finish shell syntax check input")?;
        }
        let output = child
            .wait_with_output()
            .context("failed to read shell syntax check result")?;
        if output.status.success() {
            return Ok(ContinuationCheck {
                needs_more: false,
                prompt: None,
            });
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(ContinuationCheck {
            needs_more: is_incomplete_shell_syntax(&stderr),
            prompt: shell_continuation_prompt(&stderr),
        })
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
        if matches!(
            self.integration,
            ShellIntegration::ZshHooks | ShellIntegration::FishEvents
        ) {
            if self.integration == ShellIntegration::ZshHooks && command.contains('\n') {
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
        if matches!(
            self.integration,
            ShellIntegration::ZshHooks | ShellIntegration::FishEvents
        ) {
            if self.integration == ShellIntegration::ZshHooks && command.contains('\n') {
                return self.run_command_with_marker_events(command, timeout, &mut on_event);
            }
            return self.run_command_with_shell_events_streaming(command, timeout, &mut on_event);
        }

        self.run_command_with_marker_events(command, timeout, &mut on_event)
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
        let marker_command = format!(
            " __aish_status=$?; command -v __aish_run_prompt_command >/dev/null 2>&1 && __aish_run_prompt_command >/dev/null 2>&1; printf '\\n%s%s\\t%s\\n' '{marker}' \"$__aish_status\" \"$PWD\"; sh -c \"exit $__aish_status\"\n"
        );
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
        timeout: Duration,
        on_event: &mut F,
    ) -> Result<CommandResult>
    where
        F: FnMut(&mut Self, PtyCommandEvent<'_>) -> Result<bool>,
    {
        let _ = self.drain_for(Duration::from_millis(25));
        let marker = next_marker();
        let start_command = start_marker_command(command);
        let marker_command = format!(
            " __aish_status=$?; command -v __aish_run_prompt_command >/dev/null 2>&1 && __aish_run_prompt_command >/dev/null 2>&1; printf '\\n%s%s\\t%s\\n' '{marker}' \"$__aish_status\" \"$PWD\"; sh -c \"exit $__aish_status\"\n"
        );
        if !command.contains('\n') {
            self.write_raw(&start_command)?;
        }
        self.write_raw(command)?;
        if !command.ends_with('\n') {
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
            timeout,
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
        timeout: Duration,
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
        timeout: Duration,
        mut on_event: F,
    ) -> Result<String>
    where
        F: FnMut(&mut Self, PtyReadEvent<'_>) -> Result<()>,
    {
        let deadline = Instant::now() + timeout;
        let mut data = Vec::new();
        loop {
            if target.is_complete(&data) {
                return Ok(String::from_utf8_lossy(&data).into_owned());
            }
            let now = Instant::now();
            if now >= deadline {
                bail!(target.timeout_message());
            }
            let remaining = deadline
                .saturating_duration_since(now)
                .min(Duration::from_millis(50));
            match self.output.recv_timeout(remaining) {
                Ok(chunk) => {
                    data.extend_from_slice(&chunk);
                    on_event(self, PtyReadEvent::Chunk(&chunk))?;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    on_event(self, PtyReadEvent::Idle)?;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => bail!("backend shell PTY closed"),
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
        let parsed =
            parse_ready_status_output(&raw, self.integration == ShellIntegration::FishEvents)?;
        Ok(CommandResult {
            command: command.trim_end_matches('\n').to_string(),
            started_command: parsed.started_command,
            output: parsed.output,
            exit_code: parsed.exit_code,
            cwd: Some(parsed.cwd),
        })
    }

    fn run_command_with_shell_events_streaming<F>(
        &mut self,
        command: &str,
        timeout: Duration,
        on_event: &mut F,
    ) -> Result<CommandResult>
    where
        F: FnMut(&mut Self, PtyCommandEvent<'_>) -> Result<bool>,
    {
        let _ = self.drain_for(Duration::from_millis(25));
        self.write_raw(command)?;
        if !command.ends_with('\n') {
            self.write_raw("\n")?;
        }

        let raw = self.read_until_ready_streaming(timeout, on_event)?;
        let parsed =
            parse_ready_status_output(&raw, self.integration == ShellIntegration::FishEvents)?;
        Ok(CommandResult {
            command: command.trim_end_matches('\n').to_string(),
            started_command: parsed.started_command,
            output: parsed.output,
            exit_code: parsed.exit_code,
            cwd: Some(parsed.cwd),
        })
    }

    fn read_until_ready<F>(&mut self, timeout: Duration, on_wait: &mut F) -> Result<String>
    where
        F: FnMut(&mut Self) -> Result<bool>,
    {
        self.read_pty_until(PtyReadTarget::Ready, timeout, |backend, event| {
            if let PtyReadEvent::Idle = event {
                let _ = on_wait(backend)?;
            }
            Ok(())
        })
    }

    fn read_until_ready_streaming<F>(
        &mut self,
        timeout: Duration,
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

fn next_marker() -> String {
    let id = NEXT_MARKER_ID.fetch_add(1, Ordering::Relaxed);
    format!("{MARKER_PREFIX}{id}__")
}

fn marker_status_is_complete(raw: &str, marker: &str) -> bool {
    find_complete_marker(raw, marker).is_some()
}

fn find_complete_marker(raw: &str, marker: &str) -> Option<usize> {
    raw.match_indices(marker)
        .find_map(|(marker_pos, _)| marker_has_complete_status(raw, marker, marker_pos))
}

fn marker_has_complete_status(raw: &str, marker: &str, marker_pos: usize) -> Option<usize> {
    let status_start = marker_pos + marker.len();
    let mut chars = raw[status_start..].chars();
    let first = chars.next()?;
    if !first.is_ascii_digit() {
        return None;
    }
    chars
        .any(|ch| ch == '\n' || ch == '\r')
        .then_some(marker_pos)
}

fn start_marker_command(command: &str) -> String {
    let display_command = command.trim_end_matches('\n').replace(['\r', '\n'], "\\n");
    format!(
        " printf '\n{START_MARKER}\t%s\n' {}\n",
        shell_single_quote(&display_command)
    )
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

struct PtyOutputFilter {
    marker: String,
    pending: Vec<u8>,
    deferred_separator: Vec<u8>,
    fish: Option<FishOutputFilter>,
    command_complete: bool,
}

struct FishOutputFilter {
    command_active: bool,
    started_command: Option<String>,
    held_segment: Option<FishHeldSegment>,
}

struct FishHeldSegment {
    bytes: Vec<u8>,
    visible: String,
}

enum InternalMarker {
    Start(String),
    Ready,
    Status,
}

impl PtyOutputFilter {
    fn marker(marker: &str) -> Self {
        Self {
            marker: marker.to_string(),
            pending: Vec::new(),
            deferred_separator: Vec::new(),
            fish: None,
            command_complete: false,
        }
    }

    fn shell_events(filter_fish_repaint: bool) -> Self {
        Self {
            marker: String::new(),
            pending: Vec::new(),
            deferred_separator: Vec::new(),
            fish: filter_fish_repaint.then_some(FishOutputFilter {
                command_active: false,
                started_command: None,
                held_segment: None,
            }),
            command_complete: false,
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.pending.extend_from_slice(chunk);
        let mut output = Vec::new();
        while let Some(end) = next_terminal_line_end(&self.pending) {
            let segment: Vec<u8> = self.pending.drain(..end).collect();
            self.push_segment(segment, &mut output);
        }
        output
    }

    fn push_segment(&mut self, segment: Vec<u8>, output: &mut Vec<u8>) {
        if self.command_complete {
            self.deferred_separator.clear();
            return;
        }
        if let Some(marker) = self.internal_marker_segment(&segment) {
            self.handle_internal_marker(marker, output);
            return;
        }
        if self.fish.as_ref().is_some_and(|fish| !fish.command_active) {
            self.deferred_separator.clear();
            return;
        }
        if terminal_separator_only(&segment) {
            if self
                .fish
                .as_ref()
                .is_some_and(|fish| fish.held_segment.is_some())
            {
                self.deferred_separator.extend_from_slice(&segment);
            } else {
                self.push_deferred_separator(output);
                self.deferred_separator = segment;
            }
            return;
        }
        let Some(segment) = self.filter_fish_segment(segment, output) else {
            return;
        };
        if !self.deferred_separator.is_empty() {
            output.extend_from_slice(&self.deferred_separator);
            self.deferred_separator.clear();
        }
        output.extend_from_slice(&segment);
    }

    fn internal_marker_segment(&self, segment: &[u8]) -> Option<InternalMarker> {
        let text = String::from_utf8_lossy(segment);
        let cleaned = strip_terminal_control_sequences(&text);
        let line = cleaned
            .trim_matches(['\r', '\n'])
            .trim_start_matches([' ', '\t']);
        if let Some(command) = line.strip_prefix(&format!("{START_MARKER}\t")) {
            return Some(InternalMarker::Start(command.trim_end().to_string()));
        }
        if line.starts_with(START_MARKER) {
            return Some(InternalMarker::Start(String::new()));
        }
        if line.starts_with(READY_MARKER) {
            return Some(InternalMarker::Ready);
        }
        if !self.marker.is_empty() && line.starts_with(&self.marker) {
            return Some(InternalMarker::Status);
        }
        None
    }

    fn handle_internal_marker(&mut self, marker: InternalMarker, output: &mut Vec<u8>) {
        match marker {
            InternalMarker::Start(command) => {
                if let Some(fish) = &mut self.fish {
                    fish.command_active = true;
                    fish.started_command = Some(command);
                    fish.held_segment = None;
                }
            }
            InternalMarker::Ready => {
                self.flush_fish_held_segment(output);
                if let Some(fish) = &mut self.fish {
                    fish.command_active = false;
                    fish.held_segment = None;
                }
                self.command_complete = true;
            }
            InternalMarker::Status => {
                self.command_complete = true;
            }
        }
        self.deferred_separator.clear();
    }

    fn filter_fish_segment(&mut self, segment: Vec<u8>, output: &mut Vec<u8>) -> Option<Vec<u8>> {
        if self.fish.is_none() {
            return Some(segment);
        }

        let text = String::from_utf8_lossy(&segment);
        let has_repaint_control = contains_terminal_repaint_control(&text);
        let visible = strip_terminal_control_sequences(&text).trim().to_string();

        if visible.starts_with('\u{23ce}') || visible.is_empty() && has_repaint_control {
            return None;
        }

        let started_command = self
            .fish
            .as_ref()
            .and_then(|fish| fish.started_command.as_deref());
        if let Some(command) = started_command
            && is_fish_repaint_echo_fragment(&visible, command, has_repaint_control)
        {
            return None;
        }

        if let Some(held) = self.fish.as_mut().and_then(|fish| fish.held_segment.take()) {
            if held.visible == visible && !has_repaint_control {
                self.deferred_separator.clear();
            } else {
                output.extend_from_slice(&held.bytes);
                self.push_deferred_separator(output);
            }
        }

        if has_repaint_control && !visible.is_empty() {
            if let Some(fish) = &mut self.fish {
                fish.held_segment = Some(FishHeldSegment {
                    bytes: segment,
                    visible,
                });
            }
            return None;
        }

        Some(segment)
    }

    fn flush_fish_held_segment(&mut self, output: &mut Vec<u8>) {
        let held = self.fish.as_mut().and_then(|fish| fish.held_segment.take());
        let Some(held) = held else {
            return;
        };
        let started_command = self
            .fish
            .as_ref()
            .and_then(|fish| fish.started_command.as_deref());
        if let Some(command) = started_command
            && is_fish_command_repaint_token(&held.visible, command)
        {
            self.deferred_separator.clear();
            return;
        }
        output.extend_from_slice(&held.bytes);
        self.push_deferred_separator(output);
    }

    fn push_deferred_separator(&mut self, output: &mut Vec<u8>) {
        if !self.deferred_separator.is_empty() {
            output.extend_from_slice(&self.deferred_separator);
            self.deferred_separator.clear();
        }
    }
}

fn next_terminal_line_end(bytes: &[u8]) -> Option<usize> {
    for (index, byte) in bytes.iter().enumerate() {
        match *byte {
            b'\n' => return Some(index + 1),
            b'\r' => {
                if index + 1 >= bytes.len() {
                    return None;
                }
                if bytes[index + 1] == b'\n' {
                    return Some(index + 2);
                }
                return Some(index + 1);
            }
            _ => {}
        }
    }
    None
}

fn terminal_separator_only(segment: &[u8]) -> bool {
    segment.iter().all(|byte| matches!(*byte, b'\r' | b'\n'))
}

pub fn resolve_shell(configured_shell: &str) -> String {
    if configured_shell != "auto" && !configured_shell.trim().is_empty() {
        return configured_shell.to_string();
    }
    env::var("SHELL")
        .ok()
        .filter(|shell| !shell.trim().is_empty())
        .unwrap_or_else(|| "/bin/bash".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellLaunch {
    program: String,
    args: Vec<String>,
    init_command: String,
    integration: ShellIntegration,
}

fn shell_launch(configured_shell: &str) -> ShellLaunch {
    let program = resolve_shell(configured_shell);
    let shell_name = Path::new(&program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    let (args, init_command, integration) = match shell_name {
        "bash" => (
            vec!["-i".to_string()],
            format!(
                " export HISTCONTROL=ignorespace${{HISTCONTROL:+:$HISTCONTROL}}; __aish_prompt_command_set=0; __aish_prompt_command_is_array=0; __aish_prompt_command_string=; __aish_prompt_command_array=(); if declare -p PROMPT_COMMAND >/dev/null 2>&1; then __aish_prompt_command_set=1; case \"$(declare -p PROMPT_COMMAND 2>/dev/null)\" in declare\\ -a*|declare\\ -A*) __aish_prompt_command_is_array=1; __aish_prompt_command_array=(\"${{PROMPT_COMMAND[@]}}\");; *) __aish_prompt_command_string=$PROMPT_COMMAND;; esac; fi; PROMPT_COMMAND=; trap - DEBUG 2>/dev/null || true; __aish_run_prompt_command() {{ if [ \"$__aish_prompt_command_set\" = 1 ]; then if [ \"$__aish_prompt_command_is_array\" = 1 ]; then local __aish_pc; for __aish_pc in \"${{__aish_prompt_command_array[@]}}\"; do eval \"$__aish_pc\"; done; else eval \"$__aish_prompt_command_string\"; fi; fi; }}; bind 'set enable-bracketed-paste off' 2>/dev/null || true; PS1=''; PS2=''; stty -echo; __aish_run_prompt_command >/dev/null 2>&1; printf '\\n{READY_MARKER}\\t%s\\n' \"$PWD\"\n"
            ),
            ShellIntegration::MarkerCommand,
        ),
        "zsh" => (
            vec![
                "-i".to_string(),
                "-o".to_string(),
                "histignorespace".to_string(),
            ],
            format!(
                " setopt histignorespace; stty -echo; unsetopt zle prompt_cr prompt_sp; PROMPT=''; RPROMPT=''; PROMPT2=''; function __aish_preexec() {{ printf '\\n{START_MARKER}\\t%s\\n' \"$1\"; }}; function __aish_precmd() {{ printf '\\n{READY_MARKER}\\t%s\\t%s\\n' \"$?\" \"$PWD\"; }}; autoload -Uz add-zsh-hook; add-zsh-hook -d preexec __aish_preexec 2>/dev/null || true; add-zsh-hook -d precmd __aish_precmd 2>/dev/null || true; add-zsh-hook preexec __aish_preexec; add-zsh-hook precmd __aish_precmd; preexec_functions=(__aish_preexec ${{preexec_functions:#__aish_preexec}}); precmd_functions=(__aish_precmd ${{precmd_functions:#__aish_precmd}}); __aish_precmd\n"
            ),
            ShellIntegration::ZshHooks,
        ),
        "fish" => (
            fish_launch_args(&program),
            format!(
                "stty -echo; set -g fish_greeting; function fish_title; end; function __aish_preexec --on-event fish_preexec; printf '\n{START_MARKER}\\t%s\n' $argv[1]; end; function fish_prompt; printf '\n{READY_MARKER}\\t%s\\t%s\n' $status $PWD; end; function fish_right_prompt; end; function fish_mode_prompt; end; fish_prompt\n"
            ),
            ShellIntegration::FishEvents,
        ),
        _ => (
            Vec::new(),
            format!("stty -echo; printf '\\n{READY_MARKER}\\t%s\\n' \"$PWD\"\n"),
            ShellIntegration::MarkerCommand,
        ),
    };

    ShellLaunch {
        program,
        args,
        init_command,
        integration,
    }
}

fn fish_launch_args(program: &str) -> Vec<String> {
    let mut args = Vec::new();
    if fish_supports_features(program, "no-query-term,no-mark-prompt") {
        args.push("--features".to_string());
        args.push("no-query-term,no-mark-prompt".to_string());
    }
    args
}

fn fish_supports_features(program: &str, features: &str) -> bool {
    ProcessCommand::new(program)
        .args(["--no-config", "--features", features, "-c", "true"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn shell_command_builder(launch: &ShellLaunch) -> CommandBuilder {
    let mut command = CommandBuilder::new(&launch.program);
    for arg in &launch.args {
        command.arg(arg);
    }
    if let Ok(cwd) = env::current_dir() {
        command.cwd(cwd);
    }
    command.env("BASH_SILENCE_DEPRECATION_WARNING", "1");
    command
}

fn parse_marker_output(
    raw: &str,
    marker: &str,
) -> Result<(String, i32, Option<String>, Option<String>)> {
    let marker_pos = find_complete_marker(raw, marker)
        .context("backend shell output did not contain prompt marker")?;
    let before_marker = normalize_pty_newlines(strip_marker_separator(&raw[..marker_pos]))
        .trim_start_matches('\n')
        .to_string();
    let started_command = parse_started_command(&before_marker);
    let output = clean_marker_echo(&before_marker, marker);
    let status_start = marker_pos + marker.len();
    let status: String = raw[status_start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if status.is_empty() {
        bail!("backend shell prompt marker did not include exit status");
    }
    let exit_code = status.parse::<i32>().context("invalid shell exit status")?;
    let cwd_start = status_start + status.len();
    let cwd = raw[cwd_start..]
        .strip_prefix('\t')
        .map(|rest| {
            normalize_pty_newlines(rest)
                .lines()
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .filter(|cwd| !cwd.is_empty());
    Ok((output, exit_code, cwd, started_command))
}

fn parse_started_command(output: &str) -> Option<String> {
    let prefix = format!("{START_MARKER}\t");
    output
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .next_back()
        .map(str::to_string)
        .filter(|command| !command.is_empty())
}

fn strip_marker_separator(output: &str) -> &str {
    output
        .strip_suffix("\r\n")
        .or_else(|| output.strip_suffix('\n'))
        .or_else(|| output.strip_suffix('\r'))
        .unwrap_or(output)
}

fn parse_ready_cwd(raw: &str) -> Option<String> {
    let prefix = format!("{READY_MARKER}\t");
    let normalized = normalize_pty_newlines(raw);
    complete_normalized_lines(&normalized)
        .into_iter()
        .find_map(|line| {
            let cleaned = strip_terminal_control_sequences(line);
            cleaned
                .trim_start()
                .strip_prefix(&prefix)
                .and_then(parse_ready_marker_fields)
        })
        .map(|fields| fields.cwd)
}

fn parse_ready_status_output(raw: &str, strip_terminal_repaint: bool) -> Result<HookCommandResult> {
    let raw = normalize_pty_newlines(raw);
    let mut ready = None;
    let mut current_started_command = None;
    let mut saw_start_marker = false;
    let mut pre_start_output_lines = Vec::new();
    let mut command_output_lines = Vec::new();

    for line in complete_normalized_lines(&raw) {
        let cleaned_marker_line = strip_terminal_control_sequences(line);
        let marker_line = cleaned_marker_line.trim_start();
        if let Some(command) = marker_line.strip_prefix(&format!("{START_MARKER}\t")) {
            current_started_command = Some(command.to_string());
            saw_start_marker = true;
            command_output_lines.clear();
            continue;
        }
        if let Some(rest) = marker_line.strip_prefix(&format!("{READY_MARKER}\t")) {
            if let Some(fields) = parse_ready_marker_fields(rest)
                && let Some(status) = fields.status
            {
                let output_lines = if saw_start_marker {
                    command_output_lines.clone()
                } else {
                    pre_start_output_lines.clone()
                };
                ready = Some(ReadyStatusSnapshot {
                    status,
                    cwd: fields.cwd,
                    output_lines,
                    started_command: current_started_command.clone(),
                });
            }
            continue;
        }
        if saw_start_marker {
            command_output_lines.push(line.to_string());
        } else {
            pre_start_output_lines.push(line.to_string());
        }
    }

    let ready = ready.context("backend shell output did not contain ready marker")?;
    let mut output_lines = ready.output_lines;
    let exit_code = ready
        .status
        .trim()
        .parse::<i32>()
        .context("invalid shell exit status in ready marker")?;
    let started_command = ready.started_command;
    if strip_terminal_repaint {
        output_lines = clean_fish_repaint_lines(output_lines, started_command.as_deref());
    }
    while output_lines.first().is_some_and(|line| line.is_empty()) {
        output_lines.remove(0);
    }
    while output_lines.last().is_some_and(|line| line.is_empty()) {
        output_lines.pop();
    }
    let output = if output_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", output_lines.join("\n"))
    };

    Ok(HookCommandResult {
        output,
        exit_code,
        cwd: ready.cwd,
        started_command,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadyMarkerFields {
    status: Option<String>,
    cwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadyStatusSnapshot {
    status: String,
    cwd: String,
    output_lines: Vec<String>,
    started_command: Option<String>,
}

fn parse_ready_marker_fields(rest: &str) -> Option<ReadyMarkerFields> {
    let mut parts = rest.splitn(2, '\t');
    let first = strip_terminal_control_sequences(parts.next()?);
    if let Some(cwd) = parts.next() {
        let status = first.trim().to_string();
        if status.parse::<i32>().is_err() {
            return None;
        }
        let cwd = strip_terminal_control_sequences(cwd).trim_end().to_string();
        return (!cwd.is_empty()).then_some(ReadyMarkerFields {
            status: Some(status),
            cwd,
        });
    }

    let cwd = first.trim_end().to_string();
    (!cwd.is_empty() && cwd.parse::<i32>().is_err())
        .then_some(ReadyMarkerFields { status: None, cwd })
}

fn clean_fish_repaint_lines(
    output_lines: Vec<String>,
    started_command: Option<&str>,
) -> Vec<String> {
    let lines: Vec<(String, bool)> = output_lines
        .into_iter()
        .filter_map(|line| {
            let had_terminal_control = contains_terminal_control(&line);
            let cleaned = strip_terminal_control_sequences(&line);
            let visible = cleaned.trim();
            if visible.starts_with('\u{23ce}') {
                return None;
            }
            if visible.is_empty() && had_terminal_control {
                return None;
            }
            if let Some(command) = started_command
                && is_fish_repaint_echo_fragment(visible, command, had_terminal_control)
            {
                return None;
            }
            Some((cleaned.trim_end().to_string(), had_terminal_control))
        })
        .collect();

    let mut deduped = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index].0;
        let mut end = index + 1;
        while end < lines.len() && lines[end].0 == *line {
            end += 1;
        }
        let run = &lines[index..end];
        if run.len() == 1 {
            deduped.push(line.clone());
        } else {
            let normal_lines: Vec<String> = run
                .iter()
                .filter(|(_, had_terminal_control)| !*had_terminal_control)
                .map(|(line, _)| line.clone())
                .collect();
            if normal_lines.is_empty() {
                deduped.push(line.clone());
            } else {
                deduped.extend(normal_lines);
            }
        }
        index = end;
    }
    deduped
}

fn is_fish_repaint_echo_fragment(line: &str, command: &str, had_terminal_control: bool) -> bool {
    if line.is_empty() {
        return false;
    }
    let line = line.trim_start_matches(['>', '=']).trim_start().trim_end();
    if line.is_empty() {
        return true;
    }
    line == command
        || command.starts_with(line)
        || shell_syntax_fragment(line)
            && (command.ends_with(line) || command_contains_repaint_token(command, line))
        || had_terminal_control && command_contains_repaint_token(command, line)
}

fn command_contains_repaint_token(command: &str, line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    if line
        .chars()
        .all(|ch| matches!(ch, ';' | '|' | '&' | '<' | '>'))
    {
        return command.contains(line);
    }
    let line = line.trim_matches(['\'', '"', ';']);
    let tokens: Vec<&str> = command
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '<' | '>'))
        .map(|part| part.trim_matches(['\'', '"']))
        .filter(|part| !part.is_empty())
        .collect();
    if !line.contains(['\'', '"', '\\', '$', ';', '|', '&', '<', '>']) {
        return tokens
            .iter()
            .position(|part| *part == line)
            .is_some_and(|index| index + 1 < tokens.len());
    }
    tokens.contains(&line)
}

fn is_fish_command_repaint_token(line: &str, command: &str) -> bool {
    let line = line.trim_start_matches(['>', '=']).trim_start().trim_end();
    if line.is_empty() || line == command || command.starts_with(line) {
        return true;
    }
    let line = line.trim_matches(['\'', '"', ';']);
    if line.is_empty() {
        return true;
    }
    command
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '<' | '>'))
        .map(|part| part.trim_matches(['\'', '"', ';']))
        .filter(|part| !part.is_empty())
        .any(|part| part == line)
}

fn shell_syntax_fragment(line: &str) -> bool {
    line.contains(['\'', '"', '\\', '$', ';', '|', '&', '<', '>'])
}

fn contains_terminal_control(text: &str) -> bool {
    text.as_bytes()
        .iter()
        .any(|byte| *byte == 0x1b || *byte < 0x20 && *byte != b'\t')
}

fn contains_terminal_repaint_control(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            0x1b => {
                index += 1;
                if index >= bytes.len() {
                    return true;
                }
                match bytes[index] {
                    b'[' => {
                        index += 1;
                        while index < bytes.len() {
                            let byte = bytes[index];
                            index += 1;
                            if (0x40..=0x7e).contains(&byte) {
                                if matches!(
                                    byte,
                                    b'A' | b'B'
                                        | b'C'
                                        | b'D'
                                        | b'E'
                                        | b'F'
                                        | b'G'
                                        | b'H'
                                        | b'J'
                                        | b'K'
                                        | b'S'
                                        | b'T'
                                        | b'f'
                                ) {
                                    return true;
                                }
                                break;
                            }
                        }
                    }
                    b']' | b'P' | b'^' | b'_' => return true,
                    b'(' | b')' | b'*' | b'+' => {
                        index = (index + 2).min(bytes.len());
                    }
                    _ => return true,
                }
            }
            0x08 => return true,
            byte if byte < 0x20 && !matches!(byte, b'\t' | b'\r' | b'\n') => return true,
            _ => index += 1,
        }
    }
    false
}

fn strip_terminal_control_sequences(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = String::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            0x1b => {
                index += 1;
                if index >= bytes.len() {
                    break;
                }
                match bytes[index] {
                    b'[' => {
                        index += 1;
                        while index < bytes.len() {
                            let byte = bytes[index];
                            index += 1;
                            if (0x40..=0x7e).contains(&byte) {
                                break;
                            }
                        }
                    }
                    b']' | b'P' | b'^' | b'_' => {
                        index += 1;
                        while index < bytes.len() {
                            if bytes[index] == 0x07 {
                                index += 1;
                                break;
                            }
                            if bytes[index] == 0x1b
                                && index + 1 < bytes.len()
                                && bytes[index + 1] == b'\\'
                            {
                                index += 2;
                                break;
                            }
                            index += 1;
                        }
                    }
                    b'(' | b')' | b'*' | b'+' => {
                        index = (index + 2).min(bytes.len());
                    }
                    _ => {
                        index += 1;
                    }
                }
            }
            byte if byte < 0x20 && byte != b'\t' => {
                index += 1;
            }
            _ => {
                let Some(ch) = text[index..].chars().next() else {
                    break;
                };
                output.push(ch);
                index += ch.len_utf8();
            }
        }
    }
    output
}

fn is_incomplete_shell_syntax(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("unexpected eof")
        || stderr.contains("unexpected end of file")
        || stderr.contains("unmatched \"")
        || stderr.contains("unmatched '")
        || stderr.contains("parse error near `\\n'")
        || stderr.contains("parse error near `\n'")
        || stderr.contains("parse error: unmatched")
}

fn shell_continuation_prompt(stderr: &str) -> Option<String> {
    let stderr = stderr.to_ascii_lowercase();
    if stderr.contains("unmatched \"") || stderr.contains("matching `\"'") {
        return Some("dquote> ".to_string());
    }
    if stderr.contains("unmatched '") || stderr.contains("matching `''") {
        return Some("quote> ".to_string());
    }
    if is_incomplete_shell_syntax(&stderr) {
        return Some("> ".to_string());
    }
    None
}

fn ends_with_shell_line_continuation(input: &str) -> bool {
    let trailing_backslashes = input
        .as_bytes()
        .iter()
        .rev()
        .take_while(|&&byte| byte == b'\\')
        .count();
    trailing_backslashes % 2 == 1
}

fn normalize_pty_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn complete_normalized_lines(normalized: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = normalized.lines().collect();
    if !normalized.ends_with('\n') {
        lines.pop();
    }
    lines
}

fn clean_marker_echo(output: &str, marker: &str) -> String {
    output
        .split_inclusive('\n')
        .filter(|line| {
            let text = line.trim_end_matches('\n');
            !(text.contains(READY_MARKER)
                || text.contains(START_MARKER)
                || text.contains("__aish_status=$?") && text.contains(marker))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_configured_shell_before_environment() {
        assert_eq!(resolve_shell("/bin/custom-shell"), "/bin/custom-shell");
    }

    #[test]
    fn shell_command_builder_inherits_current_directory() {
        let cwd = env::current_dir().unwrap();
        let launch = shell_launch("/bin/bash");
        let command = shell_command_builder(&launch);

        assert_eq!(
            command.get_cwd().map(|cwd| cwd.as_os_str()),
            Some(cwd.as_os_str())
        );
    }

    #[test]
    fn spawned_backend_reports_resolved_shell_program() {
        let backend = PtyBackend::spawn("/bin/bash").unwrap();

        assert_eq!(backend.shell_program(), "/bin/bash");
    }

    #[test]
    fn bash_launch_uses_clean_startup_flags() {
        let launch = shell_launch("/bin/bash");
        assert_eq!(launch.program, "/bin/bash");
        assert_eq!(launch.args, ["-i"]);
        assert!(launch.init_command.contains(READY_MARKER));
        assert!(launch.init_command.contains("HISTCONTROL=ignorespace"));
        assert!(launch.init_command.contains("enable-bracketed-paste off"));
        assert!(launch.init_command.contains("__aish_run_prompt_command"));
        assert!(launch.init_command.contains("PROMPT_COMMAND="));
        assert!(launch.init_command.contains("trap - DEBUG"));
    }

    #[test]
    fn non_bash_launch_does_not_receive_bash_only_flags() {
        let launch = shell_launch("/bin/zsh");
        assert_eq!(launch.program, "/bin/zsh");
        assert_eq!(launch.args, ["-i", "-o", "histignorespace"]);
        assert!(launch.init_command.contains("unsetopt zle"));
        assert!(launch.init_command.contains("add-zsh-hook"));
        assert!(launch.init_command.contains("__aish_preexec"));
        assert!(launch.init_command.contains("__aish_precmd"));
    }

    #[test]
    fn fish_launch_uses_event_functions_after_user_config() {
        let launch = shell_launch("/usr/bin/fish");

        assert_eq!(launch.program, "/usr/bin/fish");
        if !launch.args.is_empty() {
            assert_eq!(launch.args, ["--features", "no-query-term,no-mark-prompt"]);
        }
        assert_eq!(launch.integration, ShellIntegration::FishEvents);
        assert!(launch.init_command.contains("--on-event fish_preexec"));
        assert!(launch.init_command.contains("function fish_prompt"));
        assert!(!launch.args.contains(&"--noprofile".to_string()));
        assert!(!launch.args.contains(&"--no-config".to_string()));
    }

    #[test]
    fn parses_marker_and_hides_it_from_output() {
        let marker = "__AISH_STATUS__123__";
        let raw = format!("hello\r\n{marker}7\r\n");
        let (output, status, cwd, started) = parse_marker_output(&raw, marker).unwrap();
        assert_eq!(output, "hello");
        assert_eq!(status, 7);
        assert_eq!(cwd, None);
        assert_eq!(started, None);
    }

    #[test]
    fn parses_marker_cwd_when_present() {
        let marker = "__AISH_STATUS__123__";
        let raw = format!("hello\r\n{marker}7\t/tmp/aish\r\n");
        let (output, status, cwd, _) = parse_marker_output(&raw, marker).unwrap();
        assert_eq!(output, "hello");
        assert_eq!(status, 7);
        assert_eq!(cwd.as_deref(), Some("/tmp/aish"));
    }

    #[test]
    fn parser_ignores_old_fixed_marker_in_user_output() {
        let marker = "__AISH_STATUS__123__";
        let raw = format!("before __AISH_STATUS__ after\r\n{marker}0\r\n");
        let (output, status, _, _) = parse_marker_output(&raw, marker).unwrap();
        assert_eq!(output, "before __AISH_STATUS__ after");
        assert_eq!(status, 0);
    }

    #[test]
    fn parser_normalizes_pty_newlines() {
        let marker = "__AISH_STATUS__123__";
        let raw = format!("one\r\ntwo\r\n{marker}0\r\n");
        let (output, status, _, _) = parse_marker_output(&raw, marker).unwrap();
        assert_eq!(output, "one\ntwo");
        assert_eq!(status, 0);
    }

    #[test]
    fn parser_reads_ready_marker_cwd() {
        let raw = format!("noise\r\n{READY_MARKER}\t/tmp/aish\r\n");
        assert_eq!(parse_ready_cwd(&raw).as_deref(), Some("/tmp/aish"));
        assert_eq!(parse_ready_cwd(READY_MARKER), None);
    }

    #[test]
    fn parser_reads_ready_marker_cwd_when_status_is_present() {
        let raw = format!("noise\r\n{READY_MARKER}\t0\t/tmp/aish\r\n");
        assert_eq!(parse_ready_cwd(&raw).as_deref(), Some("/tmp/aish"));
    }

    #[test]
    fn parser_waits_for_complete_ready_marker_line() {
        let status_only = format!("{READY_MARKER}\t0");
        let partial_cwd = format!("{READY_MARKER}\t0\t/tmp/aish");

        assert_eq!(parse_ready_cwd(&status_only), None);
        assert_eq!(parse_ready_cwd(&partial_cwd), None);
        assert_eq!(
            parse_ready_cwd(&format!("{partial_cwd}\n")).as_deref(),
            Some("/tmp/aish")
        );
    }

    #[test]
    fn parser_strips_terminal_controls_from_ready_marker_cwd() {
        let raw = format!("noise\r\n\x1b[K{READY_MARKER}\t0\t/tmp/aish\x1b[K\r\n");
        assert_eq!(parse_ready_cwd(&raw).as_deref(), Some("/tmp/aish"));
    }

    #[test]
    fn parser_ignores_ready_marker_in_echoed_init_command() {
        let raw = format!(
            "stty -echo; printf '\\n{READY_MARKER}\\t%s\\n' \"$PWD\"\r\n{READY_MARKER}\t/tmp/aish\r\n"
        );
        assert_eq!(parse_ready_cwd(&raw).as_deref(), Some("/tmp/aish"));
    }

    #[test]
    fn parser_uses_real_marker_when_command_echo_contains_marker() {
        let marker = "__AISH_STATUS__123__";
        let raw =
            format!("__aish_status=$?; printf marker {marker}\r\nactual\r\n{marker}0\t/tmp\r\n");
        let (output, status, cwd, _) = parse_marker_output(&raw, marker).unwrap();
        assert_eq!(output, "actual");
        assert_eq!(status, 0);
        assert_eq!(cwd.as_deref(), Some("/tmp"));
    }

    #[test]
    fn parser_reads_start_marker_for_marker_shells() {
        let marker = "__AISH_STATUS__123__";
        let raw = format!("{START_MARKER}\tprintf hello\nhello\n{marker}0\t/tmp\n");
        let (output, status, cwd, started) = parse_marker_output(&raw, marker).unwrap();

        assert_eq!(output, "hello");
        assert_eq!(status, 0);
        assert_eq!(cwd.as_deref(), Some("/tmp"));
        assert_eq!(started.as_deref(), Some("printf hello"));
    }

    #[test]
    fn start_marker_command_quotes_shell_text_and_normalizes_multiline_display() {
        let command = start_marker_command("printf 'a\\n'\necho done");

        assert!(command.starts_with(' '));
        assert!(command.contains(START_MARKER));
        assert!(command.contains("'printf '\\''a\\n'\\''\\necho done'"));
    }

    #[test]
    fn clean_marker_echo_hides_ready_marker_lines() {
        let output = clean_marker_echo(
            &format!("echoed\n{READY_MARKER}\t/tmp/aish\nvisible"),
            "__AISH_STATUS__1__",
        );

        assert_eq!(output, "echoed\nvisible");
    }

    #[test]
    fn output_filter_hides_marker_lines_and_their_separator() {
        let marker = "__AISH_STATUS__123__";
        let mut filter = PtyOutputFilter::marker(marker);

        let output = filter.push(
            format!("\r\n{START_MARKER}\techo hi\r\nhi\r\n\r\n{marker}0\t/tmp\r\n").as_bytes(),
        );

        assert_eq!(String::from_utf8(output).unwrap(), "hi\r\n");
    }

    #[test]
    fn output_filter_suppresses_prompt_noise_after_status_marker() {
        let marker = "__AISH_STATUS__123__";
        let mut filter = PtyOutputFilter::marker(marker);

        let output =
            filter.push(format!("hi\r\n{marker}0\t/tmp\r\nprompt-command-noise\r\n").as_bytes());

        assert_eq!(String::from_utf8(output).unwrap(), "hi\r\n");
    }

    #[test]
    fn output_filter_preserves_carriage_return_progress() {
        let mut filter = PtyOutputFilter::marker("__AISH_STATUS__123__");

        let output = filter.push(b"Counting objects:  50%\rCounting objects: 100%\r\n");

        assert_eq!(
            output,
            b"Counting objects:  50%\rCounting objects: 100%\r\n"
        );
    }

    #[test]
    fn fish_output_filter_streams_only_command_output_between_markers() {
        let mut filter = PtyOutputFilter::shell_events(true);
        let raw = format!(
            "prompt repaint\r\n{START_MARKER}\tprintf 'fish-ok\\n'\r\nfish-ok\r\n{READY_MARKER}\t0\t/tmp/aish\r\nnext prompt\r\n"
        );

        let output = filter.push(raw.as_bytes());

        assert_eq!(String::from_utf8(output).unwrap(), "fish-ok\r\n");
    }

    #[test]
    fn fish_output_filter_drops_cursor_repaint_duplicate_before_plain_output() {
        let mut filter = PtyOutputFilter::shell_events(true);
        let raw = format!(
            "{START_MARKER}\tcat c/i | grep beta\r\n\x1b[50Cbeta\r\nbeta\r\n{READY_MARKER}\t0\t/tmp/aish\r\n"
        );

        let output = filter.push(raw.as_bytes());

        assert_eq!(String::from_utf8(output).unwrap(), "beta\r\n");
    }

    #[test]
    fn fish_output_filter_preserves_carriage_return_progress_inside_command() {
        let mut filter = PtyOutputFilter::shell_events(true);
        let raw = format!(
            "{START_MARKER}\tprintf progress\r\nprogress 1\rprogress 2\r\n{READY_MARKER}\t0\t/tmp/aish\r\n"
        );

        let output = filter.push(raw.as_bytes());

        assert_eq!(output, b"progress 1\rprogress 2\r\n");
    }

    #[test]
    fn parse_ready_status_output_reads_status_cwd_and_filters_hook_lines() {
        let raw = format!("{START_MARKER}\techo hello\nhello\n{READY_MARKER}\t7\t/tmp/aish\n");

        assert_eq!(
            parse_ready_status_output(&raw, false).unwrap(),
            HookCommandResult {
                output: "hello\n".to_string(),
                exit_code: 7,
                cwd: "/tmp/aish".to_string(),
                started_command: Some("echo hello".to_string()),
            }
        );
    }

    #[test]
    fn parse_ready_status_output_preserves_user_output_line_breaks() {
        let raw = format!(
            "{START_MARKER}\tprintf first\\nsecond\\n\nfirst\nsecond\n{READY_MARKER}\t0\t/tmp/aish\n"
        );

        let parsed = parse_ready_status_output(&raw, false).unwrap();

        assert_eq!(parsed.output, "first\nsecond\n");
    }

    #[test]
    fn parse_ready_status_output_ignores_prompt_noise_around_command_markers() {
        let raw = format!(
            "old prompt\n\
             {READY_MARKER}\t0\n\
             {START_MARKER}\tprintf hi\n\
             hi\n\
             {READY_MARKER}\t0\t/tmp/aish\n\
             user precmd noise\n\
             prompt> \n"
        );

        let parsed = parse_ready_status_output(&raw, false).unwrap();

        assert_eq!(parsed.output, "hi\n");
        assert_eq!(parsed.cwd, "/tmp/aish");
        assert_eq!(parsed.started_command.as_deref(), Some("printf hi"));
    }

    #[test]
    fn parse_ready_status_output_can_filter_fish_repaint_sequences() {
        let raw = format!(
            "{START_MARKER}\tprintf 'fish-ok\\n'\n\
             printf \n\
             \x1b[50C\x1b[?2004l\x1b[?2031l\x1b[>4;0m\x1b>'fish-ok\\n'\n\
             \x1b[61C\x1b[18Dprintf 'fish-ok\\n'\n\
             \x1b[61C\n\
             \x1b[m\n\
             \x1b]0;printf 'fish-ok\\n' ~/aish\x07\x1b[m\n\
             fish-ok\n\
             \x1b[?25h\x1b[2m\u{23ce}\x1b[m\n\
             \u{23ce} \n\
             \x1b[K\x1b]0;~/aish\x07\x1b[m\x1b[?2004h\x1b[?2031h\x1b[>4;1m\x1b=\x1b[K\n\
             \x1b[43C\n\
             {READY_MARKER}\t0\t/tmp/aish\n"
        );

        let parsed = parse_ready_status_output(&raw, true).unwrap();

        assert_eq!(parsed.output, "fish-ok\n");
        assert_eq!(
            parsed.started_command.as_deref(),
            Some("printf 'fish-ok\\n'")
        );
    }

    #[test]
    fn fish_repaint_filter_preserves_plain_output_matching_command_suffix() {
        let raw = format!(
            "{START_MARKER}\tcat common/items.txt | grep beta\n\
             \x1b[50Ccommon/items.txt\n\
             \x1b[50C|\n\
             \x1b[50Cgrep\n\
             \x1b[50Cbeta\n\
             beta\n\
             {READY_MARKER}\t0\t/tmp/aish\n"
        );

        let parsed = parse_ready_status_output(&raw, true).unwrap();

        assert_eq!(parsed.output, "beta\n");
    }

    #[test]
    fn fish_repaint_filter_removes_semicolon_command_fragments() {
        let raw = format!(
            "{START_MARKER}\ttest -f c/i; and echo file-exists\n\
             c/i;\n\
             file-exists\n\
             {READY_MARKER}\t0\t/tmp/aish\n"
        );

        let parsed = parse_ready_status_output(&raw, true).unwrap();

        assert_eq!(parsed.output, "file-exists\n");
    }

    #[test]
    fn fish_repaint_filter_removes_variable_command_fragments() {
        let raw = format!(
            "{START_MARKER}\tprintf '%s\\n' $AISH_FISH_RC_ENV\n\
             $AISH_FISH_RC_ENV\n\
             env-from-fish-config\n\
             {READY_MARKER}\t0\t/tmp/aish\n"
        );

        let parsed = parse_ready_status_output(&raw, true).unwrap();

        assert_eq!(parsed.output, "env-from-fish-config\n");
    }

    #[test]
    fn incomplete_shell_syntax_detection_uses_shell_error_text() {
        assert!(is_incomplete_shell_syntax(
            "bash: unexpected EOF while looking for matching `\"'"
        ));
        assert!(is_incomplete_shell_syntax("zsh: parse error: unmatched \""));
        assert!(!is_incomplete_shell_syntax(
            "syntax error near unexpected token `fi'"
        ));
    }

    #[test]
    fn line_continuation_detects_odd_trailing_backslashes() {
        assert!(ends_with_shell_line_continuation("echo aa \\"));
        assert!(!ends_with_shell_line_continuation("echo aa \\\\"));
        assert!(!ends_with_shell_line_continuation("echo aa"));
    }

    #[test]
    fn bash_syntax_check_detects_incomplete_input_without_hanging() {
        let backend = PtyBackend::spawn("/bin/bash").unwrap();

        let continued = backend.input_needs_more_lines("echo aa \\").unwrap();
        assert!(continued.needs_more);
        assert_eq!(continued.prompt.as_deref(), Some("> "));

        let unclosed = backend.input_needs_more_lines("echo \"").unwrap();
        assert!(unclosed.needs_more);
        assert_eq!(unclosed.prompt.as_deref(), Some("dquote> "));

        let single = backend.input_needs_more_lines("echo '").unwrap();
        assert!(single.needs_more);
        assert_eq!(single.prompt.as_deref(), Some("quote> "));

        let complete = backend.input_needs_more_lines("echo \"ok\"").unwrap();
        assert!(!complete.needs_more);
        assert!(complete.prompt.is_none());
    }

    #[test]
    fn zsh_syntax_check_detects_incomplete_input_without_hanging() {
        if !Path::new("/bin/zsh").exists() {
            return;
        }

        let backend = PtyBackend::spawn("/bin/zsh").unwrap();

        let unclosed = backend.input_needs_more_lines("echo \"").unwrap();
        assert!(unclosed.needs_more);
        assert_eq!(unclosed.prompt.as_deref(), Some("dquote> "));

        let complete = backend.input_needs_more_lines("echo \"ok\"").unwrap();
        assert!(!complete.needs_more);
    }

    #[test]
    fn marker_status_requires_digits_and_line_end() {
        let marker = "__AISH_STATUS__123__";
        assert!(!marker_status_is_complete("hello", marker));
        assert!(!marker_status_is_complete(marker, marker));
        assert!(!marker_status_is_complete("__AISH_STATUS__123__", marker));
        assert!(!marker_status_is_complete(
            "__AISH_STATUS__123__x\n",
            marker
        ));
        assert!(marker_status_is_complete(
            "hello\r\n__AISH_STATUS__123__0\r\n",
            marker
        ));
    }
}
