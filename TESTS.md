# Aish Test Summary

This document records the current implementation status, the tests that cover each feature, and the latest verified test commands.

Last full verification performed during development:

```text
cargo fmt --check
cargo test --lib
cargo test --test pty_backend -- --nocapture
AISH_TEST_FISH=1 cargo test --test pty_backend -- --nocapture
cargo test --test expect_runner -- --nocapture
cargo test --test tmux_capture -- --test-threads=1 --nocapture
AISH_TEST_FISH=1 cargo test --test tmux_capture tmux_common_shell_workflow_matches_fish_backend_real_terminal_screen -- --nocapture
cargo clippy --all-targets -- -D warnings
git diff --check
```

Current test inventory:

- 490 library unit tests.
- 26 draft execution integration tests.
- 1 first-run integration test.
- 44 tmux screen-capture integration tests.
- 9 of the tmux tests are manual-equivalent workflows that replaced deterministic rows from `MANUAL_TESTS.md`: real-world shell commands, prompt editing keys, completion UX mechanics, private commands/notes, templates/editor/default home, AI/context/sync config, local sync, `less` passthrough smoke, and startup failure messages.
- 21 PTY integration tests, including default bash/zsh coverage and fish cases that skip unless fish opt-in prerequisites are available.
- 114 expect-driven end-to-end interactive scenarios.
- Expect scenarios are serialized inside `expect_runner` because they launch real interactive terminals; parallel execution created false `no prompt` and Tcl/expect crash failures that did not match actual single-user operation.
- Expect scenarios force `commit.gpgsign=false` through `GIT_CONFIG_COUNT` so temporary local git repositories do not depend on a developer's global GPG/pinentry setup.
- Tmux screen-capture tests are serialized inside `tmux_capture` for the same reason: they launch real terminal panes and assert screen state.
- `MANUAL_TESTS.md` now contains only human-only checks; deterministic manual workflows should be added to tmux/expect/Rust instead of expanding the human checklist.
- Most tmux tests assert final visible rows; longer backend-specific workflows capture pane scrollback so normal scrolling does not make earlier command output disappear from the assertion window.
- Tmux screen-capture tests use an isolated short `TMUX_TMPDIR` under `/tmp` so they do not attach to a user's tmux server or exceed Unix socket path limits on macOS.
- Tmux screen-capture tests skip cleanly when `tmux` is unavailable or cannot create a local session.
- Backend-specific tmux workflows write an isolated `shell.backend` config and run against bash and zsh by default; fish is available as opt-in coverage with `AISH_TEST_FISH=1`.
- Tmux pane capture trims trailing spaces, so tmux tests must not be used as the only assertion for prompt suffix spaces; expect byte-stream and Rust rendering tests cover those details.
- Bash PTY startup records the backend shell's initial cwd so the first prompt matches the shell state before any command executes.
- Backend PTY startup inherits Aish's current directory and can be resized so child commands such as `ls` see the real terminal width.
- 0 doctests.

Current expected result:

- All active tests pass.

## Test Commands

Use these commands before committing feature changes:

