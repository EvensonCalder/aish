# Aish TODO

This implementation plan turns `SPEC.md` into a working Rust project. It is ordered to build a usable core first, then layer AI, templates, encryption, and sync on top.

## Current Completion Audit

Status as of the latest full review:

- Core interactive shell wrapper is implemented: PTY backend, raw terminal input, draft editing, continuation handling, history/AI modes, private command parsing, editor/paste flows, templates, completion, picker boundaries, AI request plumbing, context pseudo-pipe, event log, and diagnostics.
- Rust unit/integration coverage and expect-driven real terminal coverage both exist for the implemented interactive behaviors. New user-facing terminal behavior should continue to receive both Rust-level and expect-level coverage.
- Large intentionally incomplete areas remain: configurable key rebinding, GPG-backed secrets/encryption, independent PTY/timer event-loop sources, and robust automatic passthrough for arbitrary interactive commands.
- Placeholder commands for encryption/key storage are deliberately safe no-ops until Phase 18 is implemented; they should not be marked functionally complete until they perform the actual encrypted storage behavior.
- The remaining unchecked items below are the source of truth for future work; do not skip them just because adjacent scaffolding exists.

---

## Phase 0: Repository and project foundation

### Tasks

- [x] Create Rust workspace.
- [x] Add binary crate `aish`.
- [x] Add internal modules:
  - [x] `app`
  - [x] `config`
  - [x] `terminal`
  - [x] `pty`
  - [x] `input`
  - [x] `modes`
  - [x] `history`
  - [x] `ai`
  - [x] `templates`
  - [x] `completion`
  - [x] `picker`
  - [x] `editor`
  - [x] `paste`
  - [x] `encryption`
  - [x] `sync`
  - [x] `log`
  - [x] `shell_integration`
- [x] Add basic error type using `anyhow` or project-specific error enum.
- [x] Add `serde`, `serde_json`, and `toml` for config/storage.
- [x] Define `~/.aish` directory layout.
- [x] Implement first-run directory creation.
- [x] Implement config load/create/save.

### Acceptance criteria

- Running `aish` creates `~/.aish` safely.
- Missing config produces a default `config.toml`.
- Invalid config shows a readable error.
- Unit tests cover config defaults and config normalization.

---

## Phase 1: PTY backend MVP

### Tasks

- [x] Choose PTY implementation, initially `portable-pty` unless lower-level control is required.
- [x] Start backend shell:
  - [x] configured shell if set
  - [x] `$SHELL` if present
  - [x] `/bin/bash` fallback
- [x] Create PTY pair.
- [x] Spawn shell on PTY slave.
- [x] Read PTY master output asynchronously.
- [x] Write commands to PTY master.
- [x] Forward output to terminal in a basic way.
- [x] Add shell prompt marker for readiness detection.
- [x] Hide or filter the prompt marker from user output.
- [x] Detect command completion via prompt marker.

### Acceptance criteria

- `aish` can start the user's shell.
- Submitting `pwd`, `cd /tmp`, `pwd` proves shell state persists.
- Output appears in terminal.
- Aish can detect when backend shell returns to prompt.

---

## Phase 2: Terminal raw mode and event loop

### Tasks

- [x] Enable raw mode.
- [x] Enable bracketed paste mode.
- [x] Read key events.
- [x] Read paste events.
- [x] Restore terminal on panic/exit where practical.
- [ ] Implement central event loop:
  - [x] keyboard events
  - [x] paste events
  - [ ] PTY output events
  - [ ] timer/background events
- [x] Fix real-terminal backend output visibility regressions that old expect byte-stream tests missed.
- [x] Add persistent `tmux`-driven end-to-end screen-capture scripts for real terminal verification.
- [x] Add redraw function for prompt/input line.
- [x] Add `Ctrl-D` empty input exit.
- [x] Add safe terminal cleanup guard.

### Acceptance criteria

- Aish can read keys without waiting for newline.
- `Ctrl-C`, `Ctrl-D`, `Ctrl-L`, arrow keys are observable.
- Terminal state is restored after normal exit.
- Bracketed paste is detected separately from typed characters.

---

## Phase 3: Core mode state machine

### Tasks

- [x] Define primary modes:
  - [x] `Draft`
  - [x] `History`
  - [x] `Ai`
- [x] Define temporary modes:
  - [x] `CommandRunning`
  - [x] `Passthrough`
  - [x] `ExternalEditor`
  - [x] `PasteReviewEditor`
  - [x] `Picker`
  - [x] `UnlockPassthrough`
- [x] Define `AppState`:
  - [x] mode
  - [x] draft buffer
  - [x] cursor position
  - [x] selected history index
  - [x] selected AI session/item
  - [x] last exit status
  - [x] current cwd if known
  - [x] output ring buffer
