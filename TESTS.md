# Aish Test Summary

This document records the current implementation status, the tests that cover each feature, and the latest verified test commands.

Last full verification performed during development:

```text
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Current test inventory:

- 297 library unit tests.
- 23 draft execution integration tests.
- 1 first-run integration test.
- 6 active bash PTY integration tests.
- 2 active zsh PTY integration tests.
- 37 expect-driven end-to-end interactive scenarios.
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
- Paste config defaults: `multiline = "editor"`, `confirm_execute = true`.
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
- `Ctrl-R` resolves to history search without editing draft state before the picker returns a selection.
- `Ctrl-X Ctrl-E` resolves to an external-editor launch action without editing draft state.
- `Ctrl-X` advanced picker chords resolve to launch actions without editing draft state before the picker returns a selection.
- `#status` reports the default keybinding count.
- AI configuration commands `#model`, `#base-url`, and `#env-key` persist to `config.toml`; `#base-url` stores the normalized final chat-completions URL; `#key` commands remain placeholders for Phase 18 and do not store, read, or remove secrets yet.
- AI helpers normalize chat-completions URLs, read API keys from configured environment variables, build strict JSON-only chat request bodies, and parse/validate structured AI item JSON without relying on newline boundaries.
- AI session helpers persist parsed AI items to `ai.jsonl`, rebuild command indexes, and switch to `%` AI mode at the first command from the new session.
- Direct `# prompt` AI requests are wired to the configured chat-completions request path; missing config reports a readable error without crashing or mutating AI history.
- Context configuration persists `#context on|off`, `#context confirm on|off`, and `#context <bytes>` to `config.toml`; context confirmation stores a pending prompt and accepts `Y`/`Enter` or skips with `n`/`Esc`/`Ctrl-C`.
- Context pseudo-pipe helpers run context commands through a controlled `/bin/sh -c` subprocess, capture stdout and stderr, cap output by configured byte limit, disclose truncation, detect dangerous command patterns, and build contextual AI prompts with common secret token shapes redacted from command/output context.
- Event log helpers append to `logs/events.jsonl`, trim to 1000 events by default, redact common secret token shapes, record config update errors, record secret/encryption-adjacent changes such as `#key clear`, record sync config changes, and `#log <count>` prints recent events.
- Sync config commands persist remote, schedule/off state, and category toggles for AI/history/templates/drafts without running git or creating scheduler files.
- Sync lock helper atomically creates a lock file, rejects a second holder, writes metadata, and removes the lock on drop.
- Managed sync `.gitignore` helper preserves user content, replaces only the Aish managed section, and is idempotent.
- Tracked managed files warning helper identifies Aish-managed paths that may already be tracked and explicitly avoids automatic `git rm --cached` behavior.
- Sync conflict/failure logging helper writes redacted error events through the event log without running git.
- Startup sync schedule decision helper conservatively detects due/skipped sync states without creating scheduler files or running git.
- `#key set` remains a placeholder, while `#key clear` removes the encrypted key file if present and logs the action without printing stored secret content.
- `#completion` remains a private-command placeholder, but the internal completion engine is active for draft completion display and acceptance.
- Completion has pure current-token detection helpers that handle first-token classification, non-first-token classification, quoted whitespace, escaped whitespace, cursor-in-line contexts, path-like tokens, and UTF-8 cursor snapping.
- Completion has a pure path completion helper that reads matching file and directory candidates, preserves directory prefixes, sorts candidates, marks directories with trailing `/`, preserves opening quotes in replacements, and handles missing directories as no matches.
- Completion has a pure first-token helper that returns template candidates before newest-first history commands before PATH executables, with per-source deduplication.
- Completion has a pure non-first-token helper that returns path candidates, history argument candidates, and template placeholder candidates in spec order with per-source deduplication.
- Completion helpers support ignore-spaces matching and max-result limiting; config defaults expose `completion.max_results = 5`, `completion.ignore_spaces = true`, and `completion.template_first = true`.
- Runtime state carries completion config and `#config` reports completion settings read-only.
- Prompt cwd rendering abbreviates the user home directory as `~` and paths inside it as `~/...`.
- Raw-terminal display writes normalize line feeds to CRLF through a terminal display writer, so multi-line shell output and UI messages return to column zero without corrupting stored command output.
- Runtime state can build completion candidates from current draft, templates, in-memory history, cwd, PATH, and completion config without mutating input or terminal UI.
- Non-empty Tab accepts the single completion candidate immediately, stores zero/multiple candidates in a refreshable panel below the prompt, redraws the prompt with the cursor restored to the input line, and terminal completion display prints labeled candidate rows.
- Right at end-of-line accepts the first completion candidate; Right inside the line keeps ordinary cursor movement.
- Completion helpers can render labeled candidate rows, compute display-only ghost suffixes, and return accepted completion text/cursor without mutating input state.
- Picker helpers support shell quoting and pure result edits for insert-at-cursor, replace-current-token, append-as-argument, and replace-line actions.
- Picker command runner uses external `fzf` by default, can feed candidates to a command, capture the selected stdout line, report cancel status as no selection, and reject empty commands.
- File picker helpers collect sorted relative file/path candidates and can apply selected paths to draft with shell quoting.
- `Ctrl-X Ctrl-F` launches the file picker action, and selected file picker values replace the current token while cancel leaves the draft unchanged.
- `Ctrl-R` launches the history search action, scopes candidates by current mode, and selected commands replace the draft line without shell quoting.
- `Ctrl-X Ctrl-T` launches the template picker action, scopes candidates to newest unique template names, and selected templates become protected template drafts.
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
- Template commands can create, list, remove, replace, show, and use JSONL-backed templates.
- Template placeholders support `{name}`, `{name:description}`, and `{name...}` syntax.
- Template use copies rendered content to a protected template draft and blocks execution while placeholders remain unresolved.
- Template draft editing treats unresolved placeholders as spans: outside Backspace/Delete removes the whole placeholder, while editing inside expands the draft to plain editable text.
- Encryption and sync commands are recognized as placeholders but do not change files, encryption state, remotes, or run git commands yet.
- `#context` reports that context collection is currently disabled/not implemented.
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
- `terminal::tests::ctrl_r_returns_history_search_placeholder_without_editing_draft`
- `terminal::tests::ctrl_x_prefix_resolves_editor_chord_to_launch_action`
- `terminal::tests::ctrl_x_prefix_resolves_other_advanced_chords_to_placeholders`
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
- `app::tests::key_commands_report_placeholders_without_secret_side_effects`
- `app::tests::key_clear_removes_stored_encrypted_key_and_logs_event`
- `app::tests::sync_config_commands_persist_without_running_git`
- `app::tests::sync_category_toggle_rejects_invalid_usage_without_persisting`
- `app::tests::subsystem_commands_report_placeholders`
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
- `app::tests::template_commands_report_placeholders_without_storage_side_effects`
- `app::tests::encryption_and_sync_commands_report_placeholders_without_side_effects`
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
- `completion::tests::ghost_completion_suffix_is_display_only_tail`
- `completion::tests::accept_completion_replaces_token_and_returns_new_cursor`
- `app::tests::completion_candidates_use_templates_before_history_for_first_token`
- `app::tests::completion_candidates_use_path_completion_for_path_like_token`
- `app::tests::completion_candidates_skip_editor_drafts_and_read_only_modes`
- `terminal::tests::non_empty_tab_requests_completion_display_without_editing_draft`
- `terminal::tests::write_completion_candidates_prints_labeled_rows`
- `terminal::tests::tab_shows_multiple_completion_candidates_below_prompt`
- `terminal::tests::tab_display_respects_completion_max_results`
- `terminal::tests::redraw_renders_completion_panel_below_prompt_and_restores_cursor`
- `terminal::tests::editing_after_completion_panel_clears_panel`
- `input::tests::replace_updates_text_and_cursor_when_cursor_is_valid_boundary`
- `input::tests::replace_rejects_invalid_cursor_boundary`
- `terminal::tests::right_at_end_requests_completion_accept_without_editing_immediately`
- `terminal::tests::right_inside_line_keeps_cursor_movement_behavior`
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
- `picker::tests::template_picker_candidates_return_newest_unique_names`
- `app::tests::template_picker_candidates_return_newest_unique_names`
- `app::tests::replace_draft_from_template_picker_uses_newest_template_body`
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
- `#template use <name>` supports `{name}`, `{name:description}`, and `{name...}` placeholders.
- `#template use <name> key=value...` applies explicit placeholder substitutions before copying to draft.
- `#template use <name> key="value with spaces"` and single-quoted variants are supported.
- `#template use <name> key=value` reports unused keys that do not match any `{placeholder}`.
- `#template use <name>` reports unresolved placeholders that remain after explicit substitution.
- Template drafts with unresolved placeholders are not executed.
- Placeholder and unused-key reports are emitted in sorted order for stable output.

