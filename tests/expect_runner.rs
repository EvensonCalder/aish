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

use std::path::{Path, PathBuf};
use std::process::Command;

fn expect_bin() -> Option<PathBuf> {
    for candidate in [
        "/usr/bin/expect",
        "/usr/local/bin/expect",
        "/opt/homebrew/bin/expect",
    ] {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn scripts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/expect")
}

fn aish_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aish"))
}

fn run_script(name: &str) {
    let Some(expect) = expect_bin() else {
        eprintln!("skipping {name}: `expect` not installed");
        return;
    };
    let script = scripts_dir().join(name);
    assert!(
        script.exists(),
        "missing expect script: {}",
        script.display()
    );

    let output = Command::new(&expect)
        .arg(&script)
        .env("AISH_BIN", aish_binary())
        .output()
        .expect("failed to launch expect");

    if !output.status.success() {
        panic!(
            "expect script failed: {}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            script.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn list_scripts() -> Vec<String> {
    let dir = scripts_dir();
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("cannot read tests/expect") {
        let entry = entry.expect("bad dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("exp") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        if name.starts_with('_') {
            continue;
        }
        names.push(name);
    }
    names.sort();
    names
}

#[test]
fn expect_scenarios_exist() {
    let names = list_scripts();
    assert!(
        !names.is_empty(),
        "no expect scenarios found under tests/expect"
    );
}

#[test]
fn basic_echo() {
    run_script("basic_echo.exp");
}

#[test]
fn output_visible_before_prompt() {
    run_script("output_visible_before_prompt.exp");
}

#[test]
fn output_then_redraw_interactions() {
    run_script("output_then_redraw_interactions.exp");
}

#[test]
fn mixed_stdout_stderr_redraw() {
    run_script("mixed_stdout_stderr_redraw.exp");
}

#[test]
fn first_run_doctor() {
    run_script("first_run_doctor.exp");
}

#[test]
fn home_default_first_run_doctor() {
    run_script("home_default_first_run_doctor.exp");
}

#[test]
fn home_default_config_persists() {
    run_script("home_default_config_persists.exp");
}

#[test]
fn home_default_ai_key_source_redacts_secret() {
    run_script("home_default_ai_key_source_redacts_secret.exp");
}

#[test]
fn invalid_config_startup() {
    run_script("invalid_config_startup.exp");
}

#[test]
fn home_default_invalid_config_startup() {
    run_script("home_default_invalid_config_startup.exp");
}

#[test]
fn home_missing_fails_cleanly() {
    run_script("home_missing_fails_cleanly.exp");
}

#[test]
fn aish_home_empty_uses_home() {
    run_script("aish_home_empty_uses_home.exp");
}

#[test]
fn aish_home_relative_fails_cleanly() {
    run_script("aish_home_relative_fails_cleanly.exp");
}

#[test]
fn home_unwritable_fails_cleanly() {
    run_script("home_unwritable_fails_cleanly.exp");
}

#[test]
fn home_path_with_spaces_works() {
    run_script("home_path_with_spaces_works.exp");
}

#[test]
fn home_aish_path_file_fails_cleanly() {
    run_script("home_aish_path_file_fails_cleanly.exp");
}

#[test]
fn cd_persists() {
    run_script("cd_persists.exp");
}

#[test]
fn ctrl_d_exits() {
    run_script("ctrl_d_exits.exp");
}

#[test]
fn dquote_continuation() {
    run_script("dquote_continuation.exp");
}

#[test]
fn squote_continuation() {
    run_script("squote_continuation.exp");
}

#[test]
fn ctrl_c_cancels_continuation() {
    run_script("ctrl_c_cancels_continuation.exp");
}

#[test]
fn no_backend_ps2_leak() {
    run_script("no_backend_ps2_leak.exp");
}

#[test]
fn empty_tab_cycles_modes() {
    run_script("empty_tab_cycles_modes.exp");
}

#[test]
fn help_lists_commands() {
    run_script("help_lists_commands.exp");
}

#[test]
fn exit_command() {
    run_script("exit_command.exp");
}

#[test]
fn backslash_continuation() {
    run_script("backslash_continuation.exp");
}

#[test]
fn ctrl_l_clear_screen() {
    run_script("ctrl_l_clear_screen.exp");
}

#[test]
fn unknown_private_command() {
    run_script("unknown_private_command.exp");
}

#[test]
fn private_command_safe_failures() {
    run_script("private_command_safe_failures.exp");
}

#[test]
fn completion_accept_single() {
    run_script("completion_accept_single.exp");
}

#[test]
fn completion_panel_multiple() {
    run_script("completion_panel_multiple.exp");
}

#[test]
fn history_mode_execute() {
    run_script("history_mode_execute.exp");
}

#[test]
fn history_persists_across_restarts() {
    run_script("history_persists_across_restarts.exp");
}

#[test]
fn home_default_history_persists() {
    run_script("home_default_history_persists.exp");
}

#[test]
fn home_default_history_trim_persists() {
    run_script("home_default_history_trim_persists.exp");
}

#[test]
fn history_picker_cancel_preserves_draft() {
    run_script("history_picker_cancel_preserves_draft.exp");
}

#[test]
fn file_picker_cancel_preserves_draft() {
    run_script("file_picker_cancel_preserves_draft.exp");
}

#[test]
fn template_picker_cancel_preserves_draft() {
    run_script("template_picker_cancel_preserves_draft.exp");
}

#[test]
fn git_branch_picker_cancel_preserves_draft() {
    run_script("git_branch_picker_cancel_preserves_draft.exp");
}

#[test]
fn env_var_picker_cancel_preserves_draft() {
    run_script("env_var_picker_cancel_preserves_draft.exp");
}

#[test]
fn draft_persists_across_restarts() {
    run_script("draft_persists_across_restarts.exp");
}

#[test]
fn home_default_draft_persists() {
    run_script("home_default_draft_persists.exp");
}

#[test]
fn template_use_executes() {
    run_script("template_use_executes.exp");
}

#[test]
fn home_default_template_persists() {
    run_script("home_default_template_persists.exp");
}

#[test]
fn key_and_sync_placeholders() {
    run_script("key_and_sync_placeholders.exp");
}

#[test]
fn home_default_sync_config_persists() {
    run_script("home_default_sync_config_persists.exp");
}

#[test]
fn home_default_startup_sync_runs() {
    run_script("home_default_startup_sync_runs.exp");
}

#[test]
fn home_default_startup_sync_unsupported_schedule() {
    run_script("home_default_startup_sync_unsupported_schedule.exp");
}

#[test]
fn home_default_startup_sync_failure_logs() {
    run_script("home_default_startup_sync_failure_logs.exp");
}

#[test]
fn home_default_startup_sync_disabled_noops() {
    run_script("home_default_startup_sync_disabled_noops.exp");
}

#[test]
fn sync_push_local_remote() {
    run_script("sync_push_local_remote.exp");
}

#[test]
fn sync_push_failure_logs() {
    run_script("sync_push_failure_logs.exp");
}

#[test]
fn sync_push_conflict_logs() {
    run_script("sync_push_conflict_logs.exp");
}

#[test]
fn key_clear_removes_stored_key() {
    run_script("key_clear_removes_stored_key.exp");
}

#[test]
fn home_default_key_clear_removes_stored_key() {
    run_script("home_default_key_clear_removes_stored_key.exp");
}

#[test]
fn home_default_encrypt_placeholder_noops() {
    run_script("home_default_encrypt_placeholder_noops.exp");
}

#[test]
fn status_doctor_config() {
    run_script("status_doctor_config.exp");
}

#[test]
fn notes_are_swallowed() {
    run_script("notes_are_swallowed.exp");
}

#[test]
fn home_default_notes_are_swallowed() {
    run_script("home_default_notes_are_swallowed.exp");
}

#[test]
fn template_placeholder_blocks_execution() {
    run_script("template_placeholder_blocks_execution.exp");
}

#[test]
fn context_confirmation_skip() {
    run_script("context_confirmation_skip.exp");
}

#[test]
fn context_dangerous_refusal() {
    run_script("context_dangerous_refusal.exp");
}

#[test]
fn home_default_context_dangerous_refusal() {
    run_script("home_default_context_dangerous_refusal.exp");
}

#[test]
fn external_editor_roundtrip() {
    run_script("external_editor_roundtrip.exp");
}

#[test]
fn home_default_external_editor_roundtrip() {
    run_script("home_default_external_editor_roundtrip.exp");
}

#[test]
fn external_editor_failure_preserves_draft() {
    run_script("external_editor_failure_preserves_draft.exp");
}

#[test]
fn multiline_paste_editor_review() {
    run_script("multiline_paste_editor_review.exp");
}

#[test]
fn read_only_edit_copies_to_draft() {
    run_script("read_only_edit_copies_to_draft.exp");
}

#[test]
fn log_shows_context_skip() {
    run_script("log_shows_context_skip.exp");
}

#[test]
fn home_default_event_log_persists() {
    run_script("home_default_event_log_persists.exp");
}

#[test]
fn ai_mode_executes_sequence() {
    run_script("ai_mode_executes_sequence.exp");
}

#[test]
fn ai_mode_edit_copies_to_draft() {
    run_script("ai_mode_edit_copies_to_draft.exp");
}

#[test]
fn ai_config_persists() {
    run_script("ai_config_persists.exp");
}

#[test]
fn readline_editing_keys() {
    run_script("readline_editing_keys.exp");
}

#[test]
fn long_unicode_input() {
    run_script("long_unicode_input.exp");
}

#[test]
fn terminal_resize() {
    run_script("terminal_resize.exp");
}

#[test]
fn passthrough_less() {
    run_script("passthrough_less.exp");
}

#[test]
fn escape_clears_draft() {
    run_script("escape_clears_draft.exp");
}

#[test]
fn ctrl_x_unknown_chord_cancels() {
    run_script("ctrl_x_unknown_chord_cancels.exp");
}

#[test]
fn history_trim_persists() {
    run_script("history_trim_persists.exp");
}

#[test]
fn template_crud() {
    run_script("template_crud.exp");
}

#[test]
fn editor_hash_content_bypasses_parser() {
    run_script("editor_hash_content_bypasses_parser.exp");
}

#[allow(dead_code)]
fn _unused_path_check(_p: &Path) {}