- [x] Implement empty-input `Tab` mode switching.
- [x] Implement prompt symbols:
  - [x] `>` draft
  - [x] `$` history
  - [x] `%` AI
- [x] Implement customizable prompt variables.

### Acceptance criteria

- Empty `Tab` cycles between prompt modes.
- Prompt displays correct mode symbol.
- Mode transitions are deterministic and covered by unit tests.

---

## Phase 4: Draft input editor

### Tasks

- [x] Implement editable buffer with cursor.
- [x] Support insertion, deletion, and paste insertion.
- [x] Implement standard navigation:
  - [x] `Ctrl-A`
  - [x] `Ctrl-E`
  - [x] `Left/Right`
  - [x] `Alt-Left/Alt-Right`
  - [x] `Alt-B/Alt-F`
- [x] Implement deletion:
  - [x] `Backspace`
  - [x] `Delete`
  - [x] `Ctrl-W`
  - [x] `Ctrl-U`
  - [x] `Ctrl-K`
- [x] Implement redraw with cursor placement.
- [x] Implement multi-line draft buffer support for editor-returned content.
- [x] Implement interactive continuation drafts for unfinished quotes and trailing backslash continuations.
- [x] Keep continuation redraw stable in raw terminal mode and suppress backend secondary prompts from displayed command output.
- [x] Implement command submission from draft to PTY.

### Acceptance criteria

- User can edit text in the middle of a command.
- Token movement works.
- `Enter` sends the exact draft buffer to backend shell.
- Multi-line draft returned from external editor can be submitted.

---

## Phase 5: Keybinding policy

### Tasks

- [x] Define keybinding map structure.
- [x] Implement default non-conflicting keybindings.
- [x] Preserve common keys:
  - [x] `Ctrl-C`
  - [x] `Ctrl-D`
  - [x] `Ctrl-L`
  - [x] `Ctrl-A`
  - [x] `Ctrl-E`
  - [x] `Ctrl-U`
  - [x] `Ctrl-K`
  - [x] `Ctrl-W`
  - [x] `Alt-B`
  - [x] `Alt-F`
  - [x] `Alt-Left`
  - [x] `Alt-Right`
  - [x] `Ctrl-R`
  - [x] `Tab`
  - [x] `Esc`
  - [x] `Up/Down`
- [x] Implement advanced bindings:
  - [x] `Ctrl-X Ctrl-E` external editor
  - [x] `Ctrl-X Ctrl-F` file picker
  - [x] `Ctrl-X Ctrl-T` template picker
  - [x] `Ctrl-X Ctrl-B` git branch picker
  - [x] `Ctrl-X Ctrl-V` env var picker
- [ ] Add config support for user key rebinding.

### Acceptance criteria

- No default advanced key conflicts with common readline single-key bindings.
- In passthrough mode, keys are forwarded rather than interpreted.
- Keymap can be printed from `#help` or `#status`.

---

## Phase 6: History storage

### Tasks

- [x] Define JSONL formats:
  - [x] regular history entry
  - [x] draft entry
  - [x] AI session entry
  - [x] note entry
- [x] Implement append-only JSONL writer.
- [x] Implement startup loader.
- [x] Implement in-memory indexes.
- [x] Implement history trimming based on `#history <count>`.
- [x] Store exit code and timestamp for executed commands.
- [x] Store `source = ai` for executed AI commands.
- [x] Persist draft history if configured.
- [x] Implement note storage for `# TODO:` style comments.

### Acceptance criteria

- Executed commands are stored.
- Failed commands are stored with exit status.
- Draft persists when enabled.
- `#history 100` trims regular + AI command item count.
- JSONL corruption in one line does not destroy the whole history; Aish reports and skips bad lines.

---

## Phase 7: History and AI read-only browsing

### Tasks

- [x] Implement `$` history mode browsing.
- [x] Implement `%` AI mode browsing.
- [x] In history mode:
  - [x] `Up/Down` browse regular history only.
  - [x] `Enter` executes selected item.
  - [x] modification copies item to draft.
- [x] In AI mode:
  - [x] `Up/Down` browse AI items.
  - [x] `Enter` executes current item.
  - [x] success advances to next item.
  - [x] failure stays on current item.
  - [x] last success returns to draft.
  - [x] modification copies item to draft.
- [x] Ensure cursor movement does not count as modification.

### Acceptance criteria

- History and AI modes are read-only.
- Any typed character in read-only mode switches to draft with copied content.
- AI command execution follows success/failure/last-item rules exactly.
- There is no execute-all shortcut.

---

## Phase 8: `#` parser and private command dispatcher

### Tasks