```text
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Useful focused commands:

```text
cargo test --lib
cargo test --test draft_execution -- --nocapture
cargo test --test first_run -- --nocapture
cargo test --test pty_backend -- --nocapture
cargo test --test expect_runner -- --nocapture
cargo test --test tmux_capture -- --test-threads=1 --nocapture
cargo test -- --list
```

## Expect Coverage Matrix

Expect scenarios are the acceptance layer for user-visible terminal behavior. The matrix below must be updated whenever a feature, private command, or terminal regression is added.

| Area | Current scenarios | Status | Known gaps |
| --- | --- | --- | --- |
| Basic command execution and prompt redraw | `basic_echo`, `common_shell_workflow`, `tmux_common_shell_workflow_matches_bash_backend_real_terminal_screen`, `tmux_common_shell_workflow_matches_zsh_backend_real_terminal_screen`, `tmux_common_shell_workflow_matches_fish_backend_real_terminal_screen`, `tmux_manual_real_world_commands_match_visible_terminal_behavior`, `output_visible_before_prompt`, `output_then_redraw_interactions`, `mixed_stdout_stderr_redraw` | Covered for bash/zsh by default | Fish tmux coverage is opt-in with `AISH_TEST_FISH=1` until cross-platform fish behavior is validated. |
| Backend cwd and shell state | `cd_persists`, `tmux_common_shell_workflow_matches_bash_backend_real_terminal_screen`, `tmux_common_shell_workflow_matches_zsh_backend_real_terminal_screen`, `tmux_common_shell_workflow_matches_fish_backend_real_terminal_screen` | Covered for bash/zsh by default | Fish tmux coverage is opt-in with `AISH_TEST_FISH=1`; add shell-specific variants only when new backend integration behavior is added. |
| Shell continuation UX | `dquote_continuation`, `squote_continuation`, `backslash_continuation`, `ctrl_c_cancels_continuation`, `tmux_ctrl_c_cancels_continuation_and_shell_recovers`, `no_backend_ps2_leak` | Covered | Add heredoc-style continuation if it becomes user-facing. |
| Prompt/control keys | `ctrl_l_clear_screen`, `tmux_ctrl_l_clears_visible_screen_and_keeps_prompt_usable`, `tmux_shell_clear_command_redraws_prompt_on_first_line`, `readline_editing_keys`, `tmux_manual_prompt_editing_keys_match_visible_terminal_behavior`, `tmux_narrow_long_input_redraw_does_not_duplicate_prompt`, `tmux_unicode_output_matches_real_terminal_screen`, `terminal_resize`, `escape_clears_draft`, `tmux_escape_clears_draft_and_shell_recovers`, `ctrl_x_unknown_chord_cancels`, `ctrl_d_exits`, `tmux_ctrl_d_exits_session_without_leftover_pane`, `exit_command`, `tmux_exit_command_terminates_session_without_leftover_pane` | Covered | Add new prompt-control scenarios only for observed regressions. |
| Mode switching and read-only behavior | `empty_tab_cycles_modes`, `tmux_mode_redraw_preserves_prior_output_and_shell_recovers`, `history_mode_execute`, `tmux_history_mode_executes_selected_command`, `history_persists_across_restarts`, `home_default_history_persists`, `home_default_history_trim_persists`, `draft_persists_across_restarts`, `home_default_draft_persists`, `read_only_edit_copies_to_draft`, `ai_mode_executes_sequence`, `home_default_ai_mode_executes_sequence`, `ai_mode_edit_copies_to_draft`, `home_default_ai_mode_edit_copies_to_draft`, `output_then_redraw_interactions` | Covered | Add more mode redraw regressions only for observed failures. |
| Completion UI | `completion_accept_single`, `completion_panel_multiple`, `completion_inline_off_accepts_first`, `completion_tab_accept_word`, `completion_no_matches_panel`, `tmux_completion_no_matches_remains_quiet_and_usable`, `completion_right_accepts_first`, `tmux_completion_right_accepts_first_and_executes`, `completion_typo_correction_accepts_whole_command`, `completion_directory_typo_accepts_local_directory`, `template_completion_placeholder_name`, `tmux_template_completion_accepts_placeholder_name_as_protected_draft`, `tmux_completion_auto_panel_does_not_leak_to_scrollback`, `tmux_completion_panel_at_bottom_does_not_repeat_in_scrollback`, `tmux_completion_panel_is_cleared_before_enter_executes`, `tmux_inline_completion_matches_bash_backend_real_terminal_screen`, `tmux_inline_completion_matches_zsh_backend_real_terminal_screen`, `tmux_manual_completion_workflow_matches_visible_terminal_behavior`, `completion_config_persists`, `completion_first_token_source_order`, `home_default_completion_ui`, `output_then_redraw_interactions` | Covered for bash/zsh by default | Fish backend completion remains opt-in with `AISH_TEST_FISH=1` until cross-platform fish behavior is validated. Completion rows show full replacement commands for non-inline candidates and elide by word boundary in narrow terminals. |
| Picker cancellation UX | `history_picker_cancel_preserves_draft`, `home_default_history_picker_cancel_preserves_draft`, `file_picker_cancel_preserves_draft`, `home_default_file_picker_cancel_preserves_draft`, `template_picker_cancel_preserves_draft`, `home_default_template_picker_cancel_preserves_draft`, `git_branch_picker_cancel_preserves_draft`, `home_default_git_branch_picker_cancel_preserves_draft`, `env_var_picker_cancel_preserves_draft`, `env_var_picker_uses_backend_environment`, `home_default_env_var_picker_cancel_preserves_draft`, `tmux_picker_cancellation_message_starts_on_own_line` | Covered | Add picker success-path expect scenarios only when picker replacement UI changes; Rust tests cover replacement logic. |
| Private command UX and diagnostics | `first_run_doctor`, `home_default_first_run_doctor`, `home_default_config_persists`, `home_default_ai_key_source_redacts_secret`, `invalid_config_startup`, `home_default_invalid_config_startup`, `home_missing_fails_cleanly`, `aish_home_empty_uses_home`, `aish_home_relative_fails_cleanly`, `home_unwritable_fails_cleanly`, `home_path_with_spaces_works`, `home_aish_path_file_fails_cleanly`, `tmux_manual_startup_failures_are_readable_in_terminal`, `help_lists_commands`, `unknown_private_command`, `private_command_safe_failures`, `status_doctor_config`, `tmux_status_command_is_visible_and_shell_recovers`, `tmux_manual_private_config_notes_workflow_matches_visible_terminal_behavior`, `key_encryption_sync_safe_failures`, `key_clear_removes_stored_key`, `ai_config_persists` | Covered | Add new safe-failure scenarios when new private commands are added. |
| Notes, context, and logs | `notes_are_swallowed`, `home_default_notes_are_swallowed`, `tmux_manual_private_config_notes_workflow_matches_visible_terminal_behavior`, `context_config_persists`, `context_off_blocks_pseudopipe`, `context_confirm_off_runs_immediately`, `context_confirmation_skip`, `context_dangerous_refusal`, `context_dangerous_still_prompts_when_confirm_off`, `context_truncation_reports_limit`, `tmux_manual_ai_context_and_sync_config_match_visible_terminal_behavior`, `home_default_context_dangerous_refusal`, `log_shows_context_skip`, `home_default_event_log_persists` | Covered | Add new context scenarios only for observed regressions. |
| Templates | `template_use_executes`, `template_crud`, `template_placeholder_blocks_execution`, `home_default_template_persists`, `tmux_manual_templates_editor_and_default_home_match_visible_terminal_behavior` | Covered | Add completion/template interaction if UI changes. |
| Editor and paste flows | `external_editor_roundtrip`, `home_default_external_editor_roundtrip`, `external_editor_failure_preserves_draft`, `home_default_external_editor_failure_preserves_draft`, `tmux_manual_templates_editor_and_default_home_match_visible_terminal_behavior`, `tmux_editor_and_paste_review_render_cleanly`, `editor_hash_content_bypasses_parser`, `multiline_paste_editor_review`, `home_default_multiline_paste_editor_review` | Covered | Real OS clipboard and full-screen editor behavior remains human-only in `MANUAL_TESTS.md`. |
| Sync | `key_encryption_sync_safe_failures`, `home_default_sync_config_persists`, `home_default_startup_sync_runs`, `home_default_startup_sync_unsupported_schedule`, `home_default_startup_sync_failure_logs`, `home_default_startup_sync_disabled_noops`, `home_default_sync_push_local_remote`, `sync_push_local_remote`, `sync_push_failure_logs`, `sync_push_conflict_logs`, `tmux_manual_ai_context_and_sync_config_match_visible_terminal_behavior`, `tmux_manual_sync_local_remote_matches_visible_terminal_behavior` | Covered | Real remote auth and human conflict review remain manual-only. |
| Passthrough/interactive programs | `passthrough_less`, `tmux_manual_passthrough_less_recovers_prompt_when_available` when `less` is available, `tmux_python_repl_passthrough_recovers_prompt_when_available` when `python3` is available, `tmux_stdin_and_gpg_like_passthrough_recovers_prompt`, backend interrupt recovery through `pty_backend_wait_callback_can_interrupt_long_running_commands`; key forwarding is Rust-covered | Partial | Broader real interactive programs remain human-only because alternate-screen and job-control behavior vary by environment. |
| Encryption/GPG | `key_clear_removes_stored_key`, `home_default_key_clear_removes_stored_key`, `home_default_encrypt_on_migrates_storage`, `key_encryption_sync_safe_failures`; Rust coverage for `key_set_encrypts_env_api_key_without_printing_secret`, `ai_prompt_uses_gpg_stored_key_when_env_key_is_missing`, `encrypt_on_migrates_plaintext_storage_and_persists_config`, `encrypt_rotate_reencrypts_existing_storage_and_persists_fingerprint`, `encrypt_off_decrypts_storage_and_persists_config`, `encrypted_writes_use_gpg_files_without_plaintext_jsonl`, `encrypted_history_append_does_not_block_command_completion`, `encrypted_completion_uses_cached_templates_without_gpg_on_keypress`, and rewrite-history planning/script safety | Mostly covered with fake GPG | Real passphrase-protected key and pinentry behavior remains human-only in `MANUAL_TESTS.md`. Async startup unlock remains a known gap. |

## Feature Coverage

### Project Foundation

Implemented:

- Rust binary crate `aish`.
- Internal module skeleton for app, config, terminal, pty, input, modes, history, AI, templates, completion, picker, editor, paste, encryption, sync, log, and shell integration.
- `anyhow`, `serde`, `serde_json`, and `toml` are configured.

Tests:

- `config::tests::default_config_matches_spec_basics`
- `config::tests::config_roundtrips_through_json_for_future_jsonl_storage`

Status:

- Passing.

### Config And First Run

Implemented:

- Default `~/.aish` layout.
- `AISH_HOME` override for tests and isolated runs.
- Paste config defaults: `multiline = "editor"`, `confirm_execute = true`.
- First-run directory creation.
- Default production home creation at `$HOME/.aish` when `AISH_HOME` is unset.
- Config persistence under `$HOME/.aish/config.toml` when `AISH_HOME` is unset.
- History persistence under `$HOME/.aish/history/regular.jsonl` when `AISH_HOME` is unset.
- Draft restore from `$HOME/.aish/history/draft.jsonl` when `AISH_HOME` is unset.
- Template persistence under `$HOME/.aish/templates/templates.jsonl` when `AISH_HOME` is unset.
- Sync config persistence under `$HOME/.aish/config.toml` when `AISH_HOME` is unset, without scheduler files.
- Event log persistence under `$HOME/.aish/logs/events.jsonl` when `AISH_HOME` is unset.
- Missing config creates default `config.toml`.
- First-run managed directories are private on Unix where supported.
- Config files are written with private file permissions on Unix where supported.
- Invalid config returns a readable error.
- Invalid `$HOME/.aish/config.toml` returns a readable error when `AISH_HOME` is unset.
- Draft config defaults: `persist = true`, `sync = false`.

Tests:

- `config::tests::default_config_matches_spec_basics`
- `config::tests::normalize_replaces_empty_values`
- `config::tests::first_run_creates_layout_and_default_config`
- `config::tests::invalid_config_has_readable_error`
- `config::tests::aish_home_environment_overrides_default_root`
- `first_run_creates_aish_home_without_user_home_side_effects`
- `first_run_doctor` expect scenario
- `home_default_first_run_doctor` expect scenario
- `home_default_config_persists` expect scenario
- `invalid_config_startup` expect scenario
- `home_default_invalid_config_startup` expect scenario

Status:

- Passing.

### PTY Backend: Bash MVP

Implemented:

- `portable-pty` backend.
- Shell resolution: configured shell, `$SHELL`, `/bin/bash` fallback.
- Bash clean startup flags: `--noprofile --norc`.
- PTY pair creation and shell spawn.
- PTY master read thread.
- PTY writes.
- Prompt/command completion marker.
- Per-command unique marker to avoid collision with user output.
- Marker parsing waits for marker plus exit status and line ending.
- Marker parsing handles echoed marker injection commands.
- PTY CRLF output normalization.
- Bash and zsh startup enable shell-native ignore-space history behavior for Aish internal integration commands.
- Ready-marker cleanup hides internal ready lines from displayed output.
- A dedicated bash PTY integration test isolates `HOME` and verifies user commands remain in shell history while Aish internal marker commands do not.
- Backend shell is killed on `PtyBackend` drop.
- Real bash shell state persistence is tested with `pwd`, `cd /tmp`, `pwd`.

Tests:

- `pty::tests::resolves_configured_shell_before_environment`
- `pty::tests::bash_launch_uses_clean_startup_flags`
- `pty::tests::non_bash_launch_does_not_receive_bash_only_flags`
- `pty::tests::parses_marker_and_hides_it_from_output`
- `pty::tests::parser_ignores_old_fixed_marker_in_user_output`
- `pty::tests::parser_normalizes_pty_newlines`
- `pty::tests::parser_uses_real_marker_when_command_echo_contains_marker`
- `pty::tests::marker_status_requires_digits_and_line_end`
- `pty::tests::clean_marker_echo_hides_ready_marker_lines`
- `pty::tests::parser_reads_ready_marker_cwd_when_status_is_present`
- `pty::tests::parse_ready_status_output_reads_status_cwd_and_filters_hook_lines`
- `pty_backend_runs_commands_and_preserves_shell_state`
- `pty_backend_captures_failed_command_exit_status`
- `pty_backend_does_not_confuse_user_output_with_prompt_marker`
- `pty_backend_keeps_user_commands_but_not_aish_internal_markers_in_history`

Status:

- Passing for bash.

### PTY Backend: Zsh Integration

Implemented:

- zsh launch preparation uses `zsh -f -o histignorespace`.
- zsh startup disables prompt/ZLE behavior needed for the PTY hook flow.
- zsh startup enables ignore-space history behavior for Aish internal integration commands.
- zsh PTY command execution preserves shell state and reports exit status/cwd through core `preexec` / `precmd` hooks.
- zsh `preexec` start markers are consumed, filtered from command output, and exposed as `CommandResult::started_command`.
- A dedicated zsh PTY integration test isolates `HOME` and verifies user commands remain in shell history while Aish internal marker commands do not.

Tests:

- `zsh_pty_backend_runs_commands_and_preserves_shell_state_when_available`
- `zsh_pty_backend_keeps_user_commands_but_not_aish_internal_markers_in_history`

Status:

- Passing.
- Remaining gap: zsh hook start events are parsed into command results, but not yet surfaced as a separate terminal event-loop notification.

### Terminal Raw Mode And Event Loop

Implemented:

- Raw mode enable/disable guard.
- Bracketed paste enable/disable guard.
- Panic cleanup hook for terminal restoration.
- Keyboard event handling.
- Paste event handling for single-line paste insertion.
- Redraw function for prompt/input line.
- Cursor placement after redraw.
- `Ctrl-D` empty draft exit action.
- Minimal private `#exit` and `#quit` exit path.

