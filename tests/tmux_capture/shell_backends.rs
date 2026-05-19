use super::*;

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
fn tmux_carriage_return_progress_updates_in_place() {
    let Some(captured) = run_tmux_script("carriage_return_progress.sh") else {
        return;
    };
    assert_line_present(&captured, "progress 3/3");
    assert_line_absent(&captured, "progress 1/3");
    assert_line_absent(&captured, "progress 2/3");
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
fn tmux_backend_rc_inheritance_matches_bash_real_terminal_screen() {
    if !Path::new("/bin/bash").exists() {
        eprintln!("skipping bash rc inheritance tmux workflow: /bin/bash not found");
        return;
    }
    let Some(captured) = run_tmux_script_with_env(
        "backend_rc_inheritance.sh",
        &[
            ("AISH_BACKEND_SHELL", "/bin/bash"),
            ("AISH_BACKEND_KIND", "bash"),
        ],
    ) else {
        return;
    };
    assert_line_present(&captured, "alias-from-bashrc");
    assert_line_present(&captured, "function-from-bashrc");
    assert_line_present(&captured, "env:env-from-bashrc");
    assert_line_present(&captured, "path-from-bashrc");
    assert_line_present(&captured, "prompt-command:ran");
    assert!(!captured.contains("bash-prompt-noise"), "{captured:?}");
    assert!(!captured.contains("bashrc-prompt"), "{captured:?}");
}

#[test]
fn tmux_backend_rc_inheritance_matches_zsh_real_terminal_screen() {
    if !Path::new("/bin/zsh").exists() {
        eprintln!("skipping zsh rc inheritance tmux workflow: /bin/zsh not found");
        return;
    }
    let Some(captured) = run_tmux_script_with_env(
        "backend_rc_inheritance.sh",
        &[
            ("AISH_BACKEND_SHELL", "/bin/zsh"),
            ("AISH_BACKEND_KIND", "zsh"),
        ],
    ) else {
        return;
    };
    assert_line_present(&captured, "alias-from-zshrc");
    assert_line_present(&captured, "function-from-zshrc");
    assert_line_present(&captured, "env:env-from-zshrc");
    assert_line_present(&captured, "path-from-zshrc");
    assert_line_prefix(&captured, "hooks:ran|printf");
    assert!(!captured.contains("zsh-precmd-noise"), "{captured:?}");
    assert!(!captured.contains("zshrc-prompt"), "{captured:?}");
}

#[test]
fn tmux_backend_rc_inheritance_matches_fish_real_terminal_screen() {
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish rc inheritance tmux workflow: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    if !command_available("fish") {
        eprintln!("skipping fish rc inheritance tmux workflow: fish not found on PATH");
        return;
    }
    let Some(captured) = run_tmux_script_with_env(
        "backend_rc_inheritance.sh",
        &[
            ("AISH_BACKEND_SHELL", "fish"),
            ("AISH_BACKEND_KIND", "fish"),
        ],
    ) else {
        return;
    };
    assert_line_present(&captured, "function-from-fish-config");
    assert_line_present(&captured, "env:env-from-fish-config");
    assert_line_present(&captured, "path-from-fish-config");
    assert_line_prefix(&captured, "events:ran|printf");
    assert!(!captured.contains("fish-config-prompt"), "{captured:?}");
}