- [x] Implement line-leading `#` parser.
- [x] Recognize private commands:
  - [x] `#help`
  - [x] `#status`
  - [x] `#config`
  - [x] `#doctor`
  - [x] `#model`
  - [x] `#base-url`
  - [x] `#env-key`
  - [x] `#key set`
  - [x] `#key clear`
  - [x] `#context`
  - [x] `#completion`
  - [x] `#history`
  - [x] `#log`
  - [x] `#mt`
  - [x] `#template find`
  - [x] `#template show`
  - [x] `#template use`
  - [x] `#template rm`
  - [x] `#template replace`
  - [x] `#editor`
  - [x] `#encrypt`
  - [x] `#set-remote`
  - [x] `#push`
  - [x] `#sync`
- [x] Recognize `# TODO:`, `# NOTE:`, `# FIXME:`, `# HACK:`, `# XXX:` as notes.
- [x] Recognize AI prompt `# <text>`.
- [x] Recognize pseudo-pipe context syntax `# prompt < command`.
- [x] Implement continuation parsing for `#` and `#mt`.
- [x] Unknown private command shows error and suggestions.

### Acceptance criteria

- Line-leading `#` is never accidentally sent to backend shell.
- Notes are swallowed and stored as notes.
- Private commands mutate Aish config/state only.
- AI prompts go to AI pipeline.
- Context commands obey `context.enabled` and `context.confirm`.

---

## Phase 9: External editor integration

### Tasks

- [x] Resolve editor command:
  - [x] config
  - [x] `$VISUAL`
  - [x] `$EDITOR`
  - [x] `nvim`
  - [x] `vim`
  - [x] `vi`
- [x] Implement `Ctrl-X Ctrl-E`.
- [x] Create secure temporary edit file.
- [x] Write current draft/current selected item.
- [x] Compose prepare/run/read-back helper.
- [x] Suspend Aish raw mode.
- [x] Run editor and wait.
- [x] Restore raw mode.
- [x] Read file content.
- [x] Replace draft buffer.
- [x] Do not execute by default.
- [x] Add optional `editor.execute_after_save` support.
- [x] Ensure editor draft content bypasses Aish `#` parsing when submitted.

### Acceptance criteria

- `Ctrl-X Ctrl-E` opens editor.
- Saving and quitting returns content to draft.
- `#` lines inside editor content can be sent to shell as raw content.
- Editor drafts render as summaries in the main prompt and can be reopened with `Ctrl-X Ctrl-E`.
- Default does not auto-execute.
- Optional execute-after-save works only if configured.

---

## Phase 10: Multi-line paste handling

### Tasks

- [x] Enable bracketed paste mode.
- [x] Detect single-line vs multi-line paste.
- [x] Single-line paste inserts at cursor.
- [x] Add `paste.multiline` and `paste.confirm_execute` config defaults.
- [x] Multi-line paste follows `paste.multiline`:
  - [x] `editor`
  - [x] `execute`
  - [x] `discard`
- [x] Represent paste review as opaque editor draft.
- [x] Add safe execute confirmation behavior using editor draft when configured.
- [x] Ensure multi-line paste enters opaque editor draft by default without execution.
- [x] Implement raw submission to backend shell.
- [x] Keep raw multi-line submission history faithful by default.

### Acceptance criteria

- Single-line paste works like normal insertion.
- Multi-line paste never silently executes unless user configured it.
- Multi-line paste becomes an opaque editor draft by default.
- Executed multi-line content is stored as the exact command submitted to the backend shell.

---

## Phase 11: Shell logical command splitter

This is a future configurable enhancement. It must not replace the default faithful-history behavior until it can preserve shell semantics reliably.

### Tasks

- [x] Add pure best-effort shell logical command splitter helper.
- [x] Preserve backslash continuations.
- [x] Preserve quoted multi-line strings.
- [x] Preserve heredoc blocks.
- [x] Ignore blank lines.
- [x] Ignore standalone comment lines in splitter output.
- [x] Store comment-only lines as notes if enabled.
- [x] Add tests for common cases:
  - [x] two simple lines
  - [x] backslash continuation
  - [x] quoted newline
  - [x] heredoc
  - [x] comments
  - [x] mixed commands

### Acceptance criteria

- Default history stores `cd /tmp\npwd` as one submitted command string.
- Optional splitter can make `cd /tmp\npwd` become two history commands only when enabled.
- Optional splitter keeps `echo foo \\\nbar` as one history command.
- heredoc command is not split incorrectly.
- History semantics are closer to classic shell history than editor-buffer history.

---

## Phase 12: Template system

### Tasks

- [x] Implement body-first `#mt <template-body>`.
- [x] Implement multi-line `#mt` continuation.
- [x] Store templates in `templates/templates.jsonl`.
- [x] Implement stable content-hash template IDs for exact operations.
- [x] Implement `#template find <query>`.
- [x] Keep `#template list` intentionally unsupported; users can inspect the JSONL store directly for grep/redirection workflows.
- [x] Implement `#template rm <id>`.
- [x] Parse placeholders:
  - [x] `{name}`
  - [x] `{name:description}`
  - [x] `{name...}`
