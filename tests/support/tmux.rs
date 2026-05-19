use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

pub(crate) use crate::shell_support::{command_available, find_shell, fish_backend_tests_enabled};

static TMUX_RUN_LOCK: Mutex<()> = Mutex::new(());

fn tmux_available(tmux_tmpdir: &Path) -> bool {
    if std::fs::create_dir_all(tmux_tmpdir).is_err() {
        return false;
    }

    if !Command::new("tmux")
        .arg("-V")
        .env("TMUX_TMPDIR", tmux_tmpdir)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
    {
        return false;
    }

    let probe = format!("aish-tmux-probe-{}", std::process::id());
    let available = Command::new("tmux")
        .args(["new-session", "-d", "-s", &probe, "true"])
        .env("TMUX_TMPDIR", tmux_tmpdir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let _ = Command::new("tmux")
        .args(["kill-session", "-t", &probe])
        .env("TMUX_TMPDIR", tmux_tmpdir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    available
}

pub(crate) fn run_tmux_script(name: &str) -> Option<String> {
    run_tmux_script_with_env(name, &[])
}

pub(crate) fn run_tmux_script_with_env(name: &str, extra_env: &[(&str, &str)]) -> Option<String> {
    let _guard = TMUX_RUN_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tmux_tmpdir = tmux_tmpdir(name);
    if !tmux_available(&tmux_tmpdir) {
        eprintln!("skipping {name}: tmux is not installed or cannot create sessions");
        let _ = std::fs::remove_dir_all(&tmux_tmpdir);
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
        .env("TMUX_TMPDIR", &tmux_tmpdir);
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let output = command.output().expect("failed to launch tmux script");
    let _ = std::fs::remove_dir_all(&tmux_tmpdir);

    if !output.status.success() {
        panic!(
            "tmux script failed: {}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            script.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn tmux_tmpdir(name: &str) -> PathBuf {
    let safe_name: String = name
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    PathBuf::from("/tmp").join(format!("aish-tmux-{}-{safe_name}", std::process::id()))
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
