use super::*;

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
    assert_line_present(&captured, "ok");
}

#[test]
fn tmux_wrap_boundary_cursor_moves_to_next_line() {
    let Some(captured) = run_tmux_script("wrap_boundary_cursor.sh") else {
        return;
    };
    assert_line_present(&captured, "cursor=1 1");
}

#[test]
fn tmux_cjk_cursor_wrap_moves_to_next_line() {
    let Some(captured) = run_tmux_script("cjk_cursor_wrap.sh") else {
        return;
    };
    assert_line_present(&captured, "cursor=1 1");
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

#[test]
fn tmux_stdin_and_gpg_like_passthrough_recovers_prompt() {
    let Some(captured) = run_tmux_script("passthrough_stdin_recovery.sh") else {
        return;
    };
    assert_line_present(&captured, "stdin-blocker-ready");
    assert_line_present(&captured, "after-stdin-blocker");
    assert_line_present(&captured, "after-gpg");
}

#[test]
fn tmux_unknown_tui_passthrough_recovers_prompt() {
    let Some(captured) = run_tmux_script("passthrough_unknown_tui.sh") else {
        return;
    };
    assert!(captured.contains("unknown-tui-key:x"), "{captured:?}");
    assert_line_present(&captured, "after-unknown-tui");
}

#[test]
fn tmux_passthrough_forwards_raw_function_key_bytes() {
    let Some(captured) = run_tmux_script("passthrough_raw_key_sequences.sh") else {
        return;
    };
    assert_line_prefix(&captured, "raw-key-hex:1b");
    assert_line_present(&captured, "after-raw-key");
}

#[test]
fn tmux_sudo_password_prompt_waits_for_user_input() {
    let Some(captured) = run_tmux_script("sudo_password_passthrough.sh") else {
        return;
    };
    assert_line_present(&captured, "fake-sudo-password=pw-ok");
    assert_line_present(&captured, "after-sudo");
}

#[test]
fn tmux_rm_write_protected_prompt_waits_for_user_input() {
    let Some(captured) = run_tmux_script_with_env(
        "rm_write_protected_prompt.sh",
        &[("AISH_BACKEND_SHELL", "/bin/bash")],
    ) else {
        return;
    };
    assert!(
        captured.contains("remove") || captured.contains("override"),
        "{captured:?}"
    );
    assert!(captured.contains("1.t"), "{captured:?}");
    assert_line_present(&captured, "rm-declined");
}

#[test]
fn tmux_rm_write_protected_prompt_waits_for_user_input_zsh_backend() {
    let Some(zsh) = find_shell(&["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"]) else {
        eprintln!("skipping zsh rm prompt tmux workflow: zsh not found");
        return;
    };
    let Some(captured) = run_tmux_script_with_env(
        "rm_write_protected_prompt.sh",
        &[("AISH_BACKEND_SHELL", zsh)],
    ) else {
        return;
    };
    assert!(
        captured.contains("remove") || captured.contains("override"),
        "{captured:?}"
    );
    assert!(captured.contains("1.t"), "{captured:?}");
    assert_line_present(&captured, "rm-declined");
}

#[test]
fn tmux_rm_write_protected_prompt_waits_for_user_input_fish_backend() {
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish rm prompt tmux workflow: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    if !command_available("fish") {
        eprintln!("skipping fish rm prompt tmux workflow: fish not found on PATH");
        return;
    }
    let Some(captured) = run_tmux_script_with_env(
        "rm_write_protected_prompt.sh",
        &[("AISH_BACKEND_SHELL", "fish")],
    ) else {
        return;
    };
    assert!(captured.contains("remove"), "{captured:?}");
    assert!(captured.contains("1.t"), "{captured:?}");
    assert_line_present(&captured, "rm-declined");
}