- [x] Implement placeholder spans in editor buffer.
- [x] Implement whole-placeholder deletion from outside.
- [x] Implement expanded placeholder behavior when edited internally.
- [x] Block execution of unresolved placeholders in template drafts.

### Acceptance criteria

- Templates can be created manually.
- AI-suggested templates are not auto-saved.
- Template candidates appear in completion.
- Unresolved placeholders in template drafts cannot be executed.

---

## Phase 13: Completion engine

Status: implemented for v0.1.0 terminal draft completion. Candidate ranking, candidate display, and first-candidate acceptance are intentionally simple; richer interactive selection can build on the picker phase.

### Tasks

- [x] Implement weak shell lexer for current token detection.
- [x] Detect path-like token.
- [x] Implement path completion helper.
- [x] Implement first-token completion helper:
  - [x] templates
  - [x] history commands
  - [x] PATH executables
- [x] Implement non-first-token completion helper:
  - [x] structural template matches
  - [x] structural history suffixes
  - [x] paths
  - [x] history arguments
  - [x] template placeholders
- [x] Implement ignore-space matching helper.
- [x] Preserve newest-to-oldest history order in first-token helper.
- [x] Implement `completion.max_results` helper/config default.
- [x] Add completion config to runtime state and `#config` report.
- [x] Add pure candidate rendering helper for below-input display.
- [x] Implement ghost suggestion display-only helper.
- [x] Implement accept suggestion helper.
- [x] Add runtime AppState completion candidate helper.
- [x] Wire non-empty Tab to render completion candidates without editing input.
- [x] Wire accept key into terminal UI.

### Acceptance criteria

- `git sta` can suggest `git status` from history/template.
- `g s` can match `git status` when ignore-space matching is enabled.
- Path-like tokens use path completion.
- Template candidates appear before history candidates.
- Structural template matches suppress lower-priority generic placeholder/history/path fallbacks for the same completion query.
- Structural template matches use newest stored templates first.
- Template placeholders can be accepted from the typed placeholder name without requiring braces.
- Ghost suggestion never modifies buffer without explicit accept.

---

## Phase 14: Picker and fzf integration

### Tasks

- [x] Implement picker result action helpers:
  - [x] insert at cursor
  - [x] replace current token
  - [x] append as argument
  - [x] replace line
- [x] Implement shell quoting for inserted values.
- [x] Implement picker command runner boundary using external `fzf` by default.
- [x] Implement file/path picker, initially via `fzf`:
  - [x] file/path candidate collection
  - [x] apply selected path to draft
  - [x] run external `fzf` from keybinding
- [x] Implement history search picker for `Ctrl-R`.
- [x] Implement template picker.
- [x] Implement git branch picker.
- [x] Implement env var picker.
- [x] Support keybindings:
  - [x] `Ctrl-X Ctrl-F`
  - [x] `Ctrl-R`
  - [x] `Ctrl-X Ctrl-T`
  - [x] `Ctrl-X Ctrl-B`
  - [x] `Ctrl-X Ctrl-V`

Status: implemented for v0.1.0 via external `fzf`. Picker actions are intentionally simple: file, git branch, and env var pickers replace the current token; history search replaces the full draft line; template picker copies the selected template body to a protected template draft.

### Acceptance criteria

- User can insert a selected file path into current command.
- Paths with spaces are shell-quoted.
- Current token replacement works correctly.
- `Ctrl-R` searches both relevant modes according to current state.

---

## Phase 15: AI client

### Tasks

- [ ] Implement AI config:
  - [x] `#base-url`
  - [x] `#model`
  - [x] `#env-key`
  - [ ] `#key set`
- [x] Normalize final chat completions URL.
- [x] Read API key from configured environment variable.
- [x] Implement GPG-backed key storage later; initially support env key.
- [x] Build request body for chat completions-compatible endpoint.
- [x] Write strict system prompt requiring JSON only.
- [x] Discard thinking/reasoning fields if provider returns them.
- [x] Parse JSON response.
- [x] Validate `items` array.
- [x] Reject empty or invalid output with clear error.
- [x] Store AI session in `ai.jsonl`.
- [x] Switch to `%` AI mode at first generated item.

Status: direct AI prompts are wired to the chat-completions request path using configured env-key credentials. Live network behavior is not covered by automated tests; pure request/parse helpers and no-crash config-error behavior are covered.

### Acceptance criteria

- `# how do I set git global name and email?` returns AI items.
- AI result is parsed as JSON.
- AI item boundaries come from JSON items, not newlines.
- Invalid model output does not crash Aish.

