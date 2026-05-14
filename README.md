# Aish

Aish is an AI-assisted terminal wrapper. The name means **Aish Is not a SHell**: your real shell still executes commands and owns shell semantics, while Aish provides a safer prompt, history/AI browsing modes, templates, completion, editor review, and context-aware AI plumbing around it.

## Quickstart

Build and run the debug binary:

```sh
cargo build
./target/debug/aish
```

On first run, Aish creates an Aish home directory, defaulting to `~/.aish`. For isolated testing or demos, set `AISH_HOME`:

```sh
AISH_HOME=/tmp/aish-demo ./target/debug/aish
```

The prompt starts in draft mode with `>`. Type a shell command and press `Enter`; the backend shell keeps state, so `cd /tmp` affects later commands.

## Modes

- `>` draft mode: edit and submit commands.
- `$` history mode: browse regular command history.
- `%` AI mode: browse generated AI command items.

Press empty `Tab` to cycle modes. Editing a read-only history or AI item copies it back to draft mode first.

## Keybindings

- `Enter`: submit the draft or selected read-only item.
- `Tab` on an empty draft: cycle `>` / `$` / `%` modes.
- `Tab` on a non-empty draft: accept a unique completion or show candidate rows.
- `Right` at the end of a non-empty draft: accept completion.
- `Ctrl-C`: clear the current draft or cancel pending continuation/context confirmation.
- `Ctrl-D` on an empty draft: exit.
- `Ctrl-L`: clear screen.
- `Ctrl-A`, `Ctrl-E`, `Ctrl-U`, `Ctrl-K`, `Ctrl-W`, `Alt-B`, `Alt-F`, arrows: readline-style editing/navigation.
- `Ctrl-X Ctrl-E`: open the configured external editor.
- `Ctrl-R`: launch history search through external `fzf`.
- `Ctrl-X Ctrl-F`: file picker through external `fzf`.
- `Ctrl-X Ctrl-T`: template picker through external `fzf`.
- `Ctrl-X Ctrl-B`: git branch picker through external `fzf`.
- `Ctrl-X Ctrl-V`: environment variable picker through external `fzf`.

## Private Commands

Line-leading `#` input is handled by Aish and is not accidentally sent to the backend shell.

Implemented commands include:

- `#help`: list private commands and keybindings.
- `#status`: print runtime status, AI config source, completion/context config, and diagnostics.
- `#doctor`: print setup diagnostics for shell, PTY, editor, `fzf`, AI config, and storage paths.
- `#config`: print config paths and runtime config values.
- `#model <name>`: set AI model.
- `#base-url <url>`: set and normalize chat-completions base URL.
- `#env-key <NAME>`: configure the API key environment variable name.
- `#context on|off|confirm on|confirm off|<bytes>`: configure context capture.
- `#completion max <count>`: configure the completion candidate display limit.
- `#log <count>`: print recent event log entries.
- `#history <count>`: trim combined regular and AI command history.
- `#mt <name> <body>`: store a template.
- `#template list|show|use|rm|replace ...`: manage templates.
- `#exit` / `#quit`: exit Aish.

Recognized note commands are stored as notes instead of reaching the shell:

- `# TODO: ...`
- `# NOTE: ...`
- `# FIXME: ...`
- `# HACK: ...`
- `# XXX: ...`

## AI Safety

Aish never auto-executes AI output. AI responses are parsed as JSON items, shown in `%` mode, and executed only when you explicitly press `Enter` on a selected command item.

Context pseudo-pipe prompts use this form:

```text
# explain this < command producing context
```

Context commands are confirmed by default, dangerous patterns are blocked or require confirmation, captured context is byte-limited, truncation is disclosed, and common token-shaped secrets are redacted before context is added to the AI request prompt.

## Editor And Paste Review

`Ctrl-X Ctrl-E` opens the configured editor (`editor.command`, `$VISUAL`, `$EDITOR`, `nvim`, `vim`, then `vi`). Saved editor content returns as an opaque editor draft and is not executed until you press `Enter`.

Multi-line paste defaults to editor-review behavior. Aish shows an editor draft summary with `review before Enter`; pasted commands are not silently executed unless configured otherwise.

Editor-returned content intentionally bypasses Aish private-command parsing. This means a leading `#` line from the editor is submitted as raw shell content when you explicitly run the editor draft.

## Templates

Templates are stored as JSONL entries under the Aish home. Placeholders use `{name}`, `{name:description}`, and `{name...}` syntax.

Unresolved template placeholders block execution, so `echo {message}` from a template cannot run until the placeholder is resolved or edited into plain draft text.

## Completion And Pickers

Completion works directly in draft mode. Template completions rank before history and executable candidates for first-token completion. Non-first-token completion can use history arguments, template placeholders, and filesystem paths.

Picker features use external `fzf`; Aish does not implement an internal picker UI.

## Encryption And Sync Status

GPG-backed key storage and encrypted history/templates are not implemented yet. Current `#key set` and `#encrypt` commands are safe placeholders except that `#encrypt on` warns about existing plaintext in git history. `#key clear` removes an existing encrypted key file if present.

Git sync configuration and manual sync are implemented conservatively. `#set-remote`, `#sync off`, `#sync <expr>`, and `#sync ai|history|templates|drafts on|off` persist sync configuration without creating scheduler files. `#push` runs a conservative local git flow for configured remotes: pull with rebase, add managed enabled paths, commit if needed, and push. Aish does not auto-resolve conflicts, does not rewrite history, and does not run `git rm --cached` automatically.

## Shell Integration Notes

Aish starts a backend shell on a PTY. Bash and zsh are actively covered. Aish uses markers to detect command completion and cwd, filters internal marker output, and keeps Aish marker commands out of shell history where supported.

Shell continuation uses shell-native syntax checks where possible. Incomplete quote input such as `echo "` or `echo '` becomes an Aish continuation draft with shell-style prompts. Odd trailing backslashes are treated as continuations to match interactive shell behavior.

Fish integration is implemented when `fish` is available. Allowlisted interactive commands such as `less`, `vim`, `nvim`, `ssh`, `top`, `fzf`, and `tmux` can use foreground passthrough. Full automatic passthrough for arbitrary alternate-screen programs remains future work.

## Testing

Run the main verification set before committing:

```sh
cargo fmt --check
cargo test --lib
cargo test --test draft_execution
cargo test --test pty_backend
cargo test --test expect_runner
cargo test --test first_run
cargo clippy --all-targets -- -D warnings
git diff --check
cargo build
```

Expect tests launch the built `aish` binary in real terminal sessions with isolated `AISH_HOME` directories.

## Troubleshooting

Run:

```text
#doctor
```

Use `#status` for current runtime status and `#log <count>` for recent event-log entries. These diagnostics redact common token-shaped secrets and report key source without printing the key itself.
