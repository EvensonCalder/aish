use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

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

#[test]
fn tmux_output_visibility_matches_real_terminal_screen() {
    let Some(captured) = run_tmux_script("output_visibility.sh") else {
        return;
    };
    let expected_user = std::env::var("USER").unwrap_or_else(|_| "evenson".to_string());
    assert_adjacent_output(&captured, "whoami", &expected_user);
    assert_at_least_n_lines(&captured, &expected_user, 2);
    assert_adjacent_output(&captured, "echo 123", "123");
}

#[test]
fn tmux_common_shell_workflow_matches_bash_backend_real_terminal_screen() {
    if !Path::new("/bin/bash").exists() {
        eprintln!("skipping bash backend tmux workflow: /bin/bash not found");
        return;
    }
    let Some(captured) = run_tmux_script_with_env(
        "common_shell_workflow.sh",
        &[
            ("AISH_BACKEND_SHELL", "/bin/bash"),
            ("AISH_BACKEND_KIND", "bash"),
        ],
    ) else {
        return;
    };
    assert_common_shell_workflow_output(&captured);
    assert_line_prefix(&captured, "backend:bash:");
}

#[test]
fn tmux_common_shell_workflow_matches_zsh_backend_real_terminal_screen() {
    if !Path::new("/bin/zsh").exists() {
        eprintln!("skipping zsh backend tmux workflow: /bin/zsh not found");
        return;
    }
    let Some(captured) = run_tmux_script_with_env(
        "common_shell_workflow.sh",
        &[
            ("AISH_BACKEND_SHELL", "/bin/zsh"),
            ("AISH_BACKEND_KIND", "zsh"),
        ],
    ) else {
        return;
    };
    assert_common_shell_workflow_output(&captured);
    assert_line_prefix(&captured, "backend:zsh:");
}

#[test]
fn tmux_inline_completion_matches_bash_backend_real_terminal_screen() {
    if !Path::new("/bin/bash").exists() {
        eprintln!("skipping bash inline completion tmux workflow: /bin/bash not found");
        return;
    }
    let Some(captured) = run_tmux_script_with_env(
        "inline_completion_backend_independent.sh",
        &[("AISH_BACKEND_SHELL", "/bin/bash")],
    ) else {
        return;
    };
    assert_adjacent_output(&captured, "echo inline-history", "inline-history");
}

#[test]
fn tmux_inline_completion_matches_zsh_backend_real_terminal_screen() {
    if !Path::new("/bin/zsh").exists() {
        eprintln!("skipping zsh inline completion tmux workflow: /bin/zsh not found");
        return;
    }
    let Some(captured) = run_tmux_script_with_env(
        "inline_completion_backend_independent.sh",
        &[("AISH_BACKEND_SHELL", "/bin/zsh")],
    ) else {
        return;
    };
    assert_adjacent_output(&captured, "echo inline-history", "inline-history");
}

#[test]
fn tmux_common_shell_workflow_matches_fish_backend_real_terminal_screen() {
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish backend tmux workflow: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    if !command_available("fish") {
        eprintln!("skipping fish backend tmux workflow: fish not found on PATH");
        return;
    }
    let Some(captured) = run_tmux_script_with_env(
        "common_shell_workflow.sh",
        &[
            ("AISH_BACKEND_SHELL", "fish"),
            ("AISH_BACKEND_KIND", "fish"),
        ],
    ) else {
        return;
    };
    assert_common_shell_workflow_output(&captured);
    assert_line_prefix(&captured, "backend:fish:");
}

#[test]
fn tmux_unicode_output_matches_real_terminal_screen() {
    let Some(captured) = run_tmux_script("unicode_input.sh") else {
        return;
    };
    assert_adjacent_output(
        &captured,
        "printf 'unicode:%s\\n' 'café-你好'",
        "unicode:café-你好",
    );
}

