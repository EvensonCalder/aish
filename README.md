# Aish

**Aish Is not a SHell.** Aish is a smart command-input layer on top of a real shell running inside a PTY.

Aish does not try to replace Bash, Zsh, or Fish. The backend shell still owns command execution, shell syntax, aliases, functions, job control, environment state, and process behavior. Aish owns the interactive input experience around that shell: editable drafts, history and AI browsing modes, live inline completion, templates, safe context collection, editor review, pickers, logging, and conservative sync.

## Current Status

Aish is a usable `0.1` terminal wrapper with extensive unit, expect, PTY, and tmux screen-capture coverage.

Implemented and covered:

- Persistent PTY backend shell with cwd/state preservation.
- Draft, history, and AI browsing modes.
- Readline-style draft editing.
- Shell continuation drafts for unfinished quotes and trailing backslashes.
- Live inline completion with below-prompt hints.
- External editor and multiline paste review.
- History, notes, templates, and event log storage.
- Safe AI request plumbing and context pseudo-pipe flow.
- `fzf`-based history, file, template, git branch, and environment pickers.
- Allowlisted foreground passthrough for interactive and stdin-oriented commands.
- Conservative Git sync configuration and manual push flow.
- GPG-backed key storage and encrypted history/template storage.
- Real-terminal regression coverage through `tmux`.

Explicitly incomplete:

- Configurable key rebinding is not implemented yet.
- Fish support is opt-in until behavior is validated across macOS and representative Linux distributions.
- Async encrypted-history unlock and dedicated GPG/pinentry unlock passthrough remain future work; direct GPG decrypt operations temporarily yield the terminal to `gpg-agent`/pinentry for passphrase entry.
- Future scheduled background events are not implemented yet; current background work is limited to tick-driven refresh and serialized encrypted writes.
- Full automatic passthrough for arbitrary interactive programs remains future work; Aish currently uses an allowlist and tested stdin-command handling.

## Quickstart

Build and run:

```sh
cargo build
./target/debug/aish
```

For an isolated demo or test run:

```sh
AISH_HOME=/tmp/aish-demo ./target/debug/aish
```

On first run, Aish creates an Aish home directory. By default this is `~/.aish`; `AISH_HOME` overrides it and must be an absolute path.

Type a normal shell command and press `Enter`:

```sh
cd /tmp
pwd
```

The backend shell is persistent, so `cd`, `export`, sourced files, aliases, and functions affect later commands like they would in a normal shell.

## Mental Model

```text
keyboard / terminal
        |
        v
  Aish prompt editor
        |
        | raw command text
        v
 persistent PTY shell
        |
        v
 real programs and shell semantics
```

Ordinary input is sent to the backend shell unchanged. Line-leading `#` at the Aish prompt is reserved for Aish commands and AI prompts. Editor-submitted content is raw shell input and intentionally bypasses Aish private-command parsing.

## Modes

Aish has three primary modes:

| Prompt | Mode | Purpose |
| --- | --- | --- |
| `>` | Draft | Edit and submit new shell commands. |
| `$` | History | Browse regular command history read-only. |
| `%` | AI | Browse AI-generated command items read-only. |

Empty `Tab` cycles modes. Entering history mode selects the newest command, entering draft mode opens a blank prompt, and entering AI mode keeps the current AI item pointer. Editing a read-only history or AI item copies it back into draft mode first. `Enter` executes the current draft or selected read-only item.

## Keybindings

Core keys:

- `Enter`: submit the current draft or selected read-only item. Executed commands are copied into regular history, and the active prompt returns to a new blank draft. Saved drafts remain browsable with `Up` / `Down`.
- Empty `Tab`: cycle `>` / `$` / `%` modes. History opens at the newest item, draft opens blank, and AI resumes the current AI pointer.
- Non-empty `Tab`: in `auto` mode, accept the visible inline completion when one is already shown; in `tab` mode, first show completion hints, then accept on a later `Tab`.
- `Right` at end of line: accept completion using the configured accept mode.
- `Up` / `Down` in draft mode: browse saved drafts. `Up` from a blank draft restores the newest saved draft; `Down` from the newest saved draft opens a blank draft.
- `Down` from a non-empty new draft: save the current draft and open a blank draft without executing it.
- `Ctrl-C`: clear the draft, cancel continuation, or reject pending context confirmation.
- `Ctrl-D` on an empty draft: exit.
- `Ctrl-L`: clear the screen.
- `Esc`: clear the draft and return to draft mode.

