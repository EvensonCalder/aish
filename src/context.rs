use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};

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
    _timeout: Duration,
) -> Result<ContextCommandResult> {
    let mut process = Command::new("/bin/sh");
    process
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        process.current_dir(cwd);
    }

    let output = process
        .output()
        .with_context(|| format!("failed to run context command: {command}"))?;
    let combined = combine_stdout_stderr(&output.stdout, &output.stderr);
    let (captured_output, truncated) = cap_utf8(&combined, max_bytes);
    Ok(ContextCommandResult {
        output: captured_output,
        exit_code: output_status_code(output.status),
        truncated,
    })
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
        result.output
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
}