Tests:

- `terminal::tests::printable_keys_edit_draft_at_cursor`
- `terminal::tests::control_navigation_and_deletion_update_draft`
- `terminal::tests::alt_word_navigation_moves_by_tokens`
- `terminal::tests::tab_switches_mode_only_for_empty_draft`
- `terminal::tests::enter_and_empty_ctrl_d_return_actions`
- `terminal::tests::panic_cleanup_hook_can_be_installed_without_panicking`
- `terminal::tests::passthrough_mode_forwards_keys_without_interpreting_app_actions`
- `terminal::tests::passthrough_mode_forwards_navigation_escape_sequences`
- `app::tests::terminal_cursor_column_tracks_draft_cursor`
- `app::tests::private_exit_requests_app_exit`
- `tmux_ctrl_l_clears_visible_screen_and_keeps_prompt_usable`
- `tmux_ctrl_d_exits_session_without_leftover_pane`
- `tmux_exit_command_terminates_session_without_leftover_pane`

Status:

- Passing.

Known gaps:

- Command-running PTY output is exposed through explicit output/idle callbacks and real output streams before command completion.
- Timer/background support currently exists as frontend tick wakeups and encrypted-write events; future scheduled background work is not implemented.
- Raw terminal behavior is covered by expect scenarios and tmux pane-capture regressions for portable workflows; add new tmux coverage only when final-screen behavior matters.

### Core Modes

Implemented:

- Primary modes: `Draft`, `History`, `Ai`.
- Temporary modes: `CommandRunning`, `Passthrough`, `ExternalEditor`, `PasteReviewEditor`, `Picker`, `UnlockPassthrough`.
- Prompt symbols: `>`, `$`, `%`.
- Empty-input `Tab` mode cycling.
- Prompt line rendering.
- Configurable prompt templates with core variable substitution.
- Current cwd tracking from backend command completion markers.
- In-memory output ring buffer records recent command output.
- `#status` and `#doctor` report cwd when known.

Tests:

- `modes::tests::primary_modes_cycle_deterministically`
- `modes::tests::primary_mode_symbols_match_spec`
- `app::tests::empty_tab_cycles_modes`
- `app::tests::non_empty_tab_does_not_switch_modes`
- `app::tests::prompt_line_uses_current_mode_symbol`
- `app::tests::prompt_line_renders_configured_prompt_variables`
- `pty::tests::parses_marker_cwd_when_present`
- `execute_draft_updates_current_cwd_from_backend_shell`
- `app::tests::output_ring_keeps_latest_entries_up_to_capacity`
- `execute_draft_sends_command_to_backend_and_opens_blank_draft`

Status:

- Passing.

Known gaps:

- Prompt variable coverage is limited to `{user}`, `{host}`, `{cwd}`, `{basename}`, `{mode}`, and `{last_status}`.

### Draft Input Editor

Implemented:

- Editable UTF-8-safe input buffer.
- Insertion at cursor.
- String/paste insertion at cursor.
- Backspace and Delete.
- `Ctrl-A`, `Ctrl-E`.
- `Ctrl-U`, `Ctrl-K`, `Ctrl-W`.
- Left/Right.
- `Alt-B`, `Alt-F`, `Alt-Left`, `Alt-Right` through key handling.
- Cursor placement on redraw.
- Multi-line draft buffer execution.
- Draft command submission to PTY.

Tests:

- `input::tests::inserts_and_edits_in_middle`
- `input::tests::backspace_and_delete_are_utf8_safe`
- `input::tests::control_deletion_matches_readline_basics`
- `input::tests::word_navigation_skips_tokens`
- `terminal::tests::printable_keys_edit_draft_at_cursor`
- `terminal::tests::control_navigation_and_deletion_update_draft`
- `terminal::tests::alt_word_navigation_moves_by_tokens`
- `execute_draft_sends_command_to_backend_and_resets_state`
- `execute_draft_records_failed_status_and_returns_to_draft`
- `execute_draft_sends_multiline_buffer_exactly_to_backend`

Status:

- Passing.

### Private `#` Input Safety

Implemented:

