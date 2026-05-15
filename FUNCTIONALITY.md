# Aish Functionality

This document describes the behavior implemented in the current codebase. It is a product contract for automated and human testing, not a wishlist. Items marked incomplete are intentionally not claimed as shipped.

## Core Model

- Aish is a PTY-backed command input layer, not a shell.
- Ordinary input is sent to the configured backend shell unchanged.
- Shell state persists across commands, including `cd`, exports, functions, aliases, and shell-local state.
- Aish reserves direct prompt input that begins with `#`; editor-submitted content bypasses that parser and is sent as raw shell input.
- Aish starts in draft mode and renders its own prompt instead of exposing the backend shell prompt.

## Startup And Storage

- Aish chooses its state directory from non-empty absolute `AISH_HOME`; if unset or empty, it uses `$HOME/.aish`.
- Missing or relative `AISH_HOME`, missing/relative `HOME`, and an Aish home path that is a file fail with readable errors.
- First run creates `config.toml`, `history/`, `templates/`, `secrets/`, `logs/`, and `cache/runtime/`.
- Config is TOML, denies unknown fields, and normalizes empty/defaultable values.
- Invalid config fails startup with a readable error.
- Read-only diagnostic commands do not create missing history/template/log files just by printing paths.

## Modes

- `>` draft mode edits and submits commands.
- `$` history mode browses regular command history newest first.
- `%` AI mode browses command items from stored/generated AI sessions.
- Empty `Tab` cycles `>` to `$` to `%` back to `>`.
- History and AI modes are read-only; typed edits copy the selected item into draft mode first.
- Cursor-only movement in read-only modes does not copy into draft.
- `Esc` clears the draft and returns to draft mode.
- `Esc` draft clearing is validated with terminal pane capture: a cleared draft should not execute, and the next command should execute normally.
- Mode-switch redraw is validated with terminal pane capture: output visible before `$`/`%`/`>` cycling should remain visible, and the next command should execute normally after returning to draft mode.
- History-mode execution is validated with terminal pane capture: a command selected from `$` mode should execute and produce visible output in the real terminal.

## Draft Editing

- Draft editing is UTF-8 safe.
- Supported editing keys include printable input, `Left`, `Right`, `Ctrl-A`, `Ctrl-E`, `Ctrl-U`, `Ctrl-K`, `Ctrl-W`, `Backspace`, `Delete`, `Alt-B`, `Alt-F`, `Alt-Left`, and `Alt-Right`.
- `Ctrl-D` exits only on an empty draft; otherwise it deletes at cursor.
- `Ctrl-D` empty-draft exit is validated with tmux: the Aish pane should terminate instead of leaving a hidden running process behind.
- `Ctrl-C` clears draft/continuation state and returns to draft mode.
- `Ctrl-L` clears the screen and redraws the prompt without adding a blank first row.
- Multi-line ordinary drafts can be submitted as one shell input when complete.

## Command Execution And PTY

- Bash uses marker-based integration with clean startup flags.
- Zsh uses `preexec`/`precmd` hooks when available.
- Fish launch/event integration exists as experimental support; real fish PTY and tmux coverage is opt-in with `AISH_TEST_FISH=1` until it is validated across macOS and Linux distributions.
- Command completion markers include exit status and cwd.
- Command output is displayed as terminal protocol without Aish-added framing newlines.
- In actual terminal use, a command's visible output appears directly below the submitted command line without an Aish-inserted blank line, and remains visible above the next prompt after redraw.
- Common shell workflows are covered through the real binary and backend-specific tmux runs for bash and zsh by default, with opt-in fish coverage available through `AISH_TEST_FISH=1`, including persistent `cd`, `mkdir`, redirection, `cat | grep`, quoted arguments, exported environment variables, file tests, failed commands, and prompt recovery afterward.
- Failed commands are stored with exit status.
- `clear`-style terminal output and mixed stdout/stderr redraw back to a visible prompt.

## Terminal Acceptance