#[test]
fn tmux_ctrl_l_clears_visible_screen_and_keeps_prompt_usable() {
    let Some(captured) = run_tmux_script("clear_screen.sh") else {
        return;
    };
    assert_adjacent_output(&captured, "echo after-clear", "after-clear");
    assert!(
        !captured.contains("before-clear"),
        "captured pane still contained pre-clear output: {captured:?}"
    );
}

#[test]
fn tmux_completion_no_matches_panel_remains_usable() {
    let Some(captured) = run_tmux_script("completion_no_matches.sh") else {
        return;
    };
    assert!(
        captured.contains("no completions"),
        "captured pane history did not show no-completions panel: {captured:?}"
    );
    assert_adjacent_output(&captured, "echo after-completion", "after-completion");
}

#[test]
fn tmux_completion_right_accepts_first_and_executes() {
    let Some(captured) = run_tmux_script("completion_right_accepts.sh") else {
        return;
    };
    assert_adjacent_output(&captured, "cat right-target.txt", "accepted-right");
}

#[test]
fn tmux_ctrl_c_cancels_continuation_and_shell_recovers() {
    let Some(captured) = run_tmux_script("continuation_cancel.sh") else {
        return;
    };
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
    let Some(captured) = run_tmux_script("mode_redraw_preserves_output.sh") else {
        return;
    };
    assert_adjacent_output(&captured, "echo before-mode-redraw", "before-mode-redraw");
    assert_adjacent_output(&captured, "echo after-mode-redraw", "after-mode-redraw");
}

#[test]
fn tmux_history_mode_executes_selected_command() {
    let Some(captured) = run_tmux_script("history_mode_execute.sh") else {
        return;
    };
    assert!(
        captured.contains("$ "),
        "captured pane history did not show history prompt: {captured:?}"
    );
    assert_at_least_n_lines(&captured, "history-tmux-ok", 2);
}

#[test]
fn tmux_escape_clears_draft_and_shell_recovers() {
    let Some(captured) = run_tmux_script("escape_clears_draft.sh") else {
        return;
    };
    assert!(
        !captured.lines().any(|line| line == "should-not-run"),
        "escaped draft unexpectedly executed: {captured:?}"
    );
    assert_adjacent_output(&captured, "echo after-escape", "after-escape");
}

#[test]
fn tmux_ctrl_d_exits_session_without_leftover_pane() {
    let Some(captured) = run_tmux_script("ctrl_d_exits.sh") else {
        return;
    };
    assert!(
        captured.trim().is_empty(),
        "ctrl-d exit script should not leave pane output: {captured:?}"
    );
}

#[test]
fn tmux_exit_command_terminates_session_without_leftover_pane() {
    let Some(captured) = run_tmux_script("exit_command.sh") else {
        return;
    };
    assert!(
        captured.trim().is_empty(),
        "#exit script should not leave pane output: {captured:?}"
    );
}

#[test]
fn tmux_status_command_is_visible_and_shell_recovers() {
    let Some(captured) = run_tmux_script("status_command.sh") else {
        return;
    };
    assert!(
        captured.lines().any(|line| line == "last_status=none"),
        "status last_status line was not visible: {captured:?}"
    );
    assert!(
        captured
            .lines()
            .any(|line| line == "completion.max_results=5"),
        "status completion config line was not visible: {captured:?}"
    );
    assert_adjacent_output(&captured, "echo after-status", "after-status");
}

#[test]
fn tmux_manual_real_world_commands_match_visible_terminal_behavior() {
    let Some(captured) = run_tmux_script("manual_real_world_commands.sh") else {
        return;
    };
    assert_line_present(&captured, "after-real-world");
}

#[test]
fn tmux_manual_prompt_editing_keys_match_visible_terminal_behavior() {
    let Some(captured) = run_tmux_script("manual_prompt_editing_keys.sh") else {
        return;
    };
    assert_line_present(&captured, "ctrlx-ok");
}

#[test]
fn tmux_manual_completion_workflow_matches_visible_terminal_behavior() {
    let Some(captured) = run_tmux_script("manual_completion_real_world.sh") else {
        return;
    };
    assert_line_present(&captured, "main-content");
}