- Line-leading `#` is parsed before shell execution.
- Ordinary input is sent to shell.
- Notes are recognized.
- Private commands are recognized.
- AI prompts are parsed and routed to the AI request pipeline.
- AI prompts with context pseudo-pipe syntax are parsed and can execute controlled context collection.
- Visual continuation lines for `#` prompts and `#mt` template creation can be normalized by a pure parser helper.
- Context pseudo-pipe commands execute only through the controlled context collector after config, confirmation, and danger checks.
- Unknown private commands are not sent to shell.
- Unknown private commands suggest the nearest implemented command when there is a close match.
- Minimal private commands: `#help`, `#status`, `#config`, `#doctor`, `#model`, `#base-url`, `#env-key`, `#key set`, `#key clear`, `#context`, `#completion`, `#log`, `#editor`, `#mt`, `#template find`, `#template show`, `#template use`, `#template rm`, `#template replace`, `#encrypt`, `#set-remote`, `#push`, `#sync`, `#exit`, `#quit`, `#history <count>`.
- `#help` prints private commands and the default keybinding map.
- Help output distinguishes implemented keybindings from reserved keybindings.
- `Esc` clears the draft and returns to draft mode.
- `Ctrl-R` resolves to history search without editing draft state before the picker returns a selection.
- `Ctrl-X Ctrl-E` resolves to an external-editor launch action without editing draft state.
- `Ctrl-X` advanced picker chords resolve to launch actions without editing draft state before the picker returns a selection.
- `#status` reports the default keybinding count.
- AI configuration commands `#model`, `#base-url`, and `#env-key` persist to `config.toml`; `#base-url` stores the normalized final chat-completions URL; `#key set` stores the current configured environment API key with GPG when an encryption key fingerprint is configured, and `#key clear` removes the encrypted key file.
- AI helpers normalize chat-completions URLs, read API keys from configured environment variables, build strict JSON-only chat request bodies, and parse/validate structured AI item JSON without relying on newline boundaries.
- AI session helpers persist parsed AI items to `ai.jsonl`, rebuild command indexes, and switch to `%` AI mode at the first command from the new session.
- Direct `# prompt` AI requests are wired to the configured chat-completions request path; missing config reports a readable error without crashing or mutating AI history.
- Context configuration persists `#context on|off`, `#context confirm on|off`, and `#context <bytes>` to `config.toml`; context confirmation stores a pending prompt and accepts `Y`/`Enter` or skips with `n`/`Esc`/`Ctrl-C`.
- Context pseudo-pipe helpers run context commands through a controlled `/bin/sh -c` subprocess, capture stdout and stderr, enforce byte caps and timeouts, disclose truncation/timeouts, detect dangerous command patterns, and build contextual AI prompts with common secret token shapes redacted from command/output context.
- Event log helpers append to `logs/events.jsonl`, trim to 1000 events by default, redact common secret token shapes, record config update errors, record secret/encryption-adjacent changes such as `#key clear`, record sync config changes, and `#log <count>` prints recent events.
- Sync config commands persist remote, schedule/off state, and category toggles for AI/history/templates/drafts without running git or creating scheduler files.
- Sync lock helper atomically creates a lock file, rejects a second holder, writes metadata, and removes the lock on drop.
- Managed sync `.gitignore` helper preserves user content, replaces only the Aish managed section, and is idempotent.
- Tracked managed files warning helper identifies Aish-managed paths that may already be tracked and explicitly avoids automatic `git rm --cached` behavior.
- Sync conflict/failure logging helper writes redacted error events through the event log without running git.
- Startup sync schedule decision helper conservatively detects due/skipped sync states without creating scheduler files or running git.
- Git sync step classification helper identifies conflict-like git output and distinguishes conflict aborts from ordinary failures without running git.
- Managed add plan helper selects only enabled sync category paths plus `.gitignore` for future git add operations without running git.
- Pull-rebase command plan helper emits fixed `git pull --rebase` arguments without shell interpolation or running git.
- Sync commit command plan helper emits fixed `git commit -m` arguments, sanitizes message text, and rejects empty messages without running git.
- Sync push command plan helper emits fixed `git push` arguments without shell interpolation or running git.
- Git repository initialization plan helper emits fixed `git init` and `git remote add origin <remote>` arguments while rejecting empty/control-character remotes without running git.
- Ctrl-L and real `clear`-style command output handling is covered with a virtual terminal screen test that interprets CR/LF and ANSI home/clear sequences, proving the final prompt renders on row 0 instead of leaving a blank first line.
- Conservative sync flow plan helper orders pull-rebase, managed add, commit, and push steps using fixed git argument arrays without running git.
- Manual `#push` sync is covered against a local bare git remote, including pull-rebase, managed `.gitignore` add, commit, push, and completion event logging without network access.
- Startup `#sync <cron-expression>` behavior is covered for due and not-due schedules using a runtime timestamp file, sync lock, and local bare git remote without creating scheduler files.
- Marker-based shell integration now emits and parses command-start markers, with shell-quoting tests and PTY coverage that bash reports `started_command` without leaking internal markers into history.
- Bash marker integration has PTY coverage for prompt-ready initial cwd, command-start reporting, command-finish exit status, and cwd reporting after command execution.
- Zsh hook integration has PTY coverage for `preexec` command-start reporting, `precmd` finish status, and cwd reporting after command execution when `/bin/zsh` is available.
- Fish event integration has launch/unit coverage for `fish_preexec` and `fish_prompt` setup plus opt-in PTY coverage for command-start, finish status, cwd reporting, and command-token-like output preservation when `AISH_TEST_FISH=1` is set.
- Allowlisted interactive commands can run in a foreground passthrough path with raw mode disabled; `less` has skip-safe expect coverage when available.
- Interactive passthrough command detection covers common fullscreen tools, REPLs, shells, stdin-oriented commands such as `gpg` and `cat`, basenames, shell quoting, assignments, and wrappers such as `sudo`, `env`, `command`, and `exec`.
- Alternate-screen buffer detection tracks common enter/exit CSI sequences (`?47`, `?1047`, `?1049`) and ignores unrelated terminal styling escapes.
- Passthrough prompt-return detection requires process exit and normal-screen state before Aish redraws its prompt after an interactive command.
- Shell integration rollup is covered across bash marker integration, zsh hooks, opt-in fish events, foreground passthrough for allowlisted interactive commands, and local temporary git sync integration tests.
- `#encrypt on [key]` resolves a stable GPG fingerprint, migrates managed JSONL storage to encrypted `*.jsonl.gpg` files, removes plaintext JSONL files after successful encryption, warns about Git history, and starts a serialized background encrypted writer.
- Dangerous context pseudo-pipe commands have expect coverage proving refusal skips execution and leaves the target file intact.
- Prompt redraw after ordinary command output has both a Rust virtual-screen regression and a real `tmux` pane-capture regression proving repeated final visible shell output remains above the next prompt in actual use; the tmux scripts run the Cargo-provided `CARGO_BIN_EXE_aish` binary via `AISH_BIN` so they cannot accidentally validate a stale `target/debug/aish`.
- Common real-world shell workflows have expect and backend-specific tmux coverage proving Aish passes through persistent `cd`, `mkdir`, file redirection, `cat | grep`, quoted arguments, exported environment variables, file tests, failing commands, and recovery after failure across bash and zsh by default; fish coverage is opt-in with `AISH_TEST_FISH=1`.
- Command output followed by mode-switch redraw and unique completion acceptance has expect coverage through the real binary.
- Manual `#push` sync has expect coverage against a local temporary bare git remote, including managed `.gitignore` push and no scheduler file creation.
- Manual `#push` sync failure has expect coverage with a missing local remote, including visible failure output, event-log recording, and no scheduler file creation.
- Representative private-command safe failures have expect coverage for invalid history/log/template/context/sync usage, followed by a backend command proving the session remains usable.
- Unicode command output has real `tmux` pane-capture coverage so UTF-8 behavior is checked against final visible terminal state without relying on Tcl/expect Unicode handling.
- Terminal size has expect coverage proving startup outer terminal rows/columns propagate to backend child commands via `stty size`; runtime backend resize is covered by PTY integration.
- `#key set` encrypts the current configured environment API key without printing the secret, while `#key clear` removes the encrypted key file if present and logs the action.
- `#completion` reports current completion config and persists `#completion max <count>`, `#completion inline on|off`, and `#completion tab-accept full|word`.
- Live inline ghost completion, configurable full/word `Tab` acceptance, and width-aware candidate row elision are implemented with Rust, expect, and tmux coverage.
- Completion has pure current-token detection helpers that handle first-token classification, non-first-token classification, quoted whitespace, escaped whitespace, cursor-in-line contexts, path-like tokens, and UTF-8 cursor snapping.
- Completion has a pure path completion helper that reads matching file and directory candidates, preserves directory prefixes, sorts candidates, marks directories with trailing `/`, preserves opening quotes in replacements, and handles missing directories as no matches.
- Completion has a pure first-token helper that returns body-first template candidates before newest-first history commands before PATH executables, with per-source deduplication.
- Completion has a pure non-first-token helper that returns template placeholder candidates before history arguments and path candidates, with per-source deduplication.
- Completion helpers support ignore-spaces matching and panel max-result limiting; config defaults expose `completion.max_results = 5`, `completion.ignore_spaces = true`, `completion.template_first = true`, `completion.inline = true`, and `completion.tab_accept = "full"`.
- Runtime state carries completion config and `#config`/`#status` report completion settings read-only.
- Prompt cwd rendering abbreviates the user home directory as `~` and paths inside it as `~/...`.
- Raw-terminal display writes normalize line feeds to CRLF through a terminal display writer, so multi-line shell output and UI messages return to column zero without corrupting stored command output.
- Runtime state can build completion candidates from current draft, templates, in-memory history, cwd, PATH, and completion config without mutating input or terminal UI; candidate discovery is separate from below-prompt row limiting.
- Non-empty typing with inline completion enabled shows a display-only ghost suffix plus labeled below-prompt hints; the first Tab accepts the already-visible inline suggestion. Inline-disabled mode accepts the first ranked candidate directly.
- Right at end-of-line accepts the current inline suggestion or first completion candidate according to the configured accept amount; Right inside the line keeps ordinary cursor movement.
- Completion helpers can render labeled width-aware candidate rows, compute display-only ghost suffixes, elide overflow with `...`, and return full or next-word accepted completion text/cursor without mutating input state.
- Picker helpers support shell quoting and pure result edits for insert-at-cursor, replace-current-token, append-as-argument, and replace-line actions.
- Picker command runner uses external `fzf` by default, can feed candidates to a command, capture the selected stdout line, report cancel status as no selection, and reject empty commands.
- File picker helpers collect sorted relative file/path candidates and can apply selected paths to draft with shell quoting.
- `Ctrl-X Ctrl-F` launches the file picker action, and selected file picker values replace the current token while cancel leaves the draft unchanged.
- `Ctrl-R` launches the history search action, scopes candidates by current mode, and selected commands replace the draft line without shell quoting.
- `Ctrl-X Ctrl-T` launches the template picker action, scopes candidates to newest unique template IDs with body previews, and selected templates become protected template drafts.
- `Ctrl-X Ctrl-B` launches the git branch picker action, lists branches from the current git repository, and selected branch names replace the current token with shell quoting.
- `Ctrl-X Ctrl-V` launches the environment variable picker action, lists shell-compatible environment variable names, and selected names replace the current token as raw `$NAME` references.
- Editor command resolution supports config, `$VISUAL`, `$EDITOR`, and PATH fallback candidates.
- Editor session preparation writes draft/history/AI selected content to a secure temporary file.
- Editor process runner appends the prepared file path to the resolved command and waits for exit status without reading or executing content.
- Editor read-back replaces the draft buffer with saved file content without executing it.
- Editor read-back preserves saved content as an editor draft without filtering line-leading `#` lines.
- Editor drafts submit raw shell content without Aish private `#` parsing, while ordinary typed line-leading `#` input remains protected.
- Editor drafts render as opaque prompt summaries and direct inline editing keys leave the hidden editor content unchanged.
- Multi-line paste content is normalized and stored as an opaque editor draft by default.
- `paste.multiline = "discard"` ignores multi-line paste without changing draft state.
- `paste.multiline = "execute"` creates an editor draft when confirmation is enabled and requests immediate submission only when `confirm_execute = false`.
- Phase 10 paste review is represented as opaque editor drafts rather than a separate inline paste editor.
- Single-line paste copies read-only history/AI selections to draft before inserting pasted text.
- Editor draft submission preserves multi-line backslash continuation and lets the backend shell interpret it.
- Ordinary drafts use the configured backend shell's own `-n` syntax check to detect incomplete quote input before PTY submission, preserving `echo "` and `echo '` continuation behavior without hand-parsing shell quotes in Aish.
- Ordinary drafts also detect odd trailing backslashes as shell line continuations because interactive shells continue those lines even though `bash -n` accepts the synthetic trailing newline used by syntax checks.
- Multi-line draft redraw emits CRLF line breaks in raw terminal mode, tracks the previously rendered block height, and suppresses backend `PS2`/`PROMPT2` so shell continuation prompts do not leak into executed command output.
- Ordinary and editor draft history preserve backslash continuations as one submitted command string.
- Optional shell logical splitter helper splits simple lines while preserving backslash continuations; it is not wired into default history behavior.
- Optional shell logical splitter ignores standalone comments and preserves inline `#` content.
- Optional shell logical splitter preserves single-quoted and double-quoted newlines.
- Optional shell logical splitter preserves heredoc blocks.
- Optional shell logical splitter can extract standalone note comments as note candidates while preserving default command splitting behavior.
- Optional shell logical splitter common-case coverage includes simple lines, blank lines, comments, backslash continuations, quotes, heredocs, and mixed command streams.
- Editor roundtrip helper prepares a file, runs a fake editor, and reads successful edits back into draft while preserving the original draft on editor failure.
- `Ctrl-X Ctrl-E` terminal handling resolves the editor, suspends raw mode when needed, runs the roundtrip, restores raw mode when needed, and reports success/failure.
- `editor.execute_after_save = true` runs a successfully saved editor draft immediately with raw editor-draft semantics.
- Template commands can create, find, remove, replace, show, and use JSONL-backed body-first templates. `#template list` is intentionally unsupported.
- Template placeholders support `{name}`, `{name:description}`, and `{name...}` syntax.
- Template use copies rendered content to a protected template draft and blocks execution while placeholders remain unresolved.
- Template draft editing treats unresolved placeholders as spans: outside Backspace/Delete removes the whole placeholder, while editing inside expands the draft to plain editable text.
- Encryption commands can change managed storage encryption state through GPG-backed migration/rotation/decryption; sync commands persist remote/schedule/category state and `#push` runs the conservative git flow.
- `#context` reports and persists current context configuration.
- `#config` prints read-only runtime configuration and does not create missing storage files.
- `#doctor` prints read-only diagnostics and does not create missing storage files.
- `#doctor` includes shell/integration checks for backend shell, PTY, GPG, git, fzf, editor, AI config, and storage paths.

