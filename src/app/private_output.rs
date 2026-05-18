use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};

use crate::config::set_private_file_permissions;
use crate::log::EventLevel;
use crate::modes::Mode;

use super::AppState;
use super::execution::foreground_shell_args;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPrivateOutput {
    pub label: String,
    pub output: String,
    pub sink: PrivateOutputSink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrivateOutputSink {
    File { path: PathBuf, append: bool },
    Pipe { command: String },
}

impl PrivateOutputSink {
    fn description(&self) -> String {
        match self {
            Self::File { path, append } => {
                let op = if *append { "append to" } else { "write to" };
                format!("{op} {}", path.display())
            }
            Self::Pipe { command } => format!("pipe command: {command}"),
        }
    }
}

pub(super) fn list_output_from_commands<'a>(commands: impl IntoIterator<Item = &'a str>) -> String {
    let mut output = String::new();
    for command in commands {
        output.push_str(&one_line_command(command));
        output.push('\n');
    }
    output
}

fn one_line_command(command: &str) -> String {
    command.replace('\r', "\\r").replace('\n', "\\n")
}

pub(super) fn parse_list_output_sink(
    rest: &str,
    cwd: Option<&Path>,
) -> std::result::Result<Option<PrivateOutputSink>, String> {
    let rest = rest.trim();
    if rest.is_empty() {
        return Ok(None);
    }
    let Some(operator) = find_output_operator(rest)? else {
        return Err("unexpected list arguments".to_string());
    };
    if !rest[..operator.index].trim().is_empty() {
        return Err("unexpected list arguments before output operator".to_string());
    }
    let target = rest[operator.index + operator.len..].trim();
    if target.is_empty() {
        return Err("missing output target".to_string());
    }
    match operator.kind {
        OutputOperatorKind::Pipe => Ok(Some(PrivateOutputSink::Pipe {
            command: target.to_string(),
        })),
        OutputOperatorKind::Write | OutputOperatorKind::Append => {
            let words = split_shell_words(target)?;
            if words.len() != 1 {
                return Err("redirection target must be one path".to_string());
            }
            Ok(Some(PrivateOutputSink::File {
                path: resolve_output_path(&words[0], cwd),
                append: operator.kind == OutputOperatorKind::Append,
            }))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OutputOperator {
    kind: OutputOperatorKind,
    index: usize,
    len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputOperatorKind {
    Pipe,
    Write,
    Append,
}

fn find_output_operator(input: &str) -> std::result::Result<Option<OutputOperator>, String> {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
        match quote {
            Some('\'') if ch == '\'' => quote = None,
            Some('"') if escaped => escaped = false,
            Some('"') if ch == '\\' => escaped = true,
            Some('"') if ch == '"' => quote = None,
            Some(_) => {}
            None if escaped => escaped = false,
            None if ch == '\\' => escaped = true,
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch == '|' => {
                return Ok(Some(OutputOperator {
                    kind: OutputOperatorKind::Pipe,
                    index,
                    len: 1,
                }));
            }
            None if ch == '>' => {
                let append = input[index + ch.len_utf8()..].starts_with('>');
                return Ok(Some(OutputOperator {
                    kind: if append {
                        OutputOperatorKind::Append
                    } else {
                        OutputOperatorKind::Write
                    },
                    index,
                    len: if append { 2 } else { 1 },
                }));
            }
            None => {}
        }
    }
    if quote.is_some() {
        return Err("unterminated quote".to_string());
    }
    Ok(None)
}

fn split_shell_words(input: &str) -> std::result::Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut token_started = false;

    for ch in input.chars() {
        match quote {
            Some('\'') if ch == '\'' => quote = None,
            Some('\'') => {
                token_started = true;
                current.push(ch);
            }
            Some('"') if escaped => {
                escaped = false;
                token_started = true;
                current.push(ch);
            }
            Some('"') if ch == '\\' => escaped = true,
            Some('"') if ch == '"' => quote = None,
            Some('"') => {
                token_started = true;
                current.push(ch);
            }
            None if escaped => {
                escaped = false;
                token_started = true;
                current.push(ch);
            }
            None if ch == '\\' => {
                escaped = true;
                token_started = true;
            }
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                token_started = true;
            }
            None if ch.is_whitespace() => {
                if token_started {
                    words.push(std::mem::take(&mut current));
                    token_started = false;
                }
            }
            None => {
                token_started = true;
                current.push(ch);
            }
            Some(_) => unreachable!("only single and double quotes are tracked"),
        }
    }

    if quote.is_some() {
        return Err("unterminated quote".to_string());
    }
    if escaped {
        current.push('\\');
    }
    if token_started {
        words.push(current);
    }
    Ok(words)
}

fn resolve_output_path(value: &str, cwd: Option<&Path>) -> PathBuf {
    let path = expand_home_path(value);
    if path.is_absolute() {
        return path;
    }
    cwd.map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
        .join(path)
}

fn expand_home_path(value: &str) -> PathBuf {
    if value == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    } else if let Some(rest) = value.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(value)
}