Tests:

- `templates::tests::template_entry_roundtrips_through_jsonl`
- `templates::tests::find_template_by_name_returns_newest_match`
- `templates::tests::template_placeholders_returns_unique_simple_names_in_order`
- `templates::tests::template_placeholders_support_descriptions_and_variadic_markers`
- `templates::tests::template_placeholder_spans_return_valid_byte_ranges`
- `templates::tests::apply_template_values_replaces_known_placeholders_and_leaves_unknown`
- `templates::tests::apply_template_values_with_usage_reports_used_keys`
- `templates::tests::apply_template_values_replaces_described_and_variadic_placeholders_by_name`
- `app::tests::mt_command_persists_template_entry`
- `app::tests::template_list_prints_stored_template_names`
- `app::tests::template_show_prints_newest_matching_body`
- `templates::tests::remove_templates_by_name_removes_all_matches_and_keeps_others`
- `templates::tests::replace_template_removes_old_matches_and_appends_replacement`
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
- Interactive smoke coverage now checks real terminal input/output for basic command execution, cwd persistence, mode cycling, private command safety, help output, status/config/doctor diagnostics, notes, context confirmation skip, event log output, clear screen, exit paths, completion, readline-style editing keys, unknown `Ctrl-X` chord cancellation, history execution, persisted history trimming, AI command sequencing, AI config persistence and key-source redaction, read-only edit-copy behavior, template execution, template CRUD, unresolved template blocking, external editor roundtrip, editor hash-content parser bypass, multiline paste editor-review warning/execution, key/encryption/sync no-op safety, quote continuation, backslash continuation, Ctrl-C continuation cancellation, and backend prompt leak prevention.
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
- `expect_runner::history_mode_execute`
- `expect_runner::template_use_executes`
- `expect_runner::key_and_sync_placeholders`
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
