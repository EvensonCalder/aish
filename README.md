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
- Real-terminal regression coverage through `tmux`.

Explicitly incomplete:

- GPG-backed key storage and encrypted history/templates are not implemented yet.
- `#key set` and `#encrypt on|off` are safe placeholders; `#key clear` can remove an existing key file.
- Configurable key rebinding is not implemented yet.
- Fish support is opt-in until behavior is validated across macOS and representative Linux distributions.
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

Empty `Tab` cycles modes. Editing a read-only history or AI item copies it back into draft mode first. `Enter` executes the current draft or selected read-only item.

## Keybindings

Core keys:

- `Enter`: submit the current draft or selected read-only item. Ordinary drafts stay visible after execution and are copied into regular history.
- Empty `Tab`: cycle `>` / `$` / `%` modes.
- Non-empty `Tab`: accept the current inline completion, or directly accept the first candidate when inline completion is disabled.
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
- `Ctrl-W`: delete previous word.

Tools:

- `Ctrl-R`: history search through external `fzf`.
- `Ctrl-X Ctrl-E`: open the configured external editor.
- `Ctrl-X Ctrl-F`: file picker through external `fzf`.
- `Ctrl-X Ctrl-T`: template picker through external `fzf`.
- `Ctrl-X Ctrl-B`: git branch picker through external `fzf`.
- `Ctrl-X Ctrl-V`: environment variable picker through external `fzf`.

## Completion

Inline completion is enabled by default and refreshes while you type. The best candidate appears as dim ghost text on the active prompt line. Remaining candidates can appear below the prompt as informational hints.

Important rules:

- The inline suggestion is display-only until accepted.
- The below-prompt panel is advisory and never decides what `Tab` accepts.
- `completion.max_results` controls only the number of below-prompt rows.
- The panel skips the current inline candidate and shows remaining suffixes where possible.
- Candidate rows are width-aware and elide with `...` instead of wrapping.
- If `completion.inline=false`, non-empty `Tab` preserves the legacy behavior and accepts the first ranked candidate directly.

Completion sources:

- First token: templates, regular history, then PATH executables.
- Non-first token: structural template matches, structural history suffixes, template placeholders, history arguments, and filesystem paths.
- Template completions use newest stored templates first.
- Paths preserve directory prefixes and mark directories with `/`.
- Matching ignores spaces by default.

Configuration:

```toml
[completion]
max_results = 5
ignore_spaces = true
template_first = true
inline = true
tab_accept = "full" # "full" or "word"
```

Commands:

```text
#completion
#completion max 8
#completion inline on
#completion inline off
#completion tab-accept full
#completion tab-accept word
```

`tab_accept = "word"` accepts only through the next whitespace boundary in the untyped suffix. This is useful for long history completions such as `kubectl apply -f deployment.yaml`.

## Private Commands

Line-leading `#` is handled by Aish and is not accidentally sent to the backend shell.

Diagnostics and status:

```text
#help
#status
#doctor
#config
#log <count>
```

AI configuration:

```text
#model <name>
#base-url <url>
#env-key <ENV_NAME>
```

Completion:

```text
#completion
#completion max <count>
#completion inline on|off
#completion tab-accept full|word
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

Encryption placeholders:

```text
#key set
#key clear
#encrypt on
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

Multiline paste defaults to editor-review behavior. Aish shows a compact draft summary instead of rendering the full pasted content inline. This prevents accidental execution and keeps the prompt usable. Editor-returned content is submitted as raw shell input when explicitly executed, even if it contains line-leading `#`.

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

Aish foregrounds allowlisted interactive commands so they can own the terminal until they return. This includes common shells, editors, pagers, SSH-like tools, REPLs, database CLIs, `tmux`/`screen`, `gpg`/`pinentry`, and similar programs.

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
- Rewrite history.
- Run `git rm --cached` automatically.
- Create scheduler files.
- Remove user-managed files.

## Encryption Status

Encryption is intentionally not overclaimed.

Current behavior:

- `#key set` reports that key storage is not implemented yet.
- `#key clear` removes an existing encrypted key file if present.
- `#encrypt on|off` are safe placeholders and do not migrate storage.
- `#encrypt on` warns conservatively about plaintext that may already exist in Git history.

Future encryption work needs a tested GPG boundary, fake-GPG or isolated-key integration tests, encrypted history/template writes, locked-history behavior, and a terminal-safe unlock/pinentry flow before it can be considered complete.

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

- 383 library unit tests.
- 23 draft execution integration tests.
- 1 first-run integration test.
- 13 PTY integration tests, with bash/zsh active by default and fish-specific cases opt-in.
- 106 expect-driven end-to-end interactive scenarios.
- 32 tmux screen-capture integration tests.

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
