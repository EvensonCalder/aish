# Aish Test Summary

This document records the current implementation status, the tests that cover each feature, and the latest verified test commands.

Last full verification performed during development:

```text
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Current test inventory:

- 102 library unit tests.
- 14 draft execution integration tests.
- 1 first-run integration test.
- 3 active bash PTY integration tests.
- 1 ignored zsh PTY integration test.
- 0 doctests.

Current expected result:

- All active tests pass.
- `zsh_pty_backend_runs_commands_and_preserves_shell_state_when_available` is intentionally ignored until shell-specific zsh prompt/echo integration is implemented.

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
cargo test -- --list
```

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
- First-run directory creation.
- Missing config creates default `config.toml`.
- Invalid config returns a readable error.
- Draft config defaults: `persist = true`, `sync = false`.

Tests:

- `config::tests::default_config_matches_spec_basics`
- `config::tests::normalize_replaces_empty_values`
- `config::tests::first_run_creates_layout_and_default_config`
- `config::tests::invalid_config_has_readable_error`
- `config::tests::aish_home_environment_overrides_default_root`
- `first_run_creates_aish_home_without_user_home_side_effects`

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
- `pty_backend_runs_commands_and_preserves_shell_state`
- `pty_backend_captures_failed_command_exit_status`
- `pty_backend_does_not_confuse_user_output_with_prompt_marker`

Status:

- Passing for bash.

### PTY Backend: Zsh Preparation

Implemented:

- zsh launch preparation uses `zsh -f`.
- zsh init attempts to disable prompt/ZLE behavior for future support.
- A real zsh PTY integration test exists but is ignored.

Tests:

- `zsh_pty_backend_runs_commands_and_preserves_shell_state_when_available`

Status:

- Ignored by design.
- Reason: zsh still needs shell-specific prompt/echo integration beyond bash v0.1.

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
- `app::tests::terminal_cursor_column_tracks_draft_cursor`
- `app::tests::private_exit_requests_app_exit`

Status:

- Passing.

Known gaps:

- PTY output is not yet integrated as a separate event-loop source.
- Timer/background events are not implemented.
- Binary-level raw terminal smoke test was attempted with `expectrl` but was not stable enough to keep. Current coverage is unit/integration level rather than full interactive terminal automation.

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
- `execute_draft_sends_command_to_backend_and_resets_state`

Status:

- Passing.

Known gaps:

- Selected history index is implemented for regular history mode only.
- Selected AI session/item state is not implemented.
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
- AI prompts are recognized as placeholders but not sent to AI yet.
- AI prompts with context pseudo-pipe syntax are recognized as placeholders.
- Visual continuation lines for `#` prompts and `#mt` template creation can be normalized by a pure parser helper.
- Context pseudo-pipe commands are not executed yet.
- Unknown private commands are not sent to shell.
- Unknown private commands suggest the nearest implemented command when there is a close match.
- Minimal private commands: `#help`, `#status`, `#config`, `#doctor`, `#model`, `#base-url`, `#env-key`, `#key set`, `#key clear`, `#context`, `#completion`, `#log`, `#editor`, `#mt`, `#template list`, `#template rm`, `#encrypt`, `#set-remote`, `#push`, `#sync`, `#exit`, `#quit`, `#history <count>`.
- `#help` prints private commands and the default keybinding map.
- Help output distinguishes implemented keybindings from reserved keybindings.
- `Esc` clears the draft and returns to draft mode.
- `Ctrl-R` returns a safe history-search placeholder action without editing draft state.
- `#status` reports the default keybinding count.
- AI configuration commands are recognized as placeholders but do not persist config or read secrets yet.
- Key commands are recognized as placeholders but do not store, read, or remove secrets yet.
- Completion, log, and editor commands are recognized as placeholders but do not activate those subsystems yet.
- Template commands are recognized as placeholders but do not read or write template storage yet.
- Encryption and sync commands are recognized as placeholders but do not change files, encryption state, remotes, or run git commands yet.
- `#context` reports that context collection is currently disabled/not implemented.
- `#config` prints read-only runtime configuration and does not create missing storage files.
- `#doctor` prints read-only diagnostics and does not create missing storage files.

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
- `execute_draft_does_not_run_context_pseudo_pipe_command`
- `app::tests::private_help_prints_available_commands`
- `keybindings::tests::default_keybindings_include_common_and_advanced_bindings`
- `keybindings::tests::default_keybindings_distinguish_implemented_and_reserved_bindings`
- `terminal::tests::esc_clears_draft_and_returns_to_draft_mode`
- `terminal::tests::ctrl_r_returns_history_search_placeholder_without_editing_draft`
- `app::tests::private_status_prints_mode_and_last_status`
- `app::tests::private_config_prints_read_only_runtime_config`
- `app::tests::private_doctor_prints_read_only_diagnostics`
- `app::tests::private_context_reports_disabled_placeholder`
- `app::tests::ai_config_commands_report_placeholders_without_persisting`
- `app::tests::key_commands_report_placeholders_without_secret_side_effects`
- `app::tests::subsystem_commands_report_placeholders`
- `app::tests::template_commands_report_placeholders_without_storage_side_effects`
- `app::tests::encryption_and_sync_commands_report_placeholders_without_side_effects`
- `app::tests::private_history_without_count_prints_usage`
- `app::tests::private_exit_requests_app_exit`
- `app::tests::unknown_private_command_prints_suggestion`