---

## Phase 16: Context pseudo-pipe

### Tasks

- [x] Parse `# prompt < command`.
- [x] Implement `#context on|off`.
- [x] Implement `#context confirm on|off`.
- [x] Implement `#context <bytes>`.
- [x] Execute context command through backend shell or a controlled shell subprocess.
- [x] Capture stdout/stderr.
- [x] Apply max byte cap.
- [x] Add truncation notice.
- [x] Add dangerous command detection.
- [x] Force confirmation for dangerous context commands.
- [x] Include context in AI request.

### Acceptance criteria

- Context command output can improve AI request.
- `#context off` disables the feature.
- Confirmation works.
- Large output is capped at default 65536 bytes.
- Dangerous context commands are not executed silently.

---

## Phase 17: Event log

### Tasks

- [x] Implement `logs/events.jsonl`.
- [x] Add log writer.
- [x] Add log trimming to 1000 events.
- [x] Add `#log <count>`.
- [ ] Log:
  - [x] AI request success/failure
  - [x] context confirmation/skip
  - [x] encryption changes
  - [x] sync changes
  - [x] sync failures
  - [x] config errors
- [x] Ensure secrets are redacted.

### Acceptance criteria

- `#log 20` shows recent events.
- Logs are not synchronized.
- Logs do not contain API keys.

---

## Phase 18: GPG secrets and encryption

### Tasks

- [ ] Implement `#key set` using GPG encryption.
- [x] Implement `#key clear`.
- [ ] Implement `#encrypt on`.
- [ ] Implement `#encrypt off`.
- [ ] Encrypt:
  - [ ] regular history
  - [ ] AI history
  - [ ] draft history
  - [ ] notes
  - [ ] templates
- [ ] Encrypt template payload metadata and avoid plaintext template names, search indexes, and list indexes when encryption is enabled.
- [ ] Do not persist plaintext search indexes when encrypted.
- [ ] Decrypt asynchronously on startup.
- [ ] Show `history is still unlocking...` when needed.
- [ ] Handle GPG/pinentry by temporarily entering UnlockPassthrough.
- [x] Add atomic encrypted-write helper.
- [x] Warn about existing plaintext in git history.

### Acceptance criteria

- API key can be stored and used from GPG secret.
- Encrypted history/templates can be loaded.
- Aish remains usable while decrypting history.
- No plaintext index is written when encrypted.
- Enabling encryption prints the git-history warning.

---

## Phase 19: Git sync

### Tasks

- [x] Initialize git repository in `~/.aish` if requested.
- [x] Implement `#set-remote`.
- [x] Implement `#push` manual sync.
- [x] Implement `#sync <cron-expression>`.
- [x] Implement `#sync off`.
- [x] Implement category sync toggles:
  - [x] AI
  - [x] regular history
  - [x] templates
  - [x] drafts
- [x] Maintain managed `.gitignore` section.
- [x] Warn if files may already be tracked; do not run `git rm --cached` automatically.
- [x] Implement lock file.
- [x] Implement startup cron check.
- [x] Implement conservative sync flow:
  - [x] pull --rebase
  - [x] add managed files
  - [x] commit
  - [x] push
- [x] Abort on conflict.
- [x] Log conflict/failure.

### Acceptance criteria

- Manual `#push` works when repo and remote are configured.
- Automatic sync never runs concurrently.
- Conflicts are not auto-resolved.
- Category sync toggles affect future sync behavior only.

---

## Phase 20: Shell integration improvements

### Tasks

- [x] Replace simple prompt marker with shell-specific integration where possible.
- [x] Bash integration:
  - [x] prompt-ready marker
  - [x] command-start marker
  - [x] command-finish marker with exit code
  - [x] cwd reporting
- [x] Zsh integration:
  - [x] `precmd`
  - [x] `preexec`
  - [x] cwd reporting
  - [x] surface command-start events beyond output filtering
- [ ] Fish integration:
  - [x] prompt/event functions
  - [x] cwd reporting
  - [ ] Promote fish from opt-in experimental support only after validation across macOS and representative Linux distributions.
- [x] Detect interactive commands for passthrough:
  - [x] command allowlist
  - [x] alternate screen buffer detection
  - [x] prompt return detection
- [x] Add `#doctor` integration checks.

### Acceptance criteria

- Aish reliably knows when commands start/finish.
- Exit status is captured.
- Current directory is tracked.
- `vim`, `nvim`, `ssh`, `top`, `less`, `fzf` behave normally.

---

## Phase 21: Safety hardening

### Tasks

