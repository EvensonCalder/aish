//! Runs every `tests/expect/*.exp` script against the built `aish` binary.
//!
//! Every Aish feature should have at least one `.exp` scenario in this folder so
//! that interactive behavior (rendering, multi-line continuation, key handling,
//! shell integration) is covered end-to-end alongside the Rust unit and
//! integration tests.
//!
//! Conventions:
//!   - One `.exp` file per scenario.
//!   - `_lib.exp` is shared and not run on its own.
//!   - Each scenario must finish with a clean exit (`aish_quit` or eof).
//!   - Each scenario gets an isolated `AISH_HOME` provided by `_lib.exp`.

#[path = "support/expect.rs"]
mod expect_support;

use expect_support::{list_scripts, run_script};

#[test]
fn expect_scenarios_exist() {
    let names = list_scripts();
    assert!(
        !names.is_empty(),
        "no expect scenarios found under tests/expect"
    );
}

macro_rules! expect_scenarios {
    ($($name:ident => $script:literal,)+) => {
        $(
            #[test]
            fn $name() {
                run_script($script);
            }
        )+
    };
}

expect_scenarios! {
    basic_echo => "basic_echo.exp",
    output_visible_before_prompt => "output_visible_before_prompt.exp",
    output_then_redraw_interactions => "output_then_redraw_interactions.exp",
    mixed_stdout_stderr_redraw => "mixed_stdout_stderr_redraw.exp",
    common_shell_workflow => "common_shell_workflow.exp",
    backend_rc_inheritance_bash => "backend_rc_inheritance_bash.exp",
    backend_rc_inheritance_zsh => "backend_rc_inheritance_zsh.exp",
    backend_rc_inheritance_fish => "backend_rc_inheritance_fish.exp",
    first_run_doctor => "first_run_doctor.exp",
    home_default_first_run_doctor => "home_default_first_run_doctor.exp",
    home_default_config_persists => "home_default_config_persists.exp",
    home_default_ai_key_source_redacts_secret => "home_default_ai_key_source_redacts_secret.exp",
    invalid_config_startup => "invalid_config_startup.exp",
    home_default_invalid_config_startup => "home_default_invalid_config_startup.exp",
    home_missing_fails_cleanly => "home_missing_fails_cleanly.exp",
    aish_home_empty_uses_home => "aish_home_empty_uses_home.exp",
    aish_home_relative_fails_cleanly => "aish_home_relative_fails_cleanly.exp",
    home_unwritable_fails_cleanly => "home_unwritable_fails_cleanly.exp",
    home_path_with_spaces_works => "home_path_with_spaces_works.exp",
    home_aish_path_file_fails_cleanly => "home_aish_path_file_fails_cleanly.exp",
    cd_persists => "cd_persists.exp",
    ctrl_d_exits => "ctrl_d_exits.exp",
    dquote_continuation => "dquote_continuation.exp",
    squote_continuation => "squote_continuation.exp",
    ctrl_c_cancels_continuation => "ctrl_c_cancels_continuation.exp",
    no_backend_ps2_leak => "no_backend_ps2_leak.exp",
    empty_tab_cycles_modes => "empty_tab_cycles_modes.exp",
    help_lists_commands => "help_lists_commands.exp",
    exit_command => "exit_command.exp",
    plain_exit_command => "plain_exit_command.exp",
    backslash_continuation => "backslash_continuation.exp",
    ctrl_l_clear_screen => "ctrl_l_clear_screen.exp",
    unknown_private_command => "unknown_private_command.exp",
    private_command_safe_failures => "private_command_safe_failures.exp",
    completion_accept_single => "completion_accept_single.exp",
    completion_panel_multiple => "completion_panel_multiple.exp",
    completion_inline_off_accepts_first => "completion_inline_off_accepts_first.exp",
    completion_typo_correction_accepts_whole_command => "completion_typo_correction_accepts_whole_command.exp",
    completion_directory_typo_accepts_local_directory => "completion_directory_typo_accepts_local_directory.exp",
    completion_tab_accept_word => "completion_tab_accept_word.exp",
    completion_history_quoted_word => "completion_history_quoted_word.exp",
    completion_path_shell_escaping => "completion_path_shell_escaping.exp",
    completion_no_matches_panel => "completion_no_matches_panel.exp",
    completion_right_accepts_first => "completion_right_accepts_first.exp",
    completion_config_persists => "completion_config_persists.exp",
    completion_first_token_source_order => "completion_first_token_source_order.exp",
    template_completion_placeholder_name => "template_completion_placeholder_name.exp",
    home_default_completion_ui => "home_default_completion_ui.exp",
    history_mode_execute => "history_mode_execute.exp",
    history_persists_across_restarts => "history_persists_across_restarts.exp",
    home_default_history_persists => "home_default_history_persists.exp",
    home_default_history_trim_persists => "home_default_history_trim_persists.exp",
    history_picker_cancel_preserves_draft => "history_picker_cancel_preserves_draft.exp",
    home_default_history_picker_cancel_preserves_draft => "home_default_history_picker_cancel_preserves_draft.exp",
    file_picker_cancel_preserves_draft => "file_picker_cancel_preserves_draft.exp",
    home_default_file_picker_cancel_preserves_draft => "home_default_file_picker_cancel_preserves_draft.exp",
    template_picker_cancel_preserves_draft => "template_picker_cancel_preserves_draft.exp",
    home_default_template_picker_cancel_preserves_draft => "home_default_template_picker_cancel_preserves_draft.exp",
    git_branch_picker_cancel_preserves_draft => "git_branch_picker_cancel_preserves_draft.exp",
    home_default_git_branch_picker_cancel_preserves_draft => "home_default_git_branch_picker_cancel_preserves_draft.exp",
    env_var_picker_cancel_preserves_draft => "env_var_picker_cancel_preserves_draft.exp",
    env_var_picker_uses_backend_environment => "env_var_picker_uses_backend_environment.exp",
    home_default_env_var_picker_cancel_preserves_draft => "home_default_env_var_picker_cancel_preserves_draft.exp",
    draft_persists_across_restarts => "draft_persists_across_restarts.exp",
    home_default_draft_persists => "home_default_draft_persists.exp",
    draft_up_down_browses_saved_drafts => "draft_down_starts_new_draft.exp",
    template_use_executes => "template_use_executes.exp",
    home_default_template_persists => "home_default_template_persists.exp",
    key_encryption_sync_safe_failures => "key_encryption_sync_safe_failures.exp",
    encrypt_ambiguous_key_recovers => "encrypt_ambiguous_key_recovers.exp",
    encrypted_startup_unlock => "encrypted_startup_unlock.exp",
    home_default_sync_config_persists => "home_default_sync_config_persists.exp",
    home_default_startup_sync_runs => "home_default_startup_sync_runs.exp",
    home_default_startup_sync_unsupported_schedule => "home_default_startup_sync_unsupported_schedule.exp",
    home_default_startup_sync_failure_logs => "home_default_startup_sync_failure_logs.exp",
    home_default_startup_sync_disabled_noops => "home_default_startup_sync_disabled_noops.exp",
    home_default_sync_push_local_remote => "home_default_sync_push_local_remote.exp",
    sync_push_local_remote => "sync_push_local_remote.exp",
    sync_push_failure_logs => "sync_push_failure_logs.exp",
    sync_push_conflict_logs => "sync_push_conflict_logs.exp",
    key_clear_removes_stored_key => "key_clear_removes_stored_key.exp",
    home_default_key_clear_removes_stored_key => "home_default_key_clear_removes_stored_key.exp",
    home_default_encrypt_on_migrates_storage => "home_default_encrypt_on_migrates_storage.exp",
    status_doctor_config => "status_doctor_config.exp",
    notes_are_swallowed => "notes_are_swallowed.exp",
    home_default_notes_are_swallowed => "home_default_notes_are_swallowed.exp",
    template_placeholder_blocks_execution => "template_placeholder_blocks_execution.exp",
    context_confirmation_skip => "context_confirmation_skip.exp",
    context_config_persists => "context_config_persists.exp",
    context_off_blocks_pseudopipe => "context_off_blocks_pseudopipe.exp",
    context_confirm_off_runs_immediately => "context_confirm_off_runs_immediately.exp",
    context_dangerous_refusal => "context_dangerous_refusal.exp",
    context_dangerous_still_prompts_when_confirm_off => "context_dangerous_still_prompts_when_confirm_off.exp",
    context_truncation_reports_limit => "context_truncation_reports_limit.exp",
    home_default_context_dangerous_refusal => "home_default_context_dangerous_refusal.exp",
    external_editor_roundtrip => "external_editor_roundtrip.exp",
    home_default_external_editor_roundtrip => "home_default_external_editor_roundtrip.exp",
    external_editor_failure_preserves_draft => "external_editor_failure_preserves_draft.exp",
    home_default_external_editor_failure_preserves_draft => "home_default_external_editor_failure_preserves_draft.exp",
    multiline_paste_editor_review => "multiline_paste_editor_review.exp",
    paste_config_persists => "paste_config_persists.exp",
    home_default_multiline_paste_editor_review => "home_default_multiline_paste_editor_review.exp",
    read_only_edit_copies_to_draft => "read_only_edit_copies_to_draft.exp",
    log_shows_context_skip => "log_shows_context_skip.exp",
    home_default_event_log_persists => "home_default_event_log_persists.exp",
    ai_mode_executes_sequence => "ai_mode_executes_sequence.exp",
    ai_prompt_editor_roundtrip => "ai_prompt_editor_roundtrip.exp",
    home_default_ai_mode_executes_sequence => "home_default_ai_mode_executes_sequence.exp",
    ai_mode_edit_copies_to_draft => "ai_mode_edit_copies_to_draft.exp",
    home_default_ai_mode_edit_copies_to_draft => "home_default_ai_mode_edit_copies_to_draft.exp",
    ai_config_persists => "ai_config_persists.exp",
    readline_editing_keys => "readline_editing_keys.exp",
    terminal_resize => "terminal_resize.exp",
    passthrough_less => "passthrough_less.exp",
    escape_clears_draft => "escape_clears_draft.exp",
    ctrl_x_unknown_chord_cancels => "ctrl_x_unknown_chord_cancels.exp",
    history_trim_persists => "history_trim_persists.exp",
    template_crud => "template_crud.exp",
    editor_hash_content_uses_private_parser => "editor_hash_content_uses_private_parser.exp",
}