Tests:

- `commands::tests::ordinary_input_is_not_private`
- `commands::tests::line_leading_hash_space_is_ai_prompt`
- `commands::tests::ai_prompt_with_context_command_is_detected`
- `commands::tests::incomplete_context_syntax_stays_plain_ai_prompt`
- `commands::tests::normalizes_ai_prompt_continuation_lines`
- `commands::tests::normalizes_mt_continuation_lines`
- `commands::tests::continuation_normalization_rejects_mixed_or_single_lines`
- `commands::tests::private_command_allows_no_space_after_hash`
- `commands::tests::unknown_private_command_suggestion_uses_nearest_implemented_command`
- `commands::tests::notes_are_detected_with_or_without_space_after_hash`
- `execute_draft_does_not_send_line_leading_hash_to_backend_shell`
- `editor_draft_can_send_line_leading_hash_to_shell`
- `editor_draft_sends_multiline_backslash_continuation_to_shell`
- `execute_draft_keeps_unfinished_quote_as_continuation_draft`
- `execute_draft_keeps_unfinished_single_quote_as_continuation_draft`
- `execute_draft_runs_completed_multiline_quote_after_continuation`
- `execute_draft_preserves_backslash_continuation_and_history`
- `editor_draft_preserves_backslash_continuation_in_history`
- `execute_draft_does_not_run_context_pseudo_pipe_command`
- `app::tests::private_help_prints_available_commands`
- `keybindings::tests::default_keybindings_include_common_and_advanced_bindings`
- `keybindings::tests::default_keybindings_distinguish_implemented_and_reserved_bindings`
- `terminal::tests::esc_clears_draft_and_returns_to_draft_mode`
- `terminal::tests::ctrl_r_returns_history_search_action_without_editing_draft`
- `terminal::tests::ctrl_x_prefix_resolves_editor_chord_to_launch_action`
- `terminal::tests::ctrl_x_prefix_resolves_file_picker_chord_to_launch_action`
- `terminal::tests::ctrl_x_prefix_resolves_template_picker_chord_to_launch_action`
- `terminal::tests::ctrl_x_prefix_resolves_git_branch_picker_chord_to_launch_action`
- `terminal::tests::ctrl_x_prefix_resolves_env_var_picker_chord_to_launch_action`
- `terminal::tests::run_external_editor_replaces_draft_after_success`
- `terminal::tests::run_external_editor_keeps_draft_after_editor_failure`
- `terminal::tests::run_external_editor_reports_missing_editor`
- `terminal::tests::run_external_editor_executes_after_save_when_configured`
- `terminal::tests::editor_draft_ignores_inline_editing_keys`
- `terminal::tests::normalize_paste_newlines_canonicalizes_crlf_and_cr`
- `terminal::tests::single_line_paste_inserts_into_draft`
- `terminal::tests::single_line_paste_copies_history_selection_first`
- `terminal::tests::multiline_paste_creates_opaque_editor_draft`
- `terminal::tests::multiline_paste_discard_config_ignores_content`
- `terminal::tests::multiline_paste_execute_with_confirm_creates_editor_draft`
- `terminal::tests::multiline_paste_execute_without_confirm_requests_submit`
- `app::tests::replace_draft_from_editor_text_creates_opaque_editor_draft`
- `terminal::tests::ctrl_x_prefix_cancels_on_unknown_chord_without_editing_draft`
- `app::tests::private_status_prints_mode_and_last_status`
- `app::tests::private_config_prints_read_only_runtime_config`
- `app::tests::private_doctor_prints_read_only_diagnostics`
- `app::tests::private_context_reports_current_config`
- `app::tests::private_context_commands_persist_config`
- `app::tests::private_context_rejects_invalid_usage_without_persisting`
- `app::tests::ai_prompt_with_context_waits_for_confirmation_by_default`
- `app::tests::ai_prompt_with_context_disabled_does_not_execute_command`
- `app::tests::ai_prompt_with_context_blocks_dangerous_command_even_without_confirmation`
- `app::tests::answer_context_confirmation_can_skip_pending_command`
- `context::tests::contextual_ai_prompt_redacts_common_secret_shapes`
- `context::tests::run_context_command_enforces_timeout`
- `app::tests::private_log_prints_recent_events`
- `app::tests::private_log_reports_usage_or_missing_storage`
- `app::tests::ai_config_write_errors_are_logged`
- `app::tests::context_config_write_errors_are_logged`
- `app::tests::ai_config_commands_persist_and_report_values`
- `app::tests::ai_config_commands_report_unconfigured_without_config_path`
- `ai::tests::normalize_chat_completions_url_appends_endpoint`
- `ai::tests::normalize_chat_completions_url_rejects_missing_scheme_or_empty`
- `ai::tests::build_chat_completions_body_uses_strict_json_prompt`
- `ai::tests::extract_chat_message_content_reads_first_choice_message`
- `ai::tests::extract_chat_message_content_rejects_missing_content`
- `ai::tests::parse_ai_items_accepts_command_and_template_items`
- `ai::tests::parse_ai_items_rejects_empty_or_invalid_items`
- `app::tests::store_ai_session_from_items_persists_and_selects_first_command`
- `app::tests::store_ai_session_from_items_without_commands_stays_in_draft`
- `app::tests::ai_prompt_reports_config_error_without_crashing`
- `app::tests::key_commands_report_current_state_without_secret_side_effects`
- `app::tests::key_clear_removes_stored_encrypted_key_and_logs_event`
- `app::tests::key_set_encrypts_env_api_key_without_printing_secret`
- `app::tests::ai_prompt_uses_gpg_stored_key_when_env_key_is_missing`
- `app::tests::encrypt_on_migrates_plaintext_storage_and_persists_config`
- `app::tests::encrypt_rotate_reencrypts_existing_storage_and_persists_fingerprint`
- `app::tests::encrypt_off_decrypts_storage_and_persists_config`
- `app::tests::encrypted_writes_use_gpg_files_without_plaintext_jsonl`
- `app::tests::encrypted_history_append_does_not_block_command_completion`
- `app::tests::encrypted_completion_uses_cached_templates_without_gpg_on_keypress`
- `app::tests::encrypt_rewrite_history_plan_reports_manual_confirmed_flow`
- `app::tests::encrypt_rewrite_history_run_requires_clean_git_worktree`
- `app::tests::encrypt_rewrite_history_script_keeps_decrypted_temp_outside_rewrite_tree`
- `app::tests::sync_config_commands_persist_without_running_git`
- `app::tests::sync_category_toggle_rejects_invalid_usage_without_persisting`
- `app::tests::subsystem_commands_report_current_state`
- `app::tests::private_editor_reports_resolution_without_launching_editor`
- `editor::tests::resolve_editor_prefers_config_command`
- `editor::tests::resolve_editor_uses_visual_before_editor`
- `editor::tests::resolve_editor_falls_back_to_path_candidates`
- `editor::tests::prepare_editor_file_writes_initial_text_to_secure_temp_file`
- `editor::tests::run_editor_command_appends_session_path_and_waits`
- `editor::tests::run_editor_command_returns_nonzero_status_without_reading_file`
- `editor::tests::run_editor_command_rejects_empty_argv`
- `editor::tests::read_editor_file_returns_saved_content`
- `app::tests::prepare_editor_session_writes_draft_text`
- `app::tests::prepare_editor_session_copies_history_selection_to_draft_and_file`
- `app::tests::prepare_editor_session_copies_ai_selection_to_draft_and_file`
- `app::tests::replace_draft_from_editor_session_preserves_editor_content`
- `app::tests::editor_draft_renders_as_opaque_summary`
- `app::tests::run_editor_roundtrip_replaces_draft_after_success`
- `app::tests::run_editor_roundtrip_keeps_original_draft_after_editor_failure`
- `app::tests::template_commands_report_usage_for_invalid_input`
- `app::tests::encryption_and_sync_commands_report_current_state_without_side_effects`
- `app::tests::private_history_without_count_prints_usage`
- `app::tests::private_exit_requests_app_exit`
- `app::tests::unknown_private_command_prints_suggestion`
- `completion::tests::current_token_detects_first_token_prefix`
- `completion::tests::current_token_detects_non_first_token_at_cursor`
- `completion::tests::current_token_keeps_quoted_whitespace_inside_token`
- `completion::tests::current_token_keeps_escaped_whitespace_inside_token`
- `completion::tests::current_token_handles_cursor_inside_line`
- `completion::tests::path_like_detection_covers_common_shell_path_prefixes`
- `completion::tests::cursor_is_snapped_to_previous_utf8_boundary`
- `completion::tests::complete_path_returns_sorted_matching_file_and_directory_candidates`
- `completion::tests::complete_path_uses_relative_directory_prefix`
- `completion::tests::complete_path_preserves_opening_quote_in_replacement_only`
- `completion::tests::complete_path_returns_empty_for_missing_directory`
- `completion::tests::complete_first_token_orders_templates_history_then_executables`
- `completion::tests::complete_first_token_deduplicates_each_source`
- `completion::tests::complete_non_first_token_orders_history_arguments_before_path_candidates`
- `completion::tests::complete_non_first_token_includes_history_arguments_without_path_prefix`
- `completion::tests::command_arguments_preserve_quoted_argument_spaces`
- `completion::tests::complete_first_token_can_match_while_ignoring_spaces_and_limit_results`
- `completion::tests::complete_non_first_token_applies_options_to_history_and_placeholders`
- `completion::tests::matches_completion_prefix_can_ignore_spaces`
- `completion::tests::render_completion_candidates_labels_sources_without_mutating_input`
- `completion::tests::render_completion_candidates_labels_directories_separately_from_files`
- `completion::tests::render_completion_candidates_for_width_elides_without_wrapping`
- `completion::tests::render_completion_candidates_for_width_keeps_source_label_when_possible`
- `completion::tests::ghost_completion_suffix_is_display_only_tail`
- `completion::tests::ghost_completion_suffix_works_across_completion_sources`
- `completion::tests::accept_completion_replaces_token_and_returns_new_cursor`
- `completion::tests::accept_completion_word_mode_stops_at_next_word_boundary`
- `completion::tests::accept_completion_word_mode_includes_leading_space_and_next_word`
- `completion::tests::accept_completion_word_mode_uses_full_suffix_without_boundary`
- `app::tests::completion_candidates_use_templates_before_history_for_first_token`
- `app::tests::completion_candidates_use_path_completion_for_path_like_token`
- `app::tests::completion_candidates_split_discovery_from_panel_row_limit`
- `app::tests::completion_candidates_skip_editor_drafts_and_read_only_modes`
- `terminal::tests::non_empty_tab_requests_completion_display_without_editing_draft`
- `terminal::tests::write_completion_candidates_prints_labeled_rows`
- `terminal::tests::tab_with_inline_enabled_shows_single_completion_before_accepting`
- `terminal::tests::tab_with_inline_disabled_accepts_first_completion_candidate`
- `terminal::tests::tab_shows_multiple_completion_candidates_below_prompt`
- `terminal::tests::tab_display_respects_completion_max_results`
- `terminal::tests::typed_input_shows_live_inline_completion_without_tab`
- `terminal::tests::live_inline_completion_shows_remaining_candidates_as_panel_hints`
- `terminal::tests::first_tab_accepts_live_inline_completion`
- `terminal::tests::live_inline_completion_respects_inline_disabled_config`
- `terminal::tests::redraw_renders_completion_panel_below_prompt_and_restores_cursor`
- `terminal::tests::redraw_renders_inline_completion_suffix_without_moving_cursor`
- `terminal::tests::inline_completion_suffix_elides_to_terminal_width`
- `terminal::tests::editing_after_completion_panel_clears_panel`
- `terminal::tests::tab_accept_word_mode_accepts_only_next_word_from_inline_suggestion`
- `input::tests::replace_updates_text_and_cursor_when_cursor_is_valid_boundary`
- `input::tests::replace_rejects_invalid_cursor_boundary`
- `terminal::tests::right_at_end_requests_completion_accept_without_editing_immediately`
- `terminal::tests::right_inside_line_keeps_cursor_movement_behavior`
- `terminal::tests::right_accepts_inline_completion_when_available`
- `terminal::tests::accept_first_completion_replaces_current_token`
- `picker::tests::shell_quote_leaves_safe_values_unquoted`
- `picker::tests::shell_quote_quotes_spaces_and_embedded_single_quotes`
- `picker::tests::picker_insert_at_cursor_inserts_quoted_value`
- `picker::tests::picker_replace_current_token_replaces_token_under_cursor`
- `picker::tests::picker_append_as_argument_adds_separator_when_needed`
- `picker::tests::picker_replace_line_replaces_everything`
- `picker::tests::run_picker_command_returns_selected_stdout_line`
- `picker::tests::run_picker_command_returns_none_on_cancel_status`
- `picker::tests::run_picker_command_rejects_empty_command`
- `picker::tests::default_fzf_command_uses_external_fzf`
- `picker::tests::file_picker_candidates_returns_sorted_relative_files_and_dirs`
- `app::tests::apply_picker_selection_replaces_current_token_with_quoted_value`
- `app::tests::apply_picker_selection_skips_editor_and_read_only_modes`
- `terminal::tests::ctrl_x_prefix_resolves_file_picker_chord_to_launch_action`
- `terminal::tests::apply_file_picker_result_replaces_current_token`
- `terminal::tests::apply_file_picker_result_reports_cancel_without_editing`
- `picker::tests::history_picker_candidates_follow_history_modes`
- `app::tests::history_picker_candidates_follow_current_mode_scope`
- `app::tests::replace_draft_from_history_picker_copies_raw_command_to_draft`
- `terminal::tests::ctrl_r_returns_history_search_action_without_editing_draft`
- `terminal::tests::apply_history_picker_result_replaces_draft_without_shell_quoting`
- `terminal::tests::apply_history_picker_result_reports_cancel_without_editing`
- `picker::tests::template_picker_candidates_return_newest_unique_ids`
- `app::tests::template_picker_candidates_return_newest_unique_ids`
- `app::tests::replace_draft_from_template_picker_uses_selected_template_id`
- `terminal::tests::ctrl_x_prefix_resolves_template_picker_chord_to_launch_action`
- `terminal::tests::apply_template_picker_result_copies_template_to_protected_draft`
- `terminal::tests::apply_template_picker_result_reports_cancel_without_editing`
- `picker::tests::git_branch_picker_candidates_return_sorted_branches`
- `picker::tests::git_branch_picker_candidates_return_empty_outside_repo`
- `terminal::tests::ctrl_x_prefix_resolves_git_branch_picker_chord_to_launch_action`
- `terminal::tests::apply_git_branch_picker_result_replaces_current_token`
- `terminal::tests::apply_git_branch_picker_result_reports_cancel_without_editing`
- `picker::tests::env_var_picker_candidates_keep_shell_compatible_names_sorted`
- `picker::tests::shell_env_var_reference_requires_valid_shell_name`
- `picker::tests::apply_raw_picker_result_does_not_shell_quote_value`
- `app::tests::apply_raw_picker_selection_replaces_without_shell_quoting`
- `terminal::tests::ctrl_x_prefix_resolves_env_var_picker_chord_to_launch_action`
- `terminal::tests::apply_env_var_picker_result_replaces_current_token_with_reference`
- `terminal::tests::apply_env_var_picker_result_rejects_invalid_names_without_editing`
- `terminal::tests::apply_env_var_picker_result_reports_cancel_without_editing`