Editing keys:

- `Left` / `Right`: move by character.
- `Ctrl-A` / `Ctrl-E`: move to start/end.
- `Alt-B` / `Alt-F` or `Alt-Left` / `Alt-Right`: move by word.
- `Backspace` / `Delete`: delete around the cursor.
- `Ctrl-U` / `Ctrl-K`: delete to start/end.
- `Ctrl-W` / `Alt-Backspace`: delete previous word.
- `Alt-D` / `Alt-Delete`: delete next word.

Tools:

- `Ctrl-R`: history search through external `fzf`.
- `Ctrl-X Ctrl-E`: open the configured external editor. On a `# ...` AI prompt, the editor opens the AI prompt body and returns an opaque AI prompt draft for explicit sending.
- `Ctrl-X Ctrl-F`: file picker through external `fzf`.
- `Ctrl-X Ctrl-T`: template picker through external `fzf`.
- `Ctrl-X Ctrl-B`: git branch picker through external `fzf`.
- `Ctrl-X Ctrl-V`: environment variable picker through external `fzf`.

## Completion

Inline completion is enabled by default and refreshes while you type. The best candidate appears as dim ghost text on the active prompt line. Remaining candidates can appear below the prompt as informational hints.

Important rules:

- `completion.mode="auto"` shows live completion hints while you type.
- `completion.mode="tab"` keeps typing quiet; the first `Tab` shows hints and the next `Tab` accepts the visible suggestion or first ranked candidate.
- `completion.mode="off"` disables all Aish completion candidates and makes non-empty `Tab` do nothing.
- `completion.enabled=false` and `completion.inline=false` remain compatibility fields for older configs. Without an explicit `completion.mode`, `inline=false` selects `tab` mode; it does not disable the inline hint that can appear after pressing `Tab`.
- `completion.fuzzy=false` keeps fast prefix/structural completion but disables typo-correction work.
- The inline suggestion is display-only until accepted.
- The below-prompt panel is advisory and never decides what `Tab` accepts.
- `completion.max_results` controls only the number of below-prompt rows.
- `completion.coalesce_ms` controls how long Aish may wait for the next background completion tier before refreshing the live UI. The default is `50` ms; `0` restores immediate tier-by-tier refreshes. First-token executable-only live hints may also wait for this window so history can replace lower-priority PATH matches before anything is drawn.
- `completion.display_delay_ms` controls how long auto mode waits after the latest edit before drawing completion UI. Matching still runs in the background while the display is delayed. The default is `120` ms; `0` draws as soon as candidates are ready.
- The panel skips the current inline candidate and shows the full command that would result from accepting each remaining candidate.
- Candidate rows are width-aware, align command text with the prompt input column when space permits, and left-elide long commands with `...` at word boundaries instead of wrapping.
- Structural history/template matches use `completion.match_threshold_percent` as a word-position match rate. The default is `50`, so one matching word out of two typed words is enough.
- Typo correction is separate and uses `completion.typo_threshold_percent`; accepting a typo candidate replaces the mistyped command with the corrected command.
- `# ` AI prompts stay quiet. `#cmd` input only offers Aish private command names, and private command arguments use the same completion UI for nested subcommands such as `#completion mode tab` or `#encrypt rewrite-history plan`.

Completion sources:

- First token: templates, regular history, then PATH executables.
- Non-first token: structural template matches, structural history suffixes, template placeholders, history arguments, and filesystem paths.
- After a trailing space, Aish uses structural template/history matches and does not show unrelated filesystem entries for the empty token.
- Template completions use newest stored templates first.
- Paths preserve directory prefixes and mark directories with `/`. Matching local directories are kept ahead of lower-priority argument/history fallbacks, and recent directory scans are cached briefly while typing.
- With `completion.fuzzy=true`, Aish can also correct local directory typos such as `./srd` to `./src/`.
- Live completion is layered: cheap local path candidates can be found immediately, template/history/PATH executable matching arrives from a background worker, and slower typo-correction results can update the same UI later. Stale worker results are ignored when the input changes.

Configuration:

