#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use crate::process_support::{RunLimiter, default_test_jobs, run_with_timeout, script_timeout};
pub(crate) use crate::shell_support::{command_available, find_shell, fish_backend_tests_enabled};

static TMUX_RUN_LIMITER: OnceLock<RunLimiter> = OnceLock::new();

fn tmux_run_limiter() -> &'static RunLimiter {
    TMUX_RUN_LIMITER.get_or_init(|| RunLimiter::new(default_test_jobs("AISH_TMUX_TEST_JOBS")))
}

fn tmux_available(tmux_socket: &Path) -> bool {
    if let Some(parent) = tmux_socket.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return false;
    }

    if !Command::new(tmux_binary())
        .arg("-V")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
    {
        return false;
    }

    let tmux = tmux_binary();
    let probe = format!("aish-tmux-probe-{}", std::process::id());
    let _ = Command::new(&tmux)
        .arg("-S")
        .arg(tmux_socket)
        .args(["new-session", "-d", "-s", &probe, "sleep 30"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let available = Command::new(&tmux)
        .arg("-S")
        .arg(tmux_socket)
        .args(["has-session", "-t", &probe])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let _ = Command::new(&tmux)
        .arg("-S")
        .arg(tmux_socket)
        .arg("kill-server")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    available
}

pub(crate) fn run_tmux_script(name: &str) -> Option<String> {
    run_tmux_script_with_env(name, &[])
}

pub(crate) fn run_tmux_script_with_env(name: &str, extra_env: &[(&str, &str)]) -> Option<String> {
    let _permit = tmux_run_limiter().acquire();
    let tmux_tmpdir = tmux_tempdir();
    let tmux_socket = tmux_tmpdir.path().join("tmux.sock");
    let tmux_wrapper = create_tmux_wrapper(tmux_tmpdir.path(), &tmux_socket, &tmux_binary());
    let artifact_dir = tmux_tmpdir.path().join("a");
    std::fs::create_dir_all(&artifact_dir).expect("failed to create tmux test artifact dir");

    if !tmux_available(&tmux_socket) {
        eprintln!("skipping {name}: tmux is not installed or cannot create sessions");
        return None;
    }

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = repo.join("tests/tmux").join(name);
    assert!(script.exists(), "missing tmux script: {}", script.display());

    let mut command = Command::new("sh");
    command
        .arg(&script)
        .current_dir(&repo)
        .env("AISH_BIN", env!("CARGO_BIN_EXE_aish"))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("PATH", prepend_to_path(&tmux_wrapper))
        .env("AISH_TMUX_ARTIFACT_DIR", &artifact_dir);
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let result = run_with_timeout(
        command,
        script_timeout("AISH_TMUX_TEST_TIMEOUT_SECS", 120),
        || terminate_tmux_server(&tmux_socket),
    )
    .expect("failed to launch tmux script");
    terminate_tmux_server(&tmux_socket);
    let output = result.output;

    if result.timed_out || !output.status.success() {
        panic!(
            "tmux script failed: {}\nstatus: {}\ntimed out: {}\nstdout:\n{}\nstderr:\n{}",
            script.display(),
            output.status,
            result.timed_out,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn tmux_tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("at-")
        .tempdir_in("/tmp")
        .expect("failed to create tmux test tempdir")
}

fn create_tmux_wrapper(root: &Path, tmux_socket: &Path, tmux_bin: &Path) -> PathBuf {
    let bin_dir = root.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("failed to create tmux wrapper dir");
    let wrapper = bin_dir.join("tmux");
    let script = format!(
        "#!/bin/sh\nexec {} -S {} \"$@\"\n",
        shell_quote(&tmux_bin.to_string_lossy()),
        shell_quote(&tmux_socket.to_string_lossy())
    );
    std::fs::write(&wrapper, script).expect("failed to write tmux wrapper");
    #[cfg(unix)]
    {
        let mut permissions = std::fs::metadata(&wrapper)
            .expect("failed to stat tmux wrapper")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&wrapper, permissions)
            .expect("failed to make tmux wrapper executable");
    }
    bin_dir
}

fn prepend_to_path(prefix: &Path) -> std::ffi::OsString {
    let mut value = std::ffi::OsString::from(prefix);
    if let Some(path) = std::env::var_os("PATH") {
        value.push(":");
        value.push(path);
    }
    value
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn tmux_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("tmux");
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from("tmux")
}

fn terminate_tmux_server(tmux_socket: &Path) {
    let _ = Command::new(tmux_binary())
        .arg("-S")
        .arg(tmux_socket)
        .arg("kill-server")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub(crate) fn assert_adjacent_output(captured: &str, command: &str, expected_output: &str) {
    let lines: Vec<&str> = captured.lines().collect();
    for pair in lines.windows(2) {
        if pair[0].ends_with(command) && pair[1] == expected_output {
            return;
        }
    }
    panic!(
        "expected {expected_output:?} immediately after {command:?}; captured pane was {captured:?}"
    );
}

pub(crate) fn assert_at_least_n_lines(captured: &str, expected_line: &str, min_count: usize) {
    let count = captured
        .lines()
        .filter(|line| *line == expected_line)
        .count();
    assert!(
        count >= min_count,
        "expected at least {min_count} {expected_line:?} lines, got {count}; captured pane was {captured:?}"
    );
}

pub(crate) fn assert_common_shell_workflow_output(captured: &str) {
    assert_line_present(captured, "beta-output");
    assert_line_present(captured, "quoted:value with spaces");
    assert_line_present(captured, "visible");
    assert_line_present(captured, "file exists");
    assert_line_present(captured, "after:failure");
}

pub(crate) fn assert_line_present(captured: &str, expected_line: &str) {
    assert!(
        captured.lines().any(|line| line == expected_line),
        "expected line {expected_line:?}; captured pane was {captured:?}"
    );
}

pub(crate) fn assert_line_absent(captured: &str, unexpected_line: &str) {
    assert!(
        !captured.lines().any(|line| line == unexpected_line),
        "unexpected line {unexpected_line:?}; captured pane was {captured:?}"
    );
}

pub(crate) fn assert_line_prefix(captured: &str, expected_prefix: &str) {
    assert!(
        captured
            .lines()
            .any(|line| line.starts_with(expected_prefix)),
        "expected line prefix {expected_prefix:?}; captured pane was {captured:?}"
    );
}

pub(crate) fn assert_first_non_empty_line(captured: &str, expected_index: usize) {
    let index = captured
        .lines()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(usize::MAX);
    assert_eq!(
        index, expected_index,
        "expected first non-empty line index {expected_index}; captured pane was {captured:?}"
    );
}