pub(super) fn write_or_confirm_private_output(
    state: &mut AppState,
    out: &mut impl Write,
    label: &str,
    output: String,
    sink: Option<PrivateOutputSink>,
) -> Result<()> {
    let Some(sink) = sink else {
        out.write_all(output.as_bytes())?;
        return Ok(());
    };

    let line_count = output_line_count(&output);
    writeln!(
        out,
        "aish will export {line_count} {label} line(s) to {}.",
        sink.description()
    )?;
    writeln!(
        out,
        "This output may contain private history, AI output, drafts, templates, or secrets."
    )?;
    writeln!(out, "Export list output? [Y/n]")?;
    writeln!(out, "answer Y to export or n to skip")?;
    state.pending_private_output = Some(PendingPrivateOutput {
        label: label.to_string(),
        output,
        sink,
    });
    state.mode = Mode::Draft;
    Ok(())
}

pub fn answer_private_output_confirmation(
    state: &mut AppState,
    accepted: bool,
    out: &mut impl Write,
    timeout: Duration,
) -> Result<()> {
    let Some(pending) = state.pending_private_output.take() else {
        return Ok(());
    };
    state.mode = Mode::Draft;
    if !accepted {
        writeln!(out, "private output export skipped")?;
        state.append_event(EventLevel::Info, "private output export skipped")?;
        return Ok(());
    }

    let line_count = output_line_count(&pending.output);
    match pending.sink {
        PrivateOutputSink::File { path, append } => {
            write_private_output_file(&path, append, pending.output.as_bytes())?;
            let verb = if append { "appended" } else { "exported" };
            writeln!(
                out,
                "{verb} {line_count} {} line(s) to {}",
                pending.label,
                path.display()
            )?;
        }
        PrivateOutputSink::Pipe { command } => {
            let result = run_pipe_command(
                state.backend_shell.as_deref(),
                &command,
                pending.output.as_bytes(),
                state.current_cwd.as_deref(),
                timeout,
                state.context_config.max_bytes,
            )?;
            out.write_all(result.output.as_bytes())?;
            if result.truncated {
                writeln!(
                    out,
                    "pipe command output truncated to {} bytes",
                    state.context_config.max_bytes
                )?;
            }
            if result.timed_out {
                writeln!(
                    out,
                    "pipe command timed out after {} ms",
                    timeout.as_millis()
                )?;
            }
            if result.exit_code != Some(0) {
                let status = result
                    .exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                writeln!(out, "pipe command exited with {status}")?;
            }
        }
    }
    state.append_event(EventLevel::Info, "private output exported")?;
    Ok(())
}

fn output_line_count(output: &str) -> usize {
    output
        .as_bytes()
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
}

fn write_private_output_file(path: &Path, append: bool, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create(true).write(true);
    if append {
        options.append(true);
    } else {
        options.truncate(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("failed to open private output file {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write private output file {}", path.display()))?;
    set_private_file_permissions(path)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PipeCommandResult {
    output: String,
    exit_code: Option<i32>,
    truncated: bool,
    timed_out: bool,
}

fn run_pipe_command(
    shell: Option<&str>,
    command: &str,
    input: &[u8],
    cwd: Option<&Path>,
    timeout: Duration,
    max_output_bytes: usize,
) -> Result<PipeCommandResult> {
    let shell = shell
        .map(str::trim)
        .filter(|shell| !shell.is_empty())
        .unwrap_or(default_shell());
    let args = output_shell_args(shell, command);
    let mut process = Command::new(shell);
    process
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_pipe_process(&mut process);
    if let Some(cwd) = cwd {
        process.current_dir(cwd);
    }

    let mut child = process
        .spawn()
        .with_context(|| format!("failed to run pipe command: {command}"))?;
    let mut stdin = child.stdin.take().context("pipe command stdin missing")?;
    let input = input.to_vec();
    let stdin_writer = thread::spawn(move || stdin.write_all(&input));
    let stdout = child.stdout.take().context("pipe command stdout missing")?;
    let stderr = child.stderr.take().context("pipe command stderr missing")?;
    let stdout_reader = thread::spawn(move || read_stream_capped(stdout, max_output_bytes));
    let stderr_reader = thread::spawn(move || read_stream_capped(stderr, max_output_bytes));

    let (status, timed_out) = wait_child_with_timeout(&mut child, timeout)
        .with_context(|| format!("failed to wait for pipe command: {command}"))?;
    match stdin_writer
        .join()
        .map_err(|_| anyhow!("pipe command stdin writer panicked"))?
    {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {}
        Err(error) => return Err(error).context("failed to write pipe command stdin"),
    }
    let stdout = join_stream_reader(stdout_reader, "stdout")?;
    let stderr = join_stream_reader(stderr_reader, "stderr")?;
    let combined = combine_stdout_stderr(&stdout.bytes, &stderr.bytes);
    let (output, combined_truncated) = cap_utf8(&combined, max_output_bytes);
    Ok(PipeCommandResult {
        output,
        exit_code: status.and_then(output_status_code),
        truncated: stdout.truncated || stderr.truncated || combined_truncated,
        timed_out,
    })
}

#[cfg(windows)]
fn default_shell() -> &'static str {
    "cmd"
}

#[cfg(not(windows))]
fn default_shell() -> &'static str {
    "/bin/sh"
}

