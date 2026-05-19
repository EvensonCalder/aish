use super::*;

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
fn tmux_draft_up_down_browses_saved_drafts() {
    let Some(captured) = run_tmux_script("draft_down_new_draft.sh") else {
        return;
    };
    assert_adjacent_output(
        &captured,
        "echo after-down-new-draft",
        "after-down-new-draft",
    );
}

#[test]
fn tmux_ctrl_d_exits_session_without_leftover_pane() {
    let Some(captured) = run_tmux_script("ctrl_d_exits.sh") else {
        return;
    };
    assert_line_present(&captured, "__AISH_AFTER_CTRL_D__");
    assert!(
        captured.contains("exit"),
        "ctrl-d exit should render an exit marker before closing: {captured:?}"
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
            .any(|line| line == "completion.enabled=true"),
        "status completion enabled line was not visible: {captured:?}"
    );
    assert!(
        captured
            .lines()
            .any(|line| line == "completion.max_results=5"),
        "status completion config line was not visible: {captured:?}"
    );
    assert!(
        captured
            .lines()
            .any(|line| line == "completion.coalesce_ms=50"),
        "status completion coalesce config line was not visible: {captured:?}"
    );
    assert!(
        captured
            .lines()
            .any(|line| line == "completion.display_delay_ms=120"),
        "status completion display delay config line was not visible: {captured:?}"
    );
    assert_adjacent_output(&captured, "echo after-status", "after-status");
}