Status:

- Passing.

Known gaps:

- Most private commands from `SPEC.md` are still not implemented.
- Continuation parsing is pure parser logic only; terminal multiline collection is not wired yet.
- Context pseudo-pipe execution is not implemented.

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
- AI command source persistence is not implemented because AI execution is not implemented.

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

- Startup loading into a structured in-memory store is not implemented yet.
- Draft browsing behavior is not implemented yet.

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

- No AI client yet.
- No chat completions API usage yet.
- No AI prompt execution pipeline yet.
- AI browsing currently depends on pre-existing stored AI sessions; prompt-to-session generation is not implemented yet.

### JSONL Storage Helpers

Implemented:

- `append_jsonl`.
- `load_jsonl`.
- `rewrite_jsonl`.
- Missing file loads as empty.
- Corrupt lines are reported and skipped.

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
- `#mt <name> <body>` appends a template entry to `templates/templates.jsonl`.
- `#template list` reads template entries and prints template names.
- `#template show <name>` prints the newest matching template body without changing draft.
- `#template rm <name>` removes all valid template entries matching a name.
- `#template replace <name> <body>` removes existing matches and appends one replacement entry.
- `#template use <name>` copies the newest matching template body into draft without executing it.
- `#template use <name>` reports simple `{placeholder}` names found in the copied body.
- `#template use <name> key=value...` applies explicit placeholder substitutions before copying to draft.
- `#template use <name> key="value with spaces"` and single-quoted variants are supported.
- `#template use <name> key=value` reports unused keys that do not match any `{placeholder}`.
- `#template use <name>` reports unresolved placeholders that remain after explicit substitution.
- Placeholder and unused-key reports are emitted in sorted order for stable output.

Tests:

- `templates::tests::template_entry_roundtrips_through_jsonl`
- `templates::tests::find_template_by_name_returns_newest_match`
- `templates::tests::template_placeholders_returns_unique_simple_names_in_order`
- `templates::tests::apply_template_values_replaces_known_placeholders_and_leaves_unknown`
- `templates::tests::apply_template_values_with_usage_reports_used_keys`
- `app::tests::mt_command_persists_template_entry`
- `app::tests::template_list_prints_stored_template_names`
- `app::tests::template_show_prints_newest_matching_body`
- `templates::tests::remove_templates_by_name_removes_all_matches_and_keeps_others`
- `templates::tests::replace_template_removes_old_matches_and_appends_replacement`
- `app::tests::template_rm_removes_matching_templates`
- `app::tests::template_replace_rewrites_matching_templates`
- `app::tests::template_use_copies_newest_matching_body_to_draft`
- `app::tests::template_use_supports_quoted_values_with_spaces`
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

Status:

- Passing.

## Current Ignored Test

Ignored test:

- `zsh_pty_backend_runs_commands_and_preserves_shell_state_when_available`

Reason:

- zsh emits prompt/ZLE/control-sequence behavior that needs shell-specific integration. The project currently targets bash v0.1 behavior first while preserving a test scaffold for zsh.

How to run ignored tests manually:

```text
cargo test --test pty_backend -- --ignored --nocapture
```

Expected current result:

- The zsh ignored test is not expected to pass yet.

## Current Gaps

Important missing or partial areas:

- Full terminal event loop integration for concurrent PTY output.
- Full keybinding map and rebinding config.
- Full history browsing UX beyond regular Up/Down/Enter/edit-copy foundation.
- Full AI browsing UX beyond Up/Down/Enter/edit-copy foundation.
- AI client and chat completions parsing.
- Context pseudo-pipe.
- External editor integration.
- Multi-line paste review editor.
- Template picker UI and placeholder expansion UX.
- Completion engine.
- Pickers/fzf integration.
- Event log.
- Encryption.
- Git sync.
- Shell-specific zsh/fish integration.

## Recommended Next Tests

Next high-value tests to add:

- `HistoryStore` startup loader tests for regular, draft, AI, and note JSONL files.
- In-memory index tests for newest-to-oldest regular history ordering.
- Read-only history mode transition tests.
- AI item flattening tests from `AiSession` into browsable command candidates.
- More terminal-level tests around paste behavior and redraw with multi-line drafts.