- [x] Block unresolved placeholders from execution.
- [x] Add context dangerous-command detection.
- [x] Add multi-line paste warnings.
- [x] Redact secrets from logs.
- [x] Redact secrets from AI context where feasible.
- [x] Ensure line-leading `#` direct input never reaches shell.
- [x] Ensure editor content bypasses Aish `#` parser intentionally.
- [x] Ensure passthrough mode does not intercept app keys.
- [x] Add terminal cleanup on panic.

### Acceptance criteria

- Safety rules from `SPEC.md` have tests.
- User cannot accidentally run unresolved `{placeholder}` commands.
- AI cannot auto-execute commands.
- Multi-line paste is never silently executed by default.

---

## Phase 22: `#doctor`, `#status`, and help UX

### Tasks

- [x] Implement `#help`.
- [x] Implement `#status`:
  - [x] mode
  - [x] shell
  - [x] model
  - [x] final AI URL
  - [x] API key source
  - [x] encryption state
  - [x] sync state
  - [x] context config
  - [x] completion config
- [x] Implement `#doctor`:
  - [x] backend shell check
  - [x] PTY check
  - [x] gpg check
  - [x] git check
  - [x] fzf check
  - [x] editor check
  - [x] AI URL/key check
  - [x] storage permissions check
- [x] Implement `#config` to open or print config path.

### Acceptance criteria

- A new user can run `#doctor` and understand setup problems.
- `#status` shows final request URL and key source without leaking the key.
- `#help` lists private commands and keybindings.

---

## Phase 23: Testing strategy

### Unit tests

- [x] Config load/save/defaults.
- [x] Prompt rendering.
- [x] Mode transitions.
- [x] `#` parser.
- [x] Continuation parser.
- [x] Context parser.
- [x] Completion matching.
- [x] Placeholder parser/editor behavior.
- [x] History trimming.
- [x] Logical command splitter.
- [x] AI JSON schema parsing.
- [x] URL normalization.
- [x] Keybinding resolution.
- [x] GPG fake command-boundary flow.

### Integration tests

- [x] PTY starts backend shell.
- [x] `cd` persists across commands.
- [x] Command exit status captured.
- [x] History mode read-only behavior.
- [x] AI mode read-only behavior.
- [x] External editor roundtrip using a fake editor script.
- [x] Multi-line paste editor-review flow.
- [x] Git sync in temporary repo.

### Expect end-to-end tests

- [x] Add an `expect` runner that launches the built `aish` binary with isolated `AISH_HOME`.
- [x] Cover basic command execution and prompt return.
- [x] Cover common shell workflows with redirection, pipes, quoting, exports, file tests, failures, and recovery.
- [x] Cover backend-specific tmux common workflows for bash and zsh by default, with fish coverage kept opt-in through `AISH_TEST_FISH=1`.
- [x] Cover persistent backend cwd with `cd /tmp` followed by `pwd`.
- [x] Cover empty `Tab` mode cycling through draft/history/AI prompts.
- [x] Cover `#help`, unknown private commands, `#exit`, and empty `Ctrl-D` exit.
- [x] Cover common readline editing keys (`Ctrl-A`, `Ctrl-E`, `Ctrl-U`, `Ctrl-K`, `Ctrl-W`, and `Esc`).
- [x] Cover unknown `Ctrl-X` chord cancellation.
- [x] Cover `#status`, `#doctor`, and `#config` diagnostics.
- [x] Cover note capture without backend shell execution.
- [x] Cover context pseudo-pipe confirmation skip flow.
- [x] Cover event log output after a context skip.
- [x] Cover `Ctrl-L` clear-screen behavior.
- [x] Cover single-candidate completion acceptance and multi-candidate completion panel display.
- [x] Cover history-mode command execution.
- [x] Cover persisted regular history trimming and post-trim browsing.
- [x] Cover read-only history edit-copy behavior.
- [x] Cover AI-mode command sequencing and read-only edit-copy behavior.
- [x] Cover AI config persistence and diagnostic key-source redaction.
- [x] Cover template creation/use/execution flow.
- [x] Cover template find/show/replace/rm CRUD flow and intentional list rejection.
- [x] Cover unresolved template placeholder execution blocking.
- [x] Cover external editor roundtrip.
- [x] Cover editor-returned line-leading `#` content bypassing Aish private command parsing.
- [x] Cover multiline paste editor-review execution.
- [x] Cover multiline paste/editor draft review warning before execution.
- [x] Cover key/encryption/sync placeholder commands as safe no-ops.
- [x] Cover `echo "` and `echo '` continuation UX.
- [x] Cover trailing backslash continuation UX.
- [x] Cover `Ctrl-C` cancellation from continuation drafts.
- [x] Cover backend `PS2`/`PROMPT2` leak prevention.
- [x] Cover terminal panic cleanup hook installation.
- [x] Cover passthrough key forwarding without Aish app-key interception.
- [x] Define and maintain an expect coverage matrix for every user-visible feature.
- [x] Add screen-level expect regressions for prompt redraw after ordinary command output.
- [x] Add expect coverage for command output followed by completion/redraw/mode switches.
- [x] Add expect coverage for sync success/failure using local temporary git remotes.
- [x] Add expect coverage for representative safe failure paths for all private commands.
- [x] Add terminal coverage for long/Unicode input workflows.
- [x] Add expect coverage for terminal resize workflows.
- [x] Add expect coverage for passthrough candidates where portable in CI (`less`, `fzf` fallback, simple TUI fixture).

