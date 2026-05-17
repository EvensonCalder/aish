use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};

use crate::log::redact_secrets;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCommandResult {
    pub output: String,
    pub exit_code: Option<i32>,
    pub truncated: bool,
}

pub fn is_dangerous_context_command(command: &str) -> bool {
    let command = command.trim().to_ascii_lowercase();
    if command.is_empty() {
        return false;
    }

    [
        "rm -rf",
        "rm -fr",
        "sudo ",
        "doas ",
        "mkfs",
        "dd if=",
        "diskutil erase",
        "shutdown",
        "reboot",
        ":(){",
    ]
    .iter()
    .any(|pattern| command.contains(pattern))
}

pub fn run_context_command(
    command: &str,
    cwd: Option<&Path>,
    max_bytes: usize,
    timeout: Duration,
) -> Result<ContextCommandResult> {
    let mut process = Command::new("/bin/sh");
    process
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_context_process(&mut process);
    if let Some(cwd) = cwd {
        process.current_dir(cwd);
    }

    let mut child = process
        .spawn()
        .with_context(|| format!("failed to run context command: {command}"))?;
    let stdout = child.stdout.take().context("context stdout pipe missing")?;
    let stderr = child.stderr.take().context("context stderr pipe missing")?;
    let stdout_reader = thread::spawn(move || read_stream_capped(stdout, max_bytes));
    let stderr_reader = thread::spawn(move || read_stream_capped(stderr, max_bytes));

    let (status, timed_out) = wait_child_with_timeout(&mut child, timeout)
        .with_context(|| format!("failed to wait for context command: {command}"))?;
    let stdout = join_stream_reader(stdout_reader, "stdout")?;
    let mut stderr = join_stream_reader(stderr_reader, "stderr")?;
    if timed_out {
        if !stderr.bytes.is_empty() && !stderr.bytes.ends_with(b"\n") {
            stderr.bytes.push(b'\n');
        }
        stderr.bytes.extend_from_slice(
            format!(
                "context command timed out after {} ms\n",
                timeout.as_millis()
            )
            .as_bytes(),
        );
    }

    let combined = combine_stdout_stderr(&stdout.bytes, &stderr.bytes);
    let (captured_output, cap_truncated) = cap_utf8(&combined, max_bytes);
    Ok(ContextCommandResult {
        output: captured_output,
        exit_code: status.and_then(output_status_code),
        truncated: stdout.truncated || stderr.truncated || cap_truncated,
    })
}

#[cfg(unix)]
fn configure_context_process(process: &mut Command) {
    use std::os::unix::process::CommandExt;

    process.process_group(0);
}

#[cfg(not(unix))]
fn configure_context_process(_process: &mut Command) {}

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
        .map_err(|_| anyhow!("context {name} reader panicked"))?
        .with_context(|| format!("failed to read context command {name}"))
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
            let status = terminate_context_child(child)?;
            return Ok((Some(status), true));
        }
        let remaining = timeout.saturating_sub(elapsed);
        thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

#[cfg(unix)]
fn terminate_context_child(child: &mut Child) -> io::Result<ExitStatus> {
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
fn terminate_context_child(child: &mut Child) -> io::Result<ExitStatus> {
    let _ = child.kill();
    child.wait()
}

pub fn build_contextual_ai_prompt(
    prompt: &str,
    command: &str,
    result: &ContextCommandResult,
) -> String {
    let truncation = if result.truncated {
        " truncated to configured byte limit"
    } else {
        ""
    };
    let exit_code = result
        .exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "User prompt:\n{prompt}\n\nContext command:\n{command}\n\nContext exit status: {exit_code}\n\nContext output{truncation}:\n```text\n{}\n```",
        redact_secrets(&result.output),
        command = redact_secrets(command)
    )
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

fn output_status_code(status: std::process::ExitStatus) -> Option<i32> {
    status.code()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_context_command_detection_catches_destructive_patterns() {
        assert!(is_dangerous_context_command("rm -rf /tmp/example"));
        assert!(is_dangerous_context_command("sudo cat /etc/passwd"));
        assert!(is_dangerous_context_command(
            "diskutil eraseDisk FAT32 X /dev/disk9"
        ));
        assert!(!is_dangerous_context_command("git status --short"));
    }

    #[test]
    fn run_context_command_captures_stdout_and_stderr() {
        let result = run_context_command(
            "printf out; printf err >&2",
            None,
            1024,
            Duration::from_secs(5),
        )
        .unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert!(result.output.contains("out"));
        assert!(result.output.contains("[stderr]"));
        assert!(result.output.contains("err"));
        assert!(!result.truncated);
    }

    #[test]
    fn run_context_command_caps_output() {
        let result =
            run_context_command("printf 123456789", None, 4, Duration::from_secs(5)).unwrap();

        assert_eq!(result.output, "1234");
        assert!(result.truncated);
    }

    #[test]
    fn run_context_command_enforces_timeout() {
        let started = Instant::now();
        let result = run_context_command(
            "printf start; sleep 5; printf done",
            None,
            1024,
            Duration::from_millis(50),
        )
        .unwrap();

        assert!(
            started.elapsed() < Duration::from_secs(1),
            "context command ignored timeout"
        );
        assert_eq!(result.exit_code, None);
        assert!(result.output.contains("start"));
        assert!(!result.output.contains("done"));
        assert!(result.output.contains("context command timed out"));
    }

    #[test]
    fn contextual_ai_prompt_discloses_truncation() {
        let prompt = build_contextual_ai_prompt(
            "explain this",
            "printf hello",
            &ContextCommandResult {
                output: "hell".to_string(),
                exit_code: Some(0),
                truncated: true,
            },
        );

        assert!(prompt.contains("User prompt:\nexplain this"));
        assert!(prompt.contains("Context command:\nprintf hello"));
        assert!(prompt.contains("Context output truncated to configured byte limit"));
    }

    #[test]
    fn contextual_ai_prompt_redacts_common_secret_shapes() {
        let prompt = build_contextual_ai_prompt(
            "explain this",
            "printf sk-command-secret",
            &ContextCommandResult {
                output: "token sk-output-secret ghp_output".to_string(),
                exit_code: Some(0),
                truncated: false,
            },
        );

        assert!(prompt.contains("Context command:\nprintf [redacted]"));
        assert!(prompt.contains("token [redacted] [redacted]"));
        assert!(!prompt.contains("sk-command-secret"));
        assert!(!prompt.contains("sk-output-secret"));
        assert!(!prompt.contains("ghp_output"));
    }
}