- Byte-stream expect assertions are not sufficient for prompt/output regressions.
- Output visibility regressions must be checked against final rendered terminal state, not only against text that appeared in raw PTY output at some point.
- Persistent `tmux` screen-capture tests cover real terminal workflows where redraw or cursor motion could otherwise hide output.
- Longer backend-specific tmux workflows capture pane scrollback to validate the whole interactive session after normal terminal scrolling.
- `Ctrl-L` clear-screen behavior is validated against final terminal pane state: pre-clear output should be gone, the prompt should remain usable, and post-clear command output should appear normally.
- Completion no-match behavior is validated with terminal pane capture: `no completions` should become visible before dismissal, `Esc` should return to a usable prompt, and the next command should execute normally.
- Completion acceptance via `Right` at end-of-line is validated with terminal pane capture: accepting a file completion should update the visible command and execute that completed command normally.
- Real interactive expect scenarios are serialized in the test runner because parallel terminal sessions can create scheduler and PTY races that do not represent single-user operation.
- Real `tmux` screen-capture tests are serialized for the same reason; they model one user driving one terminal workflow at a time.
- Unicode final-screen behavior is covered through `tmux` pane capture rather than Tcl/expect when expect's own Unicode handling is unstable.
- `tmux capture-pane -p` trims trailing spaces from captured lines, so tmux tests must not use it to prove prompt or continuation trailing-space behavior; use expect byte-stream assertions or Rust terminal rendering tests for those details.

## Continuation

- Incomplete double quotes show `dquote> ` continuation.
- Incomplete single quotes show `quote> ` continuation.
- Odd trailing backslashes show generic `> ` continuation.
- Backend shell `PS2`/`PROMPT2` prompts are suppressed from visible command output.
- `Ctrl-C` cancels continuation state without wedging the backend shell.
- Continuation cancellation is validated with terminal pane capture: the continuation prompt should be visible before cancellation, `Ctrl-C` should return to a normal prompt, and the next command should execute normally.

## Private Commands

- `#help` lists private commands and keybindings.
- `#status` prints runtime status, shell, AI URL/key source, encryption state, sync state, context config, completion config, and keybinding count.
- `#status` visibility is validated with tmux: status detail lines should be visible in the real terminal, while full header coverage remains in expect because long status output can scroll in a small pane; the next backend-shell command should execute normally.
- `#config` prints runtime config values and storage paths.
- `#doctor` prints read-only diagnostics for shell, PTY, GPG/git/fzf placeholders, editor, AI config, sync, encryption, and storage paths.
- `#exit` and `#quit` exit Aish.
- `#exit` is validated with tmux: the Aish pane should terminate instead of leaving a hidden running process behind.
- Unknown private commands never reach the backend shell and may show a nearest-command suggestion.
- Invalid private command usage leaves the session usable.

## Notes And Logs

- Direct prompt notes `# TODO:`, `#TODO:`, `# NOTE:`, `# FIXME:`, `# HACK:`, and `# XXX:` are stored as notes and are not sent to the shell or AI.
- The event log lives at `logs/events.jsonl` and is capped at 1000 events.
- `#log <count>` prints recent events.
- Event logs redact common secret-shaped tokens.

## AI

- `#model <name>`, `#base-url <url>`, and `#env-key <NAME>` persist AI configuration.
- `#base-url` normalizes to a chat-completions URL.
- Direct `# prompt` requests use configured env-key credentials and a chat-completions-compatible API.
- AI responses must contain final JSON items; invalid or empty responses fail clearly.
- AI item boundaries come from JSON `items`, not line breaks.
- Template AI items are not auto-saved.
- Missing AI config reports an error without crashing or mutating AI history.
- AI mode executes one command item at a time, advances on success, stays on failure, and returns to draft after the last successful command in a session.
- There is no execute-all shortcut.

## Context Pseudo-Pipe

- `# prompt < command` can collect command output as AI context.
- `#context`, `#context on`, `#context off`, `#context confirm on`, `#context confirm off`, and `#context <bytes>` are implemented and persist to config.
- Context collection is enabled and confirmation is on by default.
- Dangerous context commands require confirmation even when confirmation is otherwise disabled.
- Skipped context commands are logged and not executed.
- Context output captures stdout and stderr, applies a byte cap, discloses truncation, and redacts common secret-shaped tokens in the AI prompt.