```toml
[completion]
mode = "auto" # "auto", "tab", or "off"
enabled = true
max_results = 5
coalesce_ms = 50
display_delay_ms = 120
ignore_spaces = true
template_first = true
inline = true
fuzzy = true
tab_accept = "word" # "full" or "word"
match_threshold_percent = 50
typo_threshold_percent = 80
```

Commands:

```text
#completion
#completion on
#completion off
#completion mode auto
#completion mode tab
#completion mode off
#completion max 8
#completion coalesce-ms 50
#completion display-delay-ms 120
#completion inline on
#completion inline off
#completion fuzzy on
#completion fuzzy off
#completion tab-accept full
#completion tab-accept word
#completion match-threshold 50
#completion typo-threshold 80
```

`tab_accept = "word"` is the default. It accepts only through the next whitespace boundary in the untyped suffix. Use `tab_accept = "full"` to accept the whole suggestion in one step.

## Private Commands

Line-leading `#` is handled by Aish and is not accidentally sent to the backend shell.

Use `#help` for grouped in-terminal help, or `#help commands|keys|ai|paste|completion|templates|sync|encryption|config` for a specific topic.

Diagnostics and status:

```text
#help [topic]
#status
#doctor
#config
#log <count>
#editor
```

AI configuration:

```text
#model <name>
#base-url <url>
#env-key <ENV_NAME>
```

Prompt customization:

```text
#prompt
#prompt draft "{basename} > "
#prompt history "{basename} $ "
#prompt ai "{basename} % "
#prompt reset
```

Use quotes when the prompt template should keep leading or trailing spaces.

Completion:

```text
#completion
#completion on|off
#completion mode auto|tab|off
#completion max <count>
#completion coalesce-ms <0-1000>
#completion display-delay-ms <0-1000>
#completion inline on|off
#completion fuzzy on|off
#completion tab-accept full|word
#completion match-threshold <0-100>
#completion typo-threshold <0-100>
```

Paste review:

```text
#paste
#paste multiline editor|execute|discard
#paste confirm on|off
#paste preview on|off
#paste preview-lines <1-20>
#paste preview-bytes <1-4096>
```

Context:

```text
#context on|off
#context confirm on|off
#context <max-bytes>
```

History, notes, and templates:

```text
#history <count>
#mt <template-body>
#template find <query>
#template show <id>
#template use <id> [key=value ...]
#template rm <id>
#template replace <id> <template-body>
```

Sync:

```text
#set-remote <git-url>
#push
#sync <cron-expression>
#sync off
#sync ai on|off
#sync history on|off
#sync templates on|off
#sync drafts on|off
```

Encryption:

```text
#key set
#key clear
#encrypt on [key-fingerprint|unique-email]
#encrypt rotate <key-fingerprint|unique-email>
#encrypt rewrite-history plan
#encrypt rewrite-history run <key-fingerprint|unique-email> --confirm-rewrite-history
#encrypt off
```

Exit:

```text
#exit
#quit
```

Recognized note lines are stored as notes rather than reaching the shell:

```text
# TODO: ...
# NOTE: ...
# FIXME: ...
# HACK: ...
# XXX: ...
```

## AI And Context

Aish never silently edits or executes AI output. AI results are parsed into browsable items and shown in `%` mode. You execute one selected command at a time with `Enter`.

For generic requests, Aish asks the AI provider to use brace placeholders instead of literals. For example, a generic "echo something" request should prefer a reusable shape like `echo {message}` over `echo "something"`.

Context pseudo-pipe prompts use this form:

```text
# explain this < command producing context
```

Context safety rules:

- Context collection is enabled by default.
- Context commands require confirmation by default.
- `#context confirm off` allows safe context commands to run immediately.
- Dangerous command patterns still require confirmation or are blocked.
- Captured context is byte-limited.
- Context commands are timeout-limited and timed-out process groups are terminated where supported.
- Truncation is disclosed.
- Common token-shaped secrets are redacted before the AI request prompt is built.

AI requests use a chat-completions-compatible endpoint. Configure the model, base URL, and key environment variable with the commands above or in `config.toml`.

## Editor And Paste Review

`Ctrl-X Ctrl-E` opens the configured editor. Resolution order:

1. `editor.command` from config.
2. `$VISUAL`.
3. `$EDITOR`.
4. `nvim`.
5. `vim`.
6. `vi`.

