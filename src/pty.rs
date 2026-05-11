use std::env;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

const MARKER_PREFIX: &str = "__AISH_STATUS__";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub command: String,
    pub output: String,
    pub exit_code: i32,
}

pub struct PtyBackend {
    writer: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtyBackend {
    pub fn spawn(configured_shell: &str) -> Result<Self> {
        let launch = shell_launch(configured_shell);
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to create PTY")?;

        let mut command = CommandBuilder::new(&launch.program);
        for arg in &launch.args {
            command.arg(arg);
        }
        command.env("PS1", "");

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
            writer,
            output: rx,
            _child: child,
        };
        backend.write_raw("stty -echo\n")?;
        let _ = backend.drain_for(Duration::from_millis(150));
        Ok(backend)
    }

    pub fn write_raw(&mut self, text: &str) -> Result<()> {
        self.writer
            .write_all(text.as_bytes())
            .context("failed to write to PTY")?;
        self.writer.flush().context("failed to flush PTY")?;
        Ok(())
    }

    pub fn run_command(&mut self, command: &str, timeout: Duration) -> Result<CommandResult> {
        let marker_command = format!("printf '\\n{MARKER_PREFIX}%s\\n' $?\n");
        self.write_raw(command)?;
        if !command.ends_with('\n') {
            self.write_raw("\n")?;
        }
        self.write_raw(&marker_command)?;

        let raw = self.read_until_marker(timeout)?;
        let (output, exit_code) = parse_marker_output(&raw)?;
        Ok(CommandResult {
            command: command.trim_end_matches('\n').to_string(),
            output,
            exit_code,
        })
    }

    fn read_until_marker(&mut self, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        let mut data = Vec::new();
        loop {
            if String::from_utf8_lossy(&data).contains(MARKER_PREFIX) {
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
}

fn shell_launch(configured_shell: &str) -> ShellLaunch {
    let program = resolve_shell(configured_shell);
    let shell_name = Path::new(&program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    let args = match shell_name {
        "bash" => vec!["--noprofile".to_string(), "--norc".to_string()],
        _ => Vec::new(),
    };

    ShellLaunch { program, args }
}

fn parse_marker_output(raw: &str) -> Result<(String, i32)> {
    let marker_pos = raw
        .find(MARKER_PREFIX)
        .context("backend shell output did not contain prompt marker")?;
    let output = raw[..marker_pos].trim_matches(['\r', '\n']).to_string();
    let status_start = marker_pos + MARKER_PREFIX.len();
    let status: String = raw[status_start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if status.is_empty() {
        bail!("backend shell prompt marker did not include exit status");
    }
    let exit_code = status.parse::<i32>().context("invalid shell exit status")?;
    Ok((output, exit_code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_configured_shell_before_environment() {
        assert_eq!(resolve_shell("/bin/custom-shell"), "/bin/custom-shell");
    }

    #[test]
    fn bash_launch_uses_clean_startup_flags() {
        let launch = shell_launch("/bin/bash");
        assert_eq!(launch.program, "/bin/bash");
        assert_eq!(launch.args, ["--noprofile", "--norc"]);
    }

    #[test]
    fn non_bash_launch_does_not_receive_bash_only_flags() {
        let launch = shell_launch("/bin/zsh");
        assert_eq!(launch.program, "/bin/zsh");
        assert!(launch.args.is_empty());
    }

    #[test]
    fn parses_marker_and_hides_it_from_output() {
        let (output, status) = parse_marker_output("hello\r\n__AISH_STATUS__7\r\n").unwrap();
        assert_eq!(output, "hello");
        assert_eq!(status, 7);
    }
}