## Completion

- Non-empty `Tab` in draft mode computes completion candidates.
- Inline completion is enabled by default; the first completion action displays the highest-ranked candidate as a display-only inline suffix and shows below-prompt candidate rows.
- A following `Tab` accepts the inline suggestion. `completion.tab_accept = "full"` accepts the full suffix; `"word"` accepts only through the next shell word.
- When inline completion is disabled, non-empty `Tab` accepts the first ranked candidate directly.
- Candidate rows are displayed below the prompt with source labels, are limited by `completion.max_results`, and are elided with `...` instead of wrapping in narrow terminals.
- No candidates displays `no completions` below the prompt.
- `Right` at end-of-line accepts completion using the configured accept amount; `Right` inside a line moves the cursor.
- First-token completion ranks templates before history before PATH executables.
- Non-first-token completion includes history arguments, filesystem paths, and template placeholders.
- Directory path candidates end with `/`.
- Matching can ignore spaces by default.
- `#completion` prints completion config.
- `#completion max <count>` persists the maximum number of visible completion candidates; zero and non-numeric values are rejected without changing config.
- `#completion inline on|off` and `#completion tab-accept full|word` persist inline completion behavior and reject invalid values without changing config.

## Pickers

- Pickers use external `fzf`.
- `Ctrl-R` opens history search; selection replaces the draft line without shell quoting.
- `Ctrl-X Ctrl-F` opens file picker; selection replaces current token with shell quoting.
- `Ctrl-X Ctrl-T` opens template picker; selection copies the newest matching template to a protected template draft.
- `Ctrl-X Ctrl-B` opens git branch picker; selection replaces current token with shell quoting.
- `Ctrl-X Ctrl-V` opens environment variable picker; selection inserts a raw `$NAME` reference.
- Picker cancellation preserves draft content and reports cancellation.

## Editor And Paste

- Editor resolution order is config command, `$VISUAL`, `$EDITOR`, `nvim`, `vim`, then `vi`.
- `Ctrl-X Ctrl-E` opens the current draft, or copies selected history/AI content first.
- Editor temp files are created with private permissions where supported.
- Successful editor exit replaces the draft with an opaque editor draft summary.
- Failed editor exit preserves the previous draft and reports status.
- Editor drafts are not parsed as Aish `#` commands when submitted.
- `editor.execute_after_save = true` executes only after a successful editor exit.
- Single-line paste inserts at cursor and copies read-only selections first.
- Multi-line paste defaults to an opaque editor draft and does not execute immediately.
- `paste.multiline = "discard"` ignores multi-line paste.
- `paste.multiline = "execute"` submits immediately only when `confirm_execute = false`; otherwise it creates an editor draft for review.

## Templates

- `#mt <body>` appends a body-first template and prints a stable `tpl-...` content-hash ID.
- `#template find <query>` prints matching template IDs and bodies.
- `#template list` is intentionally unsupported; bulk grep/redirection belongs on the JSONL store file.
- `#template show <id>` prints the matching template body.
- `#template use <id>` copies the matching template to a protected draft.
- `#template use <id> key=value...` substitutes matching placeholders.
- Quoted template values with spaces are supported.
- `#template rm <id>` removes matching templates.
- `#template replace <id> <body>` removes old matches and appends one replacement with a new body-derived ID.
- Placeholders support `{name}`, `{name:description}`, and `{name...}`.
- Protected template drafts with unresolved placeholders cannot execute.
- Backspace/Delete outside a placeholder removes the whole placeholder span; editing inside a placeholder expands the draft to plain text.

## History