#[cfg(windows)]
fn output_shell_args(shell: &str, command: &str) -> Vec<String> {
    let shell_name = Path::new(shell.trim())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .trim_start_matches('-')
        .to_ascii_lowercase();
    if matches!(shell_name.as_str(), "cmd" | "cmd.exe") {
        vec!["/C".to_string(), command.to_string()]
    } else {
        foreground_shell_args(shell, command)
    }
}

#[cfg(not(windows))]
fn output_shell_args(shell: &str, command: &str) -> Vec<String> {
    foreground_shell_args(shell, command)
}

#[cfg(unix)]
fn configure_pipe_process(process: &mut Command) {
    use std::os::unix::process::CommandExt;

    process.process_group(0);
}

#[cfg(not(unix))]
fn configure_pipe_process(_process: &mut Command) {}

#[derive(Debug)]
struct CappedStream {
    bytes: Vec<u8>,
    truncated: bool,
}

fn read_stream_capped(mut reader: impl Read, limit: usize) -> io::Result<CappedStream> {
    let mut bytes = Vec::with_capacity(limit.min(8192));
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(bytes.len());
        let kept = remaining.min(read);
        if kept > 0 {
            bytes.extend_from_slice(&buffer[..kept]);
        }
        if kept < read {
            truncated = true;
        }
    }
    Ok(CappedStream { bytes, truncated })
}

fn join_stream_reader(
    reader: thread::JoinHandle<io::Result<CappedStream>>,
    name: &str,
) -> Result<CappedStream> {
    reader
        .join()
        .map_err(|_| anyhow!("pipe command {name} reader panicked"))?
        .with_context(|| format!("failed to read pipe command {name}"))
}

fn wait_child_with_timeout(
    child: &mut Child,
    timeout: Duration,
) -> io::Result<(Option<ExitStatus>, bool)> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((Some(status), false));
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            let status = terminate_pipe_child(child)?;
            return Ok((Some(status), true));
        }
        let remaining = timeout.saturating_sub(elapsed);
        thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

#[cfg(unix)]
fn terminate_pipe_child(child: &mut Child) -> io::Result<ExitStatus> {
    let pgid = child.id() as libc::pid_t;
    unsafe {
        libc::kill(-pgid, libc::SIGTERM);
    }
    for _ in 0..5 {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(10));
    }
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
    let _ = child.kill();
    child.wait()
}

#[cfg(not(unix))]
fn terminate_pipe_child(child: &mut Child) -> io::Result<ExitStatus> {
    let _ = child.kill();
    child.wait()
}

fn combine_stdout_stderr(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut combined = Vec::new();
    combined.extend_from_slice(stdout);
    if !stderr.is_empty() {
        if !combined.is_empty() && !combined.ends_with(b"\n") {
            combined.push(b'\n');
        }
        combined.extend_from_slice(b"[stderr]\n");
        combined.extend_from_slice(stderr);
    }
    combined
}

fn cap_utf8(bytes: &[u8], max_bytes: usize) -> (String, bool) {
    if bytes.len() <= max_bytes {
        return (String::from_utf8_lossy(bytes).to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
        end -= 1;
    }
    (String::from_utf8_lossy(&bytes[..end]).to_string(), true)
}

fn output_status_code(status: ExitStatus) -> Option<i32> {
    status.code()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list_redirection_without_shell_expansion() {
        let cwd = Path::new("/tmp/aish-cwd");
        assert_eq!(
            parse_list_output_sink("> 'my file.txt'", Some(cwd)).unwrap(),
            Some(PrivateOutputSink::File {
                path: cwd.join("my file.txt"),
                append: false,
            })
        );
        assert_eq!(
            parse_list_output_sink(">> out.txt", Some(cwd)).unwrap(),
            Some(PrivateOutputSink::File {
                path: cwd.join("out.txt"),
                append: true,
            })
        );
        assert_eq!(
            parse_list_output_sink("| wc -l", Some(cwd)).unwrap(),
            Some(PrivateOutputSink::Pipe {
                command: "wc -l".to_string(),
            })
        );
    }

    #[test]
    fn rejects_extra_list_arguments_before_sink() {
        assert!(parse_list_output_sink("extra > out", None).is_err());
        assert!(parse_list_output_sink("> out extra", None).is_err());
        assert!(parse_list_output_sink("|   ", None).is_err());
    }

    #[test]
    fn list_output_escapes_multiline_commands_as_one_line() {
        assert_eq!(
            list_output_from_commands(["printf 'one\n'", "echo two"]),
            "printf 'one\\n'\necho two\n"
        );
    }
}