Saved editor content returns as an opaque editor draft. It is not executed until you press `Enter`.

For AI prompts, type `# ` and press `Enter` to open the editor for a multi-line AI instruction. `Ctrl-X Ctrl-E` on an existing `# ...` prompt opens the same AI prompt editor and preserves the current prompt body. The returned draft shows an AI prompt summary; pressing `Enter` sends it to the AI pipeline rather than to the backend shell.

Multiline paste defaults to editor-review behavior. Aish shows a compact draft summary and a bounded escaped preview instead of rendering the full pasted content inline. This prevents accidental execution and keeps the prompt usable. `Ctrl-X Ctrl-E` opens the full pasted content in the editor, and `Enter` submits the raw editor draft as shell input even if it contains line-leading `#`.

Paste preview is controlled by:

```toml
[paste]
multiline = "editor"       # editor | execute | discard
confirm_execute = true
preview = true
preview_lines = 3
preview_bytes = 240
```

## Templates

Templates are stored as JSONL under the Aish home directory. A template is body-first: users do not assign names. Aish prints a stable `tpl-...` content-hash ID when a template is stored or found, and that ID is used for exact `show`, `use`, `rm`, and `replace` operations.

Create one:

```text
#mt rsync -avz {from} {user}@{host}:{to}
```

Find it, then use the printed ID:

```text
#template find rsync
#template use tpl-0123456789abcdef from=dist user=deploy host=example.com to=/srv/app
```

Aish intentionally does not provide a `#template list` command. Full inspection, grep, redirection, and history-oriented cleanup should happen against the template JSONL file in the Aish home directory.

Placeholders:

- `{name}`: required value.
- `{name:description}`: required value with human-readable description.
- `{name...}`: variadic value.

Unresolved placeholders block execution, so a template cannot accidentally run with `{message}` or similar still present.

Completion treats placeholders structurally. For example, after storing `#mt echo {something}`, typing `echo something` can complete to `echo {something}` even though the user did not type the braces. The accepted draft remains a protected template draft, so it cannot execute until the placeholder is replaced.

## Pickers

Aish uses external `fzf` for picker UIs. If `fzf` is missing or a picker is cancelled, Aish reports that clearly and preserves the draft.

Picker surfaces:

- `Ctrl-R`: history search.
- `Ctrl-X Ctrl-F`: file picker.
- `Ctrl-X Ctrl-T`: template picker.
- `Ctrl-X Ctrl-B`: git branch picker.
- `Ctrl-X Ctrl-V`: environment variable picker.

## Shell Backends

Default backend selection:

1. `shell.backend` from config when set to a concrete shell path.
2. `$SHELL` when `shell.backend = "auto"`.
3. `/bin/bash` fallback.

Config:

```toml
[shell]
backend = "auto"
```

Bash and zsh are the default compatibility baseline and are covered by default PTY/tmux tests. Fish support exists but remains opt-in while cross-platform behavior is validated:

```sh
AISH_TEST_FISH=1 cargo test --test pty_backend -- --nocapture
AISH_TEST_FISH=1 cargo test --test tmux_capture tmux_common_shell_workflow_matches_fish_backend_real_terminal_screen -- --nocapture
```

Aish uses shell markers to detect command completion and cwd, filters those markers from user-visible output, and avoids polluting shell history with Aish-owned marker commands where supported.

## Interactive And Stdin Passthrough

Aish foregrounds allowlisted interactive commands so they can own the terminal until they return. This includes common shells, editors, pagers, SSH-like tools, REPLs, database CLIs, `tmux`/`screen`, `gpg`/`pinentry`, privilege/password prompt tools such as `sudo`, `doas`, `sudoedit`, `su`, and `passwd`, and similar programs.

Common stdin-oriented commands such as `cat`, `grep`, `sed`, `awk`, `sort`, `uniq`, `wc`, `tee`, `base64`, and `openssl` are also foregrounded when they are not wrapped in shell control syntax. This prevents commands that wait for stdin from wedging the Aish prompt.

Full automatic detection for every possible alternate-screen or job-control program remains future work.

## Sync

Sync is deliberately conservative.

Implemented:

- Persist remote and sync category config.
- Persist supported startup sync schedules.
- Run `#push` against a configured Git remote.
- Pull with rebase before pushing.
- Add only managed enabled paths.
- Commit only when there is something to commit.
- Abort on conflict-like failures.
- Log sync failures without leaking secret-like values.

Aish does not:

- Auto-resolve conflicts.
- Rewrite history as part of sync. Encrypted storage history rewrite is a separate `#encrypt rewrite-history ... --confirm-rewrite-history` flow.
- Run `git rm --cached` automatically.
- Create scheduler files.
- Remove user-managed files.

## Encryption Status

Aish uses the `gpg` command-line tool for encrypted local storage.

Current behavior:

- Configure `[encryption].key_fingerprint` in `config.toml`, or pass a key selector once with `#encrypt on <key-fingerprint>`.
- A full GPG key fingerprint is the stable key identity. An email/user ID is accepted only when GPG resolves it to exactly one public key.
- `#encrypt on` migrates managed history, notes, drafts, AI history, and templates to `*.jsonl.gpg` files and removes the plaintext JSONL files after successful encryption.
- If encryption is already enabled and the target fingerprint changes, Aish decrypts the existing managed encrypted files and re-encrypts them for the new fingerprint.
- `#encrypt rotate <key>` explicitly re-encrypts current managed storage for a new fingerprint.
- Future writes go to encrypted JSONL files while encryption is enabled. Normal history, draft, note, AI, and template appends are queued through a serialized background encrypted writer so command output and prompt redraws do not wait for GPG.
- Aish flushes pending encrypted writes before exit, `#push`, `#history`, `#encrypt off`, key rotation, and confirmed history rewrite. A background write completion wakes the frontend tick path and refreshes live completion UI.
- `#encrypt off` decrypts managed encrypted JSONL files back to plaintext and disables encrypted writes.
- `#key set` encrypts the API key currently available through `#env-key <ENV_NAME>` into `secrets/key.json.gpg`.
- `#key clear` removes the encrypted key file if present.
- `#status`, `#config`, and `#doctor` report encryption state, key fingerprint configuration, async writer state, the last encrypted-write error if any, and GPG availability.

Typical setup:

```sh
gpg --list-keys --fingerprint
```

Choose a full fingerprint from the listed keys, then enable encryption inside Aish:

```text
#encrypt on ABCDEF0123456789ABCDEF0123456789ABCDEF01
```

After that, history, notes, drafts, AI history, and templates are written as encrypted `*.jsonl.gpg` files. `config.toml` remains plaintext because Aish needs it to find the key fingerprint and startup settings.

To rotate to a new key:

```text
#encrypt rotate FEDCBA9876543210FEDCBA9876543210FEDCBA98
```

This rewrites current managed storage by decrypting with the available old private key and encrypting to the new fingerprint. Git history is not rewritten automatically.

To inspect the manual Git history rewrite flow:

```text
#encrypt rewrite-history plan
```

To run it, first make a separate backup and ensure the Aish storage Git worktree is clean. Then run:

```text
#encrypt rewrite-history run FEDCBA9876543210FEDCBA9876543210FEDCBA98 --confirm-rewrite-history
```

This rewrites the current branch for managed storage paths, creates a local backup branch, and re-encrypts historical plaintext or old-key encrypted blobs for the target fingerprint. Rewritten commit IDs require a deliberate `git push --force-with-lease` if the storage repository is shared.

To store an AI API key with GPG, make the key available in the environment before starting Aish:

```sh
export OPENAI_API_KEY=...
./target/debug/aish
```

Then configure and store it inside Aish:

```text
#env-key OPENAI_API_KEY
#key set
```

On later launches, Aish first uses the current environment variable if it exists. If that variable is missing, Aish falls back to the encrypted key in `secrets/key.json.gpg`.

To stop using the stored key:

```text
#key clear
```

To decrypt managed storage back to plaintext and write plaintext files from then on:

```text
#encrypt off
```

Known limits: encrypted startup loading is synchronous. Direct decrypt operations temporarily leave raw mode so `gpg-agent`/pinentry can prompt for passphrases; a fully async unlock UI with dedicated GPG/pinentry unlock passthrough is still future work. Aish warns that Git history can contain plaintext data or data encrypted for an older key; history rewrite is available only through the explicit confirmed command above.

## Files And Storage

Default layout under `~/.aish`:

```text
config.toml
history/
  regular.jsonl
  ai.jsonl
  draft.jsonl
  notes.jsonl
templates/
  templates.jsonl
logs/
  events.jsonl
secrets/
cache/
.gitignore
```

Use `#doctor`, `#status`, and `#config` to inspect the active paths and runtime settings.

On Unix-like systems, Aish creates managed storage directories with private directory permissions and writes config/history/encrypted storage files with private file permissions where supported.

## Code Organization

The app module root in `src/app.rs` wires together focused runtime modules:

- `src/app/state.rs`: `AppState`, output ring state, draft/history/template state transitions, and encrypted writer lifecycle.
- `src/app/bootstrap.rs`: startup layout/config/history/template loading and terminal launch.
- `src/app/completion_runtime.rs`: AppState completion request/cache orchestration.
- `src/app/config_commands.rs`: `#model`, `#base-url`, `#env-key`, `#context`, `#paste`, and `#completion` config mutations.
- `src/app/context_prompt.rs`: AI prompt submission, context collection confirmation, and contextual prompt building.
- `src/app/encryption_commands.rs`: GPG key storage, `#encrypt`, current-storage rotation, and confirmed history rewrite.
- `src/app/execution.rs`: draft submission, command execution, foreground passthrough, PTY output forwarding, and command recording.
- `src/app/history_ops.rs`: history trimming and encrypted/plain AI history loading helpers.
- `src/app/template_args.rs`: template subcommand argument parsing.
- `src/app/event_log.rs`: event log display for `#log`.
- `src/app/reports.rs`: `#status`, `#config`, `#doctor`, `#editor`, and encryption/sync status output.
- `src/app/sync_commands.rs`: `#set-remote`, `#sync`, `#push`, startup sync checks, and git step handling.
- `src/config.rs`: public config module facade and config tests.
- `src/config/`: config model types, directory layout, private file permissions, root path resolution, file IO, and normalization.
- `src/completion.rs`: completion orchestration across templates, history, paths, private commands, and typo tiers.
- `src/completion/`: focused completion helpers for matching rules, token parsing, path/PATH scanning, private command completion, and rendering/acceptance.
- `src/terminal.rs`: terminal event loop, key/paste handling, picker/editor boundaries, and prompt redraw positioning.
- `src/terminal/completion_ui.rs`: live completion display, inline suffixes, Tab/Right acceptance, and completion panel state transitions.
- `src/pty.rs`: PTY backend lifecycle, command execution, read loop, and streaming event callbacks.
- `src/pty/`: shell launch setup, marker parsing, output filtering, and continuation syntax helpers.

## Testing

Distributed tester documentation:

- `FULL_TESTS.md`: complete functional checklist for automated, tmux, and human real-world validation.
- `TESTING_MANUAL.md`: step-by-step guide for humans or AI assistants guiding a tester through platform setup, build, tests, and report writing.

Main verification for feature changes:

```sh
cargo fmt --check
cargo test --lib
cargo test --test draft_execution -- --nocapture
cargo test --test first_run -- --nocapture
cargo test --test pty_backend -- --nocapture
cargo test --test expect_runner -- --test-threads=1 --nocapture
cargo test --test tmux_capture -- --test-threads=1 --nocapture
cargo clippy --all-targets -- -D warnings
git diff --check
cargo build
```

Shorter all-Rust pass:

```sh
cargo test
```

Current active inventory:

- 490 library unit tests.
- 26 draft execution integration tests.
- 1 first-run integration test.
- 21 PTY integration tests, with bash/zsh active by default and fish-specific cases opt-in.
- 114 expect-driven end-to-end interactive scenarios.
- 44 tmux screen-capture integration tests.

Expect and tmux tests launch real terminal sessions with isolated Aish homes. They should be serialized because concurrent real-terminal sessions can create false prompt and scheduler failures.

## Troubleshooting

Run diagnostics:

```text
#doctor
#status
#log 20
```

Useful environment overrides:

```sh
AISH_HOME=/tmp/aish-debug ./target/debug/aish
NO_COLOR=1 ./target/debug/aish
```

If a command appears to wait for stdin or take over the terminal incorrectly, test it in a normal shell and then in Aish. Aish should foreground common interactive and stdin-oriented commands; new real-world failures should get a tmux regression test.