### Manual tests

- [ ] Bash backend.
- [ ] Zsh backend.
- [ ] Fish backend.
- [ ] `vim`/`nvim` passthrough.
- [ ] `ssh` passthrough.
- [ ] `less`/`top` alternate screen behavior.
- [ ] `fzf` integration.
- [ ] Pinentry behavior.
- [ ] Terminal resize.
- [ ] Unicode input.
- [ ] Long command editing.

---

## Phase 24: Release milestones

### v0.1: PTY + draft execution

- PTY backend.
- Draft mode.
- Basic input editor.
- Command execution.
- Basic prompt marker.

### v0.2: modes + history

- Draft/history/AI mode state machine.
- JSONL regular/draft/AI storage.
- History mode read-only semantics.
- AI mode data model without real AI client yet.

### v0.3: `#` commands + editor + paste

- `#` dispatcher.
- Notes.
- `Ctrl-X Ctrl-E`.
- Multi-line paste editor-review.
- `#help`, `#status`, `#doctor` initial versions.

### v0.4: completion + templates + pickers

- `#mt` templates.
- Placeholder spans.
- Completion engine.
- File/history/template pickers.
- fzf integration.

### v0.5: AI client

- Chat completions-compatible client.
- JSON schema parsing.
- AI sessions.
- Context pseudo-pipe.
- AI execution success/failure navigation.

### v0.6: encryption

- GPG key storage.
- `#encrypt on/off`.
- Async decrypt.
- No plaintext indexes.

### v0.7: sync

- Git remote setup.
- Manual push.
- Cron sync.
- Category sync toggles.
- Locking and conflict-safe behavior.

### v1.0: hardening

- Bash/Zsh shell integration; Fish only after cross-platform validation.
- Robust passthrough detection.
- Safety test coverage.
- Documentation.
- Install script/package.

---

## Phase 25: Suggested internal data types

```rust
struct AppState {
    mode: Mode,
    draft: InputBuffer,
    history_cursor: Option<usize>,
    ai_cursor: Option<AiCursor>,
    last_status: Option<i32>,
    cwd: Option<PathBuf>,
    output_ring: OutputRing,
    config: Config,
}

enum Mode {
    Draft,
    History,
    Ai,
    CommandRunning { submitted: SubmittedCommand },
    Passthrough,
    ExternalEditor,
    PasteReviewEditor,
    Picker(PickerKind),
    UnlockPassthrough,
}

struct InputBuffer {
    segments: Vec<Segment>,
    cursor: Cursor,
}

enum Segment {
    Text(String),
    Placeholder {
        name: String,
        description: Option<String>,
        value: Option<String>,
        expanded: bool,
    },
}

struct HistoryEntry {
    id: String,
    t: i64,
    command: String,
    exit_code: Option<i32>,
    source: HistorySource,
}

enum HistorySource {
    User,
    Ai { session_id: String, item_index: usize },
    Editor,
    Paste,
}

struct AiSession {
    id: String,
    t: i64,
    prompt: String,
    ctx: bool,
    model: String,
    items: Vec<AiItem>,
}

struct AiItem {
    kind: AiItemKind,
    text: String,
    name: Option<String>,
}

enum AiItemKind {
    Command,
    Template,
}
```

---

## Phase 26: Documentation checklist

- [x] `README.md` with philosophy: Aish Is not a SHell.
- [x] Quickstart.
- [x] Keybindings.
- [x] `#` commands.
- [x] AI safety rules.
- [x] Editor mode.
- [x] Multi-line paste behavior.
- [x] Templates.
- [x] Encryption.
- [x] Git sync.
- [x] Shell integration notes.
- [x] Troubleshooting with `#doctor`.

---

## Phase 27: Phase 2 hardening

### Tasks

- [x] Complete PHASE2 improvements and tests.
- [x] Keep `PHASE2.md` current as the active hardening checklist.
- [x] Record every Phase 2 issue that is found.
- [x] Fix every recorded Phase 2 issue or explicitly defer it with a documented reason.
- [x] Fix stale `#completion` placeholder output after completion shipped.
- [x] Add expect-driven end-to-end regression coverage for every user-visible Phase 2 fix.
- [x] Replace weak tests with tests that prove real user workflows, safety behavior, persistence, or integration boundaries.
- [x] Keep `SPEC.md`, `TODO.md`, `TESTS.md`, `README.md`, and `PHASE2.md` aligned after every implementation change.