Status:

- Passing.

Known gaps:

- Async encrypted startup unlock is not implemented; encrypted history/template loading is synchronous.
- Real passphrase/pinentry GPG behavior is human-only; fake-GPG command boundaries and storage migration are automated.
- Key rebinding remains incomplete.
- Future scheduled background work is not attached to tick events yet.

### Regular History Storage

Implemented:

- `HistoryEntry` JSONL format.
- `HistorySource` serialization.
- Append-only JSONL writer.
- JSONL loader.
- Bad line reporting and skipping.
- Executed commands are appended to regular history.
- Failed commands are stored with exit status.
- Executed command timestamps are stored.
- Combined regular history and AI command-item trimming with `#history <count>`.
- Startup-loaded regular history is indexed newest-first for browsing.
- Executed commands update the in-memory regular history list.

Tests:

- `history::tests::history_entry_serializes_source_as_snake_case`
- `history::tests::append_and_load_jsonl_items`
- `history::tests::missing_jsonl_file_loads_as_empty`
- `history::tests::bad_jsonl_lines_are_reported_and_skipped`
- `history::tests::rewrite_jsonl_replaces_existing_contents`
- `history::tests::trim_regular_history_keeps_newest_entries_and_skips_bad_lines`
- `history::tests::trim_combined_history_limits_regular_plus_ai_command_items`
- `execute_draft_appends_successful_command_to_regular_history`
- `execute_draft_appends_failed_command_to_regular_history`
- `private_history_command_trims_regular_and_ai_history_to_combined_limit`
- `history::tests::history_store_indexes_regular_history_newest_first`
- `app::tests::history_mode_selects_and_renders_regular_history_newest_first`
- `app::tests::selected_history_copies_to_draft_for_editing`
- `terminal::tests::history_mode_up_down_browses_without_editing_draft`
- `terminal::tests::history_mode_typing_copies_selection_to_draft_then_edits`
- `terminal::tests::history_mode_cursor_movement_does_not_copy_to_draft`
- `execute_history_selection_runs_selected_command`