#[test]
fn tmux_manual_private_config_notes_workflow_matches_visible_terminal_behavior() {
    let Some(captured) = run_tmux_script("manual_private_config_notes.sh") else {
        return;
    };
    assert_line_present(&captured, "after-private");
}

#[test]
fn tmux_manual_templates_editor_and_default_home_match_visible_terminal_behavior() {
    let Some(captured) = run_tmux_script("manual_templates_editor_default_home.sh") else {
        return;
    };
    assert_line_present(&captured, "editor-tmux-ok");
}

#[test]
fn tmux_manual_ai_context_and_sync_config_match_visible_terminal_behavior() {
    let Some(captured) = run_tmux_script("manual_ai_context_sync.sh") else {
        return;
    };
    assert_line_present(&captured, "after-ai-context-sync");
}

#[test]
fn tmux_manual_sync_local_remote_matches_visible_terminal_behavior() {
    let Some(captured) = run_tmux_script("manual_sync_local_remote.sh") else {
        return;
    };
    if captured.contains("skipping local sync tmux workflow") {
        return;
    }
    assert_line_present(&captured, "sync push completed");
}

#[test]
fn tmux_manual_passthrough_less_recovers_prompt_when_available() {
    let Some(captured) = run_tmux_script("manual_passthrough_less.sh") else {
        return;
    };
    if captured.contains("skipping passthrough tmux workflow") {
        return;
    }
    assert_line_present(&captured, "after-less");
}

#[test]
fn tmux_manual_startup_failures_are_readable_in_terminal() {
    let Some(captured) = run_tmux_script("manual_startup_failures.sh") else {
        return;
    };
    assert_at_least_n_lines(&captured, "aish-exit:1", 3);
}

#[test]
fn tmux_narrow_long_input_redraw_does_not_duplicate_prompt() {
    let Some(captured) = run_tmux_script("narrow_long_input_redraw.sh") else {
        return;
    };
    assert_line_present(&captured, "after-narrow-redraw");
}

#[test]
fn tmux_editor_and_paste_review_render_cleanly() {
    let Some(captured) = run_tmux_script("editor_paste_review_rendering.sh") else {
        return;
    };
    assert_line_present(&captured, "edited-by-tmux");
}

#[test]
fn tmux_picker_cancellation_message_starts_on_own_line() {
    let Some(captured) = run_tmux_script("picker_cancel_rendering.sh") else {
        return;
    };
    assert_line_present(&captured, "history search cancelled");
}

#[test]
fn tmux_python_repl_passthrough_recovers_prompt_when_available() {
    let Some(captured) = run_tmux_script("passthrough_python_repl.sh") else {
        return;
    };
    if captured.contains("skipping python passthrough tmux workflow") {
        return;
    }
    assert_line_present(&captured, "after-python-repl");
}

fn run_tmux_script(name: &str) -> Option<String> {
    run_tmux_script_with_env(name, &[])
}

fn run_tmux_script_with_env(name: &str, extra_env: &[(&str, &str)]) -> Option<String> {
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

fn assert_common_shell_workflow_output(captured: &str) {
    assert_line_present(captured, "beta");
    assert_line_present(captured, "quoted:value with spaces");
    assert_line_present(captured, "visible");
    assert_line_present(captured, "file-exists");
    assert_line_present(captured, "after-failure");
}

fn assert_line_present(captured: &str, expected_line: &str) {
    assert!(
        captured.lines().any(|line| line == expected_line),
        "expected line {expected_line:?}; captured pane was {captured:?}"
    );
}

fn assert_line_prefix(captured: &str, expected_prefix: &str) {
    assert!(
        captured
            .lines()
            .any(|line| line.starts_with(expected_prefix)),
        "expected line prefix {expected_prefix:?}; captured pane was {captured:?}"
    );
}

fn command_available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn fish_backend_tests_enabled() -> bool {
    std::env::var_os("AISH_TEST_FISH").is_some()
}
