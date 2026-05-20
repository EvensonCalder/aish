use super::*;

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
fn tmux_shell_clear_command_redraws_prompt_on_first_line() {
    let Some(captured) = run_tmux_script("shell_clear_command.sh") else {
        return;
    };
    assert!(
        !captured.contains("before-shell-clear"),
        "captured pane still contained pre-clear output: {captured:?}"
    );
    assert_first_non_empty_line(&captured, 0);
}

#[test]
fn tmux_bash_then_clear_redraws_prompt_and_cursor_on_first_line() {
    let Some(captured) = run_tmux_script("bash_then_clear_cursor.sh") else {
        return;
    };
    assert!(
        !captured.contains("nested bash command text"),
        "captured pane still contained nested bash command text: {captured:?}"
    );
    assert_first_non_empty_line(&captured, 0);
    assert_line_prefix(&captured, "cursor=");
    let cursor = captured
        .lines()
        .find_map(|line| line.strip_prefix("cursor="))
        .unwrap_or_default();
    assert!(
        cursor.ends_with(" 0"),
        "cursor after bash then clear should be on row 0: {captured:?}"
    );
}

#[test]
fn tmux_completion_no_matches_remains_quiet_and_usable() {
    let Some(captured) = run_tmux_script("completion_no_matches.sh") else {
        return;
    };
    assert!(
        !captured.contains("no completions"),
        "captured pane history unexpectedly showed no-completions panel: {captured:?}"
    );
    assert_adjacent_output(&captured, "echo after-completion", "after-completion");
}

#[test]
fn tmux_completion_panel_is_cleared_before_enter_executes() {
    let Some(captured) = run_tmux_script("completion_enter_clears_panel.sh") else {
        return;
    };
    assert_adjacent_output(&captured, "echo after-enter", "after-enter");
}

#[test]
fn tmux_completion_auto_panel_does_not_leak_to_scrollback() {
    let Some(_captured) = run_tmux_script("completion_auto_panel_scrollback.sh") else {
        return;
    };
}

#[test]
fn tmux_completion_panel_at_bottom_does_not_repeat_in_scrollback() {
    let Some(_captured) = run_tmux_script("completion_panel_bottom_no_repeated_scroll.sh") else {
        return;
    };
}

#[test]
fn tmux_completion_right_accepts_first_and_executes() {
    let Some(captured) = run_tmux_script("completion_right_accepts.sh") else {
        return;
    };
    assert_line_present(&captured, "accepted-right");
}

#[test]
fn tmux_template_completion_accepts_placeholder_name_as_protected_draft() {
    let Some(captured) = run_tmux_script("template_completion_placeholder.sh") else {
        return;
    };
    assert_line_present(
        &captured,
        "cannot execute unresolved template placeholders: something",
    );
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
