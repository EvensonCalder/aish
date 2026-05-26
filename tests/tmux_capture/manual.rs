use super::*;

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
fn tmux_manual_private_config_hash_boundaries_workflow_matches_visible_terminal_behavior() {
    let Some(captured) = run_tmux_script("manual_private_config_hash_boundaries.sh") else {
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
