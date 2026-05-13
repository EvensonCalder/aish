# Aish TODO

This implementation plan turns `SPEC.md` into a working Rust project. It is ordered to build a usable core first, then layer AI, templates, encryption, and sync on top.

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
- [ ] Implement advanced bindings:
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

- [ ] Define JSONL formats:
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
- [ ] In history mode:
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
- [ ] Recognize private commands:
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
  - [x] `#template list`
  - [x] `#template rm`
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
- [ ] Store comment-only lines as notes if enabled.
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

- [x] Implement `#mt <name> <template>`.
- [x] Implement multi-line `#mt` continuation.
- [x] Store templates in `templates/templates.jsonl`.
- [x] Implement `#template list`.
- [x] Implement `#template rm <name>`.
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
  - [ ] encryption changes
  - [ ] sync changes
  - [ ] sync failures
  - [ ] config errors
- [x] Ensure secrets are redacted.

### Acceptance criteria

- `#log 20` shows recent events.
- Logs are not synchronized.
- Logs do not contain API keys.

---

## Phase 18: GPG secrets and encryption

### Tasks

- [ ] Implement `#key set` using GPG encryption.
- [ ] Implement `#key clear`.
- [ ] Implement `#encrypt on`.
- [ ] Implement `#encrypt off`.
- [ ] Encrypt:
  - [ ] regular history
  - [ ] AI history
  - [ ] draft history
  - [ ] notes
  - [ ] templates
- [ ] Do not persist plaintext search indexes when encrypted.
- [ ] Decrypt asynchronously on startup.
- [ ] Show `history is still unlocking...` when needed.
- [ ] Handle GPG/pinentry by temporarily entering UnlockPassthrough.
- [ ] Use atomic writes.
- [ ] Warn about existing plaintext in git history.

### Acceptance criteria

- API key can be stored and used from GPG secret.
- Encrypted history/templates can be loaded.
- Aish remains usable while decrypting history.
- No plaintext index is written when encrypted.
- Enabling encryption prints the git-history warning.

---

## Phase 19: Git sync

### Tasks

- [ ] Initialize git repository in `~/.aish` if requested.
- [ ] Implement `#set-remote`.
- [ ] Implement `#push` manual sync.
- [ ] Implement `#sync <cron-expression>`.
- [ ] Implement `#sync off`.
- [ ] Implement category sync toggles:
  - [ ] AI
  - [ ] regular history
  - [ ] templates
  - [ ] drafts
- [ ] Maintain managed `.gitignore` section.
- [ ] Warn if files may already be tracked; do not run `git rm --cached` automatically.
- [ ] Implement lock file.
- [ ] Implement startup cron check.
- [ ] Implement conservative sync flow:
  - [ ] pull --rebase
  - [ ] add managed files
  - [ ] commit
  - [ ] push
- [ ] Abort on conflict.
- [ ] Log conflict/failure.

### Acceptance criteria

- Manual `#push` works when repo and remote are configured.
- Automatic sync never runs concurrently.
- Conflicts are not auto-resolved.
- Category sync toggles affect future sync behavior only.

---

## Phase 20: Shell integration improvements

### Tasks

- [ ] Replace simple prompt marker with shell-specific integration where possible.
- [ ] Bash integration:
  - [ ] prompt-ready marker
  - [ ] command-start marker
  - [ ] command-finish marker with exit code
  - [ ] cwd reporting
- [ ] Zsh integration:
  - [x] `precmd`
  - [x] `preexec`
  - [x] cwd reporting
  - [ ] surface command-start events beyond output filtering
- [ ] Fish integration:
  - [ ] prompt/event functions
  - [ ] cwd reporting
- [ ] Detect interactive commands for passthrough:
  - [ ] command allowlist
  - [ ] alternate screen buffer detection
  - [ ] prompt return detection
- [ ] Add `#doctor` integration checks.

### Acceptance criteria

- Aish reliably knows when commands start/finish.
- Exit status is captured.
- Current directory is tracked.
- `vim`, `nvim`, `ssh`, `top`, `less`, `fzf` behave normally.

---

## Phase 21: Safety hardening

### Tasks

- [ ] Block unresolved placeholders from execution.
- [ ] Add context dangerous-command detection.
- [ ] Add multi-line paste warnings.
- [ ] Redact secrets from logs.
- [ ] Redact secrets from AI context where feasible.
- [ ] Ensure line-leading `#` direct input never reaches shell.
- [ ] Ensure editor content bypasses Aish `#` parser intentionally.
- [ ] Ensure passthrough mode does not intercept app keys.
- [ ] Add terminal cleanup on panic.

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

### Integration tests

- [x] PTY starts backend shell.
- [x] `cd` persists across commands.
- [x] Command exit status captured.
- [x] History mode read-only behavior.
- [x] AI mode read-only behavior.
- [x] External editor roundtrip using a fake editor script.
- [x] Multi-line paste editor-review flow.
- [ ] GPG fake command or test key flow.
- [ ] Git sync in temporary repo.

### Expect end-to-end tests

- [x] Add an `expect` runner that launches the built `aish` binary with isolated `AISH_HOME`.
- [x] Cover basic command execution and prompt return.
- [x] Cover persistent backend cwd with `cd /tmp` followed by `pwd`.
- [x] Cover empty `Tab` mode cycling through draft/history/AI prompts.
- [x] Cover `#help`, unknown private commands, `#exit`, and empty `Ctrl-D` exit.
- [x] Cover common readline editing keys (`Ctrl-A`, `Ctrl-E`, `Ctrl-U`, `Ctrl-K`, `Ctrl-W`, and `Esc`).
- [x] Cover `#status`, `#doctor`, and `#config` diagnostics.
- [x] Cover note capture without backend shell execution.
- [x] Cover context pseudo-pipe confirmation skip flow.
- [x] Cover event log output after a context skip.
- [x] Cover `Ctrl-L` clear-screen behavior.
- [x] Cover single-candidate completion acceptance and multi-candidate completion panel display.
- [x] Cover history-mode command execution.
- [x] Cover read-only history edit-copy behavior.
- [x] Cover AI-mode command sequencing and read-only edit-copy behavior.
- [x] Cover AI config persistence and diagnostic key-source redaction.
- [x] Cover template creation/use/execution flow.
- [x] Cover unresolved template placeholder execution blocking.
- [x] Cover external editor roundtrip.
- [x] Cover multiline paste editor-review execution.
- [x] Cover key/encryption/sync placeholder commands as safe no-ops.
- [x] Cover `echo "` and `echo '` continuation UX.
- [x] Cover trailing backslash continuation UX.
- [x] Cover `Ctrl-C` cancellation from continuation drafts.
- [x] Cover backend `PS2`/`PROMPT2` leak prevention.

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

- Bash/Zsh/Fish shell integration.
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

- [ ] `README.md` with philosophy: Aish Is not a SHell.
- [ ] Quickstart.
- [ ] Keybindings.
- [ ] `#` commands.
- [ ] AI safety rules.
- [ ] Editor mode.
- [ ] Multi-line paste behavior.
- [ ] Templates.
- [ ] Encryption.
- [ ] Git sync.
- [ ] Shell integration notes.
- [ ] Troubleshooting with `#doctor`.