### Acceptance criteria

- `PHASE2.md` accurately describes remaining work, expected end-to-end coverage, and known gaps.
- The full verification set passes before Phase 2 implementation commits.
- No known Phase 2 issue remains unrecorded.
- No completed Phase 2 item lacks meaningful Rust and expect coverage where practical.

---

## Phase 28: Inline completion UX

Status: implemented. Inline completion is enabled by default and refreshes while the user types, `completion.max_results` controls only the below-prompt panel row count, and bash/zsh real-terminal coverage proves completion behavior is owned by Aish rather than the backend shell. Fish backend coverage remains opt-in with `AISH_TEST_FISH=1` until cross-platform behavior is validated across macOS and Linux distributions.

### Tasks

- [x] Add completion config fields:
  - [x] `completion.inline = true` by default.
  - [x] `completion.tab_accept = "full"` by default.
  - [x] Valid `completion.tab_accept` values are `"full"` and `"word"`.
- [x] Normalize invalid or empty completion config values without silently accepting unsupported modes.
- [x] Persist and report inline completion settings through `#completion`, `#config`, and `#status`.
- [x] Add private commands:
  - [x] `#completion inline on|off`
  - [x] `#completion tab-accept full|word`
- [x] Split completion candidate discovery from panel row limiting so `completion.max_results` controls only below-prompt row count.
- [x] Track the current inline suggestion separately from the draft buffer, cursor, history, persisted draft, and below-prompt panel state.
- [x] Render the highest-ranked completion candidate as an inline ghost suffix in dim or light text while the user types when inline completion is enabled.
- [x] Render remaining candidates as live below-prompt hints while keeping the inline suggestion as the only `Tab` acceptance target.
- [x] Ensure editing, cursor movement, mode switching, prompt redraw, and command execution clear stale inline suggestions.
- [x] Make the first `Tab` accept the already-visible inline suggestion when inline completion is enabled.
- [x] Preserve legacy first-candidate acceptance when inline completion is disabled.
- [x] Implement `completion.tab_accept = "full"` to accept the complete untyped suffix.
- [x] Implement `completion.tab_accept = "word"` to accept only through the next whitespace boundary in the untyped suffix, or the full suffix when no boundary remains.
- [x] Keep `Right` at end-of-line aligned with the configured inline accept amount; keep `Right` inside the line as ordinary cursor movement.
- [x] Render below-prompt candidate rows within the current terminal width without wrapping.
- [x] Use the user's current command text as the overlap anchor for panel rows, show as much untyped candidate text as possible, and elide right-edge overflow with ASCII `...`.

### Required tests

- [x] Config default, roundtrip, normalization, and invalid-value tests for `completion.inline` and `completion.tab_accept`.
- [x] Private-command tests proving `#completion inline on|off` and `#completion tab-accept full|word` persist, report, and reject invalid input without changing config.
- [x] Pure completion tests for computing an inline suffix from history, templates, executables, paths, and non-first-token arguments.
- [x] Pure and terminal tests for structural template completion where typing a placeholder name without braces accepts the raw `{placeholder}` form.
- [x] Pure acceptance tests for full-suggestion and word-boundary acceptance, including quoted arguments and candidates with spaces.
- [x] Terminal rendering tests proving the inline ghost is display-only, refreshes while typing, uses subdued styling, does not move the real cursor, and does not mutate draft text.
- [x] Terminal state tests proving stale inline suggestions clear after editing, cursor movement, mode changes, command execution, and no-match completion.
- [x] Panel rendering tests for `completion.max_results`, narrow terminal widths, overlap anchoring, source labels, no wrapping, and `...` elision.
- [x] Expect scenarios for live inline visibility, disabled legacy mode, `Tab` full accept, `Tab` word accept, `Right` accept at end-of-line, and `Right` cursor movement inside a line.
- [x] Tmux screen-capture tests for narrow-width panel elision and no-wrap behavior in a real terminal.
- [x] Backend independence coverage for bash and zsh by default, plus opt-in fish coverage after cross-platform validation, proving inline completion behavior is owned by Aish and not by backend-shell completion.

### Acceptance criteria

- Inline suggestions behave like fish-style ghost text: visible while typing, clear enough to guide the user, but never part of the command until accepted.
- `completion.max_results` controls only the below-prompt panel row count.
- `Tab` acceptance is predictable and configurable between full-suggestion and next-word behavior.
- The below-prompt panel remains readable in narrow terminals and never wraps candidate rows.
- The feature passes pure Rust, expect, and tmux coverage before any Phase 28 checklist item is marked complete.
