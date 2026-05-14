use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

static TMUX_RUN_LOCK: Mutex<()> = Mutex::new(());

fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[test]
fn tmux_output_visibility_matches_real_terminal_screen() {
    let captured = run_tmux_script("output_visibility.sh");
    let expected_user = std::env::var("USER").unwrap_or_else(|_| "evenson".to_string());
    assert_adjacent_output(&captured, "whoami", &expected_user);
    assert_adjacent_output(&captured, "echo 123", "123");
}

#[test]
fn tmux_unicode_output_matches_real_terminal_screen() {
    let captured = run_tmux_script("unicode_input.sh");
    assert_adjacent_output(
        &captured,
        "printf 'unicode:%s\\n' 'café-你好'",
        "unicode:café-你好",
    );
}

#[test]
fn tmux_ctrl_l_clears_visible_screen_and_keeps_prompt_usable() {
    let captured = run_tmux_script("clear_screen.sh");
    assert_adjacent_output(&captured, "echo after-clear", "after-clear");
    assert!(
        !captured.contains("before-clear"),
        "captured pane still contained pre-clear output: {captured:?}"
    );
}

#[test]
fn tmux_completion_no_matches_panel_remains_usable() {
    let captured = run_tmux_script("completion_no_matches.sh");
    assert!(
        captured.contains("no completions"),
        "captured pane history did not show no-completions panel: {captured:?}"
    );
    assert_adjacent_output(&captured, "echo after-completion", "after-completion");
}

#[test]
fn tmux_completion_right_accepts_first_and_executes() {
    let captured = run_tmux_script("completion_right_accepts.sh");
    assert_adjacent_output(&captured, "cat right-target.txt", "accepted-right");
}

#[test]
fn tmux_ctrl_c_cancels_continuation_and_shell_recovers() {
    let captured = run_tmux_script("continuation_cancel.sh");
    assert!(
        captured.contains("dquote>"),
        "captured pane history did not show continuation prompt: {captured:?}"
    );
    assert_adjacent_output(&captured, "echo after-cancel", "after-cancel");
    assert!(
        !captured.contains("dquote> echo after-cancel"),
        "continuation prompt survived Ctrl-C: {captured:?}"
    );
}

#[test]
fn tmux_mode_redraw_preserves_prior_output_and_shell_recovers() {
    let captured = run_tmux_script("mode_redraw_preserves_output.sh");
    assert_adjacent_output(&captured, "echo before-mode-redraw", "before-mode-redraw");
    assert_adjacent_output(&captured, "echo after-mode-redraw", "after-mode-redraw");
}

#[test]
fn tmux_history_mode_executes_selected_command() {
    let captured = run_tmux_script("history_mode_execute.sh");
    assert!(
        captured.contains("$ "),
        "captured pane history did not show history prompt: {captured:?}"
    );
    assert_at_least_n_lines(&captured, "history-tmux-ok", 2);
}

#[test]
fn tmux_escape_clears_draft_and_shell_recovers() {
    let captured = run_tmux_script("escape_clears_draft.sh");
    assert!(
        !captured.lines().any(|line| line == "should-not-run"),
        "escaped draft unexpectedly executed: {captured:?}"
    );
    assert_adjacent_output(&captured, "echo after-escape", "after-escape");
}

#[test]
fn tmux_ctrl_d_exits_session_without_leftover_pane() {
    let captured = run_tmux_script("ctrl_d_exits.sh");
    assert!(
        captured.trim().is_empty(),
        "ctrl-d exit script should not leave pane output: {captured:?}"
    );
}

fn run_tmux_script(name: &str) -> String {
    let _guard = TMUX_RUN_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !tmux_available() {
        eprintln!("skipping {name}: tmux not installed");
        return String::new();
    }

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = repo.join("tests/tmux").join(name);
    assert!(script.exists(), "missing tmux script: {}", script.display());

    let output = Command::new("sh")
        .arg(&script)
        .current_dir(&repo)
        .env("AISH_BIN", env!("CARGO_BIN_EXE_aish"))
        .output()
        .expect("failed to launch tmux script");

    if !output.status.success() {
        panic!(
            "tmux script failed: {}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            script.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn assert_adjacent_output(captured: &str, command: &str, expected_output: &str) {
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

fn assert_at_least_n_lines(captured: &str, expected_line: &str, min_count: usize) {
    let count = captured
        .lines()
        .filter(|line| *line == expected_line)
        .count();
    assert!(
        count >= min_count,
        "expected at least {min_count} {expected_line:?} lines, got {count}; captured pane was {captured:?}"
    );
}