Status:

- Passing.

Known gaps:

- Regular history has a newest-first in-memory browse index; search-specific indexes are not implemented.

### Draft History Storage

Implemented:

- `[draft] persist = true` default config.
- `[draft] sync = false` default config.
- `DraftEntry` JSONL format.
- `save_draft_if_configured` persists non-empty drafts when configured.
- Terminal normal exit path calls `save_draft_if_configured`.

Tests:

- `history::tests::draft_entry_roundtrips_through_json`
- `app::tests::save_draft_if_configured_persists_non_empty_draft`
- `app::tests::save_draft_if_configured_skips_empty_or_disabled_drafts`

Status:

- Passing.

Known gaps:

- Draft history browsing is implemented for saved drafts; no additional search-specific draft index exists yet.

### Note Storage

Implemented:

- `NoteTag` JSON serialization.
- `NoteEntry` JSONL format.
- `# TODO:`, `# NOTE:`, `# FIXME:`, `# HACK:`, `# XXX:` are recognized.
- Notes are stored without shell execution.

Tests:

- `commands::tests::notes_are_detected_with_or_without_space_after_hash`
- `history::tests::note_entry_serializes_tag_as_snake_case`
- `execute_draft_stores_notes_without_sending_them_to_shell`

Status:

- Passing.

### AI History Data Model

Implemented:

- `AiSession` JSONL model.
- `AiItem` model.
- `AiItemKind` model with snake_case serialization.
- Template AI items can carry `name`.
- `name = None` is omitted from JSON.
- `HistoryStore` builds a flattened AI command-item browse index in execution order.
- `%` AI mode can browse generated command items.
- AI command execution stores regular history with `source = "ai"`.
- Successful AI command execution advances to the next command in the same session.
- Failed AI command execution stays on the current command.
- Last successful AI command execution returns to draft mode.
- Editing a selected AI command copies it to draft first.
- Cursor movement in AI mode is read-only and does not copy to draft.

Tests:

- `history::tests::ai_session_roundtrips_through_jsonl`
- `history::tests::ai_item_kind_serializes_as_snake_case`
- `history::tests::history_store_indexes_ai_command_items_in_execution_order`
- `app::tests::ai_mode_selects_and_renders_command_items_in_order`
- `app::tests::selected_ai_copies_to_draft_for_editing`
- `terminal::tests::ai_mode_up_down_browses_without_editing_draft`
- `terminal::tests::ai_mode_typing_copies_selection_to_draft_then_edits`
- `terminal::tests::ai_mode_cursor_movement_does_not_copy_to_draft`
- `execute_ai_selection_success_advances_to_next_command`
- `execute_ai_selection_failure_stays_on_current_command`
- `execute_ai_selection_last_success_returns_to_draft`

Status:

- Passing.

Known gaps:

- Live network/provider behavior is intentionally not covered by automated tests.
- Stored GPG API-key fallback is fake-GPG covered; real passphrase/pinentry fallback remains human-only.

### JSONL Storage Helpers

Implemented:

- `append_jsonl`.
- `load_jsonl`.
- `rewrite_jsonl`.
- Missing file loads as empty.
- Corrupt lines are reported and skipped.
- JSONL parent directories and files are private on Unix where supported.

Tests:

- `history::tests::append_and_load_jsonl_items`
- `history::tests::missing_jsonl_file_loads_as_empty`
- `history::tests::bad_jsonl_lines_are_reported_and_skipped`
- `history::tests::rewrite_jsonl_replaces_existing_contents`

Status:

- Passing.

### Template Storage

Implemented:

- `TemplateEntry` JSONL model.
- `#mt <body>` appends a body-first template entry to `templates/templates.jsonl`.
- Template IDs are stable `tpl-...` content hashes derived from the template body.
- Old `name/body` JSONL records are still readable as body-only templates.
- `#template find <query>` prints matching template IDs and bodies.
- `#template list` is intentionally unsupported because bulk grep/redirection should happen against the JSONL store.
- `#template show <id>` prints the matching template body without changing draft.
- `#template rm <id>` removes valid template entries matching that ID.
- `#template replace <id> <body>` removes existing matches and appends one replacement entry with a new body-derived ID.
- `#template use <id>` copies the matching template body into draft without executing it.
- `#template use <id>` reports simple `{placeholder}` names found in the copied body.
- `#template use <id>` supports `{name}`, `{name:description}`, and `{name...}` placeholders.
- `#template use <id> key=value...` applies explicit placeholder substitutions before copying to draft.
- `#template use <id> key="value with spaces"` and single-quoted variants are supported.
- `#template use <id> key=value` reports unused keys that do not match any `{placeholder}`.
- `#template use <id>` reports unresolved placeholders that remain after explicit substitution.
- Template drafts with unresolved placeholders are not executed.
- Placeholder and unused-key reports are emitted in sorted order for stable output.

Tests:

- `templates::tests::template_entry_roundtrips_through_jsonl`
- `templates::tests::template_id_is_a_stable_body_hash`
- `templates::tests::old_named_template_records_load_as_body_only_templates`
- `templates::tests::find_template_by_id_returns_newest_match`
- `templates::tests::template_placeholders_returns_unique_simple_names_in_order`
- `templates::tests::template_placeholders_support_descriptions_and_variadic_markers`
- `templates::tests::template_placeholder_spans_return_valid_byte_ranges`
- `templates::tests::apply_template_values_replaces_known_placeholders_and_leaves_unknown`
- `templates::tests::apply_template_values_with_usage_reports_used_keys`
- `templates::tests::apply_template_values_replaces_described_and_variadic_placeholders_by_name`
- `app::tests::mt_command_persists_template_entry`
- `app::tests::template_list_is_intentionally_unsupported`
- `app::tests::template_find_prints_matching_hash_ids`
- `app::tests::template_show_prints_newest_matching_body`
- `templates::tests::remove_templates_by_id_removes_all_matches_and_keeps_others`
- `templates::tests::replace_template_by_id_removes_old_matches_and_appends_replacement`
- `app::tests::template_rm_removes_matching_templates`
- `app::tests::template_replace_rewrites_matching_templates`
- `app::tests::template_use_copies_newest_matching_body_to_draft`
- `app::tests::template_use_supports_quoted_values_with_spaces`
- `app::tests::template_use_supports_described_and_variadic_placeholders`
- `app::tests::unresolved_template_placeholders_do_not_execute`
- `terminal::tests::template_draft_backspace_deletes_placeholder_from_outside`
- `terminal::tests::template_draft_delete_deletes_placeholder_from_outside`
- `terminal::tests::template_draft_edit_inside_placeholder_expands_to_plain_draft`
- `app::tests::template_use_reports_missing_template_without_changing_draft`
- `app::tests::template_commands_report_usage_for_invalid_input`

Status:

- Passing.

### HistoryStore Startup Loader

Implemented:

- `HistoryStore` loads regular history, draft history, AI sessions, and notes from `DirectoryLayout` paths.
- `HistoryStore` builds a newest-first regular history index for browsing.
- `HistoryStore` builds a flattened AI command-item browse index in execution order.
- JSONL bad-line errors are aggregated across categories.
- Missing category files load as empty through the shared JSONL loader.

Tests:

- `history::tests::history_store_loads_all_history_categories`
- `history::tests::history_store_aggregates_load_errors_across_categories`
- `history::tests::history_store_indexes_regular_history_newest_first`
- `history::tests::history_store_indexes_ai_command_items_in_execution_order`
- `history::tests::split_logical_commands_splits_simple_non_empty_lines`
- `history::tests::split_logical_commands_preserves_backslash_continuations`
- `history::tests::split_logical_commands_skips_standalone_comments`
- `history::tests::split_logical_commands_can_extract_comment_only_notes`
- `history::tests::split_logical_commands_preserves_inline_hash_content`
- `history::tests::split_logical_commands_preserves_single_quoted_newlines`
- `history::tests::split_logical_commands_preserves_double_quoted_newlines`
- `history::tests::split_logical_commands_ignores_escaped_quotes`
- `history::tests::split_logical_commands_preserves_heredoc_blocks`
- `history::tests::split_logical_commands_preserves_quoted_heredoc_delimiter`

Status:

- Passing.

### Expect End-To-End Scenarios

Implemented:

- A Rust integration harness runs `expect` scenarios against the built `aish` binary with isolated `AISH_HOME` directories.
- Interactive smoke coverage now checks real terminal input/output for basic command execution, cwd persistence, mode cycling, private command safety, help output, status/config/doctor diagnostics, notes, context confirmation skip, event log output, clear screen, exit paths, completion, readline-style editing keys, unknown `Ctrl-X` chord cancellation, history execution, persisted history trimming, AI command sequencing, AI config persistence and key-source redaction, read-only edit-copy behavior, template execution, template CRUD, unresolved template blocking, external editor roundtrip, editor hash-content parser bypass, multiline paste editor-review warning/execution, key/encryption/sync safe-failure behavior, quote continuation, backslash continuation, Ctrl-C continuation cancellation, and backend prompt leak prevention.
- Each new user-facing interactive feature should now receive both Rust-level tests and at least one expect scenario when it affects real terminal behavior.

Tests:

- `expect_runner::basic_echo`
- `expect_runner::cd_persists`
- `expect_runner::ctrl_d_exits`
- `expect_runner::exit_command`
- `expect_runner::empty_tab_cycles_modes`
- `expect_runner::help_lists_commands`
- `expect_runner::unknown_private_command`
- `expect_runner::ctrl_l_clear_screen`
- `expect_runner::dquote_continuation`
- `expect_runner::squote_continuation`
- `expect_runner::backslash_continuation`
- `expect_runner::ctrl_c_cancels_continuation`
- `expect_runner::no_backend_ps2_leak`
- `expect_runner::completion_accept_single`
- `expect_runner::completion_panel_multiple`
- `expect_runner::completion_inline_off_accepts_first`
- `expect_runner::completion_tab_accept_word`
- `expect_runner::history_mode_execute`
- `expect_runner::template_use_executes`
- `expect_runner::key_encryption_sync_safe_failures`
- `expect_runner::key_clear_removes_stored_key`
- `expect_runner::status_doctor_config`
- `expect_runner::notes_are_swallowed`
- `expect_runner::template_placeholder_blocks_execution`
- `expect_runner::context_confirmation_skip`
- `expect_runner::external_editor_roundtrip`
- `expect_runner::multiline_paste_editor_review`
- `expect_runner::read_only_edit_copies_to_draft`
- `expect_runner::log_shows_context_skip`
- `expect_runner::ai_mode_executes_sequence`
- `expect_runner::ai_mode_edit_copies_to_draft`
- `expect_runner::ai_config_persists`
- `expect_runner::readline_editing_keys`
- `expect_runner::escape_clears_draft`
- `expect_runner::ctrl_x_unknown_chord_cancels`
- `expect_runner::history_trim_persists`
- `expect_runner::template_crud`
- `expect_runner::editor_hash_content_bypasses_parser`

Status:

- Passing when `expect` is installed; the harness skips scenarios if `expect` is unavailable.

## Current Ignored Tests

There are no intentionally ignored tests in the current default suite. Bash and zsh PTY and tmux coverage are active by default. Fish-specific coverage remains opt-in through `AISH_TEST_FISH=1` because cross-platform fish behavior still needs broader validation.

## Current Gaps

Important missing or partial areas:

- Full keybinding map and rebinding config.
- Async encrypted-history/template startup unlock and user-visible `history is still unlocking...` state.
- Dedicated GPG/pinentry unlock passthrough state instead of synchronous direct decrypt operations.
- Future scheduled background events beyond the current tick hook and encrypted-write completion events.
- Broader automatic passthrough detection for arbitrary alternate-screen or job-control programs.
- Fish backend validation across macOS and representative Linux distributions before it becomes default required coverage.
- Search-specific indexes beyond the current in-memory history/template completion caches.
- Live network AI provider behavior, real passphrase-protected GPG/pinentry behavior, and real remote sync authentication remain manual-only.

## Recommended Next Tests

Next high-value tests to add:

- Async encrypted unlock behavior once startup decrypt is made non-blocking.
- Real passphrase/pinentry manual harness notes for isolated GPG keys.
- Additional fish tmux workflows after cross-platform fish behavior is validated.
- Focused passthrough regressions for newly allowlisted interactive programs.
- Keybinding rebinding tests when user-configurable bindings are implemented.