- Regular history is JSONL and stores command, timestamp, exit code, and source.
- Draft history is JSONL and non-empty drafts persist on normal exit when enabled.
- The newest persisted draft restores on startup when draft persistence is enabled.
- AI history is JSONL and stores prompt, model, context flag, and ordered items.
- Bad JSONL lines are skipped and reported at load/trim surfaces where applicable.
- `#history <count>` trims regular history and AI command items to a combined command-item limit.
- Default history preserves submitted multi-line editor/paste content faithfully as one command string.

## Encryption And Secrets

- Full GPG-backed `#key set` is not implemented.
- `#key clear` removes `secrets/key.json.gpg` if present and logs the action.
- `#encrypt on` and `#encrypt off` are safe placeholders; they do not change storage formats or migrate files.
- `#encrypt on` warns that existing plaintext may remain in git history and Aish will not rewrite history automatically.
- GPG encryption command planning is implemented and unit-tested with fake GPG.
- Atomic encrypted-write helper scaffolding is implemented and tested, but not wired to history/templates/secrets.
- Encrypted history/templates, decrypt-on-startup, no-plaintext indexes, and unlock passthrough remain incomplete.

## Sync

- `#set-remote <git-url>` persists the sync remote without running git.
- `#sync <schedule>` persists sync schedule without creating scheduler files.
- `#sync off` disables startup sync without creating scheduler files.
- `#sync ai|history|templates|drafts on|off` toggles future managed paths.
- `#push` runs a conservative local git flow when a remote is configured.
- Sync maintains an Aish-managed `.gitignore` section while preserving user content.
- Sync initializes a repo if needed, skips initial pull in a new repo, adds only existing enabled managed paths plus `.gitignore`, commits if needed, and pushes with upstream.
- Sync uses a lock file to avoid concurrent runs.
- Sync never auto-resolves conflicts, rewrites history, creates scheduler files, or runs `git rm --cached` automatically.
- Sync failures/conflicts are logged with redaction.

## Passthrough And Interactive Programs

- Allowlisted interactive commands such as `less`, `vim`, `nvim`, `ssh`, `top`, `fzf`, `tmux`, and similar use a foreground passthrough path.
- In passthrough/unlock passthrough state, Aish forwards keys instead of interpreting app keybindings.
- Alternate-screen enter/exit detection and prompt-return helpers are implemented as pure helpers.
- Full automatic async passthrough for arbitrary alternate-screen programs remains incomplete.

## Known Incomplete Functionality

- Configurable key rebinding.
- Full GPG-backed `#key set`.
- Encrypted history, AI history, drafts, notes, templates, and search/completion indexes.
- Async decrypt/unlock and user-visible `history is still unlocking...` state.
- GPG/pinentry terminal handoff through `UnlockPassthrough`.
- Independent central event-loop sources for PTY output and timers.
- Full automatic passthrough detection for arbitrary interactive programs.
- Internal picker UI; current picker integration depends on external `fzf`.
- Live network AI behavior is not covered by automated tests.

## Human Required Tests

- Run Aish interactively under Bash and Zsh on a real terminal by default; validate Fish on a cross-platform matrix before promoting it from opt-in experimental support.
- Manually verify `whoami`, repeated `whoami`, and `echo 123` show output directly under each submitted command with no extra blank line and no disappearing output.
- Verify real `vim`/`nvim` foreground editing, suspend/resume, save/failure behavior, and terminal restoration.
- Verify `ssh` passthrough to a local or test host, including password/key prompts and exit restoration.
- Verify `less`, `top`/`htop`, `fzf`, and `tmux`/`screen` behavior in real alternate-screen terminals.
- Verify terminal resize while commands and fullscreen programs are running.
- Verify long Unicode editing, cursor movement, deletion, and redraw in a real terminal emulator.
- Verify bracketed paste from the OS clipboard for single-line and multi-line content.
- Verify live AI requests against a test-compatible chat-completions endpoint with a disposable API key.
- Verify no secret values appear in status, logs, or AI context when using real environment variables.
- Verify GPG/pinentry behavior manually once `#key set` and unlock flows are implemented.
- Verify sync against a real non-production remote, including conflict presentation and manual recovery.
- Verify installation/package execution outside the Cargo workspace once packaging exists.
