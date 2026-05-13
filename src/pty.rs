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
        let raw = self.read_until_ready(Duration::from_secs(5))?;
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
        if self.integration == ShellIntegration::ZshHooks {
            if command.contains('\n') {
                return self.run_command_with_marker(command, timeout);
            }
            return self.run_command_with_zsh_hooks(command, timeout);
        }

        self.run_command_with_marker(command, timeout)
    }

    fn run_command_with_marker(
        &mut self,
        command: &str,
        timeout: Duration,
    ) -> Result<CommandResult> {
        let _ = self.drain_for(Duration::from_millis(25));
        let marker = next_marker();
        let start_command = start_marker_command(command);
        let marker_command = format!(
            " __aish_status=$?; printf '\\n%s%s\\t%s\\n' '{marker}' \"$__aish_status\" \"$PWD\"; sh -c \"exit $__aish_status\"\n"
        );
        self.write_raw(&start_command)?;
        self.write_raw(command)?;
        if !command.ends_with('\n') {
            self.write_raw("\n")?;
        }
        self.write_raw(&marker_command)?;

        let raw = self.read_until_marker(&marker, timeout)?;
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

    fn read_until_marker(&mut self, marker: &str, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        let mut data = Vec::new();
        loop {
            let current = String::from_utf8_lossy(&data);
            if marker_status_is_complete(&current, marker) {
                return Ok(String::from_utf8_lossy(&data).into_owned());
            }
            let now = Instant::now();
            if now >= deadline {
                bail!("timed out waiting for backend shell prompt marker");
            }
            let remaining = deadline
                .saturating_duration_since(now)
                .min(Duration::from_millis(50));
            match self.output.recv_timeout(remaining) {
                Ok(chunk) => data.extend(chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => bail!("backend shell PTY closed"),
            }
        }
    }

    fn run_command_with_zsh_hooks(
        &mut self,
        command: &str,
        timeout: Duration,
    ) -> Result<CommandResult> {
        let _ = self.drain_for(Duration::from_millis(25));
        self.write_raw(command)?;
        if !command.ends_with('\n') {
            self.write_raw("\n")?;
        }

        let raw = self.read_until_ready(timeout)?;
        let parsed = parse_ready_status_output(&raw)?;
        Ok(CommandResult {
            command: command.trim_end_matches('\n').to_string(),
            started_command: parsed.started_command,
            output: parsed.output,
            exit_code: parsed.exit_code,
            cwd: Some(parsed.cwd),
        })
    }

    fn read_until_ready(&mut self, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        let mut data = Vec::new();
        loop {
            if parse_ready_cwd(&String::from_utf8_lossy(&data)).is_some() {
                return Ok(String::from_utf8_lossy(&data).into_owned());
            }
            let now = Instant::now();
            if now >= deadline {
                bail!("timed out waiting for backend shell ready marker");
            }
            let remaining = deadline
                .saturating_duration_since(now)
                .min(Duration::from_millis(50));
            match self.output.recv_timeout(remaining) {
                Ok(chunk) => data.extend(chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => bail!("backend shell PTY closed"),
            }
        }
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
            vec!["--noprofile".to_string(), "--norc".to_string()],
            format!(
                "export HISTCONTROL=ignorespace${{HISTCONTROL:+:$HISTCONTROL}}; PS1=''; PS2=''; stty -echo; printf '\\n{READY_MARKER}\\t%s\\n' \"$PWD\"\n"
            ),
            ShellIntegration::MarkerCommand,
        ),
        "zsh" => (
            vec![
                "-f".to_string(),
                "-o".to_string(),
                "histignorespace".to_string(),
            ],
            format!(
                " stty -echo; unsetopt zle prompt_cr prompt_sp; PROMPT=''; RPROMPT=''; PROMPT2=''; preexec() {{ printf '\\n{START_MARKER}\\t%s\\n' \"$1\"; }}; precmd() {{ printf '\\n{READY_MARKER}\\t%s\\t%s\\n' \"$?\" \"$PWD\"; }}; precmd\n"
            ),
            ShellIntegration::ZshHooks,
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

fn shell_command_builder(launch: &ShellLaunch) -> CommandBuilder {
    let mut command = CommandBuilder::new(&launch.program);
    for arg in &launch.args {
        command.arg(arg);
    }
    if let Ok(cwd) = env::current_dir() {
        command.cwd(cwd);
    }
    command.env("PS1", "");
    command.env("PROMPT", "");
    command.env("RPROMPT", "");
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
    normalize_pty_newlines(raw)
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|rest| {
            let mut parts = rest.split('\t');
            let first = parts.next()?;
            Some(parts.next().unwrap_or(first).to_string())
        })
        .filter(|cwd| !cwd.is_empty())
}

fn parse_ready_status_output(raw: &str) -> Result<HookCommandResult> {
    let raw = normalize_pty_newlines(raw);
    let mut ready_line = None;
    let mut started_command = None;
    let mut output_lines = Vec::new();

    for line in raw.lines() {
        if let Some(command) = line.strip_prefix(&format!("{START_MARKER}\t")) {
            started_command = Some(command.to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix(&format!("{READY_MARKER}\t")) {
            ready_line = Some(rest.to_string());
            continue;
        }
        output_lines.push(line);
    }

    let ready = ready_line.context("backend shell output did not contain ready marker")?;
    let (status, cwd) = ready
        .split_once('\t')
        .context("backend shell ready marker did not include cwd")?;
    let exit_code = status
        .parse::<i32>()
        .context("invalid shell exit status in ready marker")?;
    Ok(HookCommandResult {
        output: output_lines
            .join("\n")
            .trim_matches(['\r', '\n'])
            .to_string(),
        exit_code,
        cwd: cwd.to_string(),
        started_command,
    })
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
        assert_eq!(launch.args, ["--noprofile", "--norc"]);
        assert!(launch.init_command.contains(READY_MARKER));
        assert!(launch.init_command.contains("HISTCONTROL=ignorespace"));
    }

    #[test]
    fn non_bash_launch_does_not_receive_bash_only_flags() {
        let launch = shell_launch("/bin/zsh");
        assert_eq!(launch.program, "/bin/zsh");
        assert_eq!(launch.args, ["-f", "-o", "histignorespace"]);
        assert!(launch.init_command.contains("unsetopt zle"));
        assert!(launch.init_command.contains("preexec()"));
        assert!(launch.init_command.contains("precmd()"));
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
    fn parse_ready_status_output_reads_status_cwd_and_filters_hook_lines() {
        let raw = format!("hello\n{START_MARKER}\techo hello\n{READY_MARKER}\t7\t/tmp/aish\n");

        assert_eq!(
            parse_ready_status_output(&raw).unwrap(),
            HookCommandResult {
                output: "hello".to_string(),
                exit_code: 7,
                cwd: "/tmp/aish".to_string(),
                started_command: Some("echo hello".to_string()),
            }
        );
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
