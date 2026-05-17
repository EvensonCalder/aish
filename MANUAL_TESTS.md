# Aish Manual-Only Test List

This file lists the remaining checks that should still be done by a human in a real terminal. Deterministic workflows from the old manual checklist have been moved into Rust, expect, or tmux automation.

Run the automated manual-equivalent tmux workflows with:

```sh
cargo test --test tmux_capture tmux_manual -- --test-threads=1 --nocapture
```

Those tmux workflows cover common shell commands, prompt editing keys, completion config and truncation, private commands and notes, templates, external editor smoke, default `$HOME/.aish`, AI/context/sync config, local sync, `less` passthrough smoke, and startup failure messages.

Use an isolated home for every human manual run unless the test explicitly asks for production-home behavior:

```sh
cargo build
export AISH_MANUAL_ROOT="/tmp/aish-manual-$(date +%s)"
mkdir -p "$AISH_MANUAL_ROOT"
export AISH_HOME="$AISH_MANUAL_ROOT/home"
./target/debug/aish
```

## Human-Only Tests

| ID | What To Do | Expected Behavior | Why It Stays Manual |
| --- | --- | --- | --- |
| H-001 | In at least two real terminal themes, type a prefix that has an inline completion and pause before accepting it. | The inline suggestion is visibly dim, readable, aligned with the typed command, and not confused with committed text. | Tmux can assert text, but not whether the color/contrast feels correct to a user. |
| H-002 | In a narrow real terminal, trigger long completions near the right edge. | The below-prompt panel stays single-line per candidate, truncates with `...`, and the inline hint does not overlap other UI. | Tmux covers basic elision; human review is still needed for visual polish across fonts and terminals. |
| H-003 | Use `Tab` with both completion modes: full accept and word accept. | Full mode accepts the complete suggestion; word mode accepts only through the next shell word; the behavior feels predictable. | The mechanics are automated; the remaining check is UX intuition. |
| H-004 | Paste one single-line command from the OS clipboard, then paste several lines from the OS clipboard. | Single-line paste inserts text without executing. Multi-line paste opens the review/editor flow and never silently executes. | Real clipboard and terminal bracketed-paste behavior varies by terminal emulator. |
| H-005 | Use your normal editor through `Ctrl-X Ctrl-E`, for example `vim`, `nvim`, or another full-screen editor. Save a command and exit. | Aish restores the terminal, shows the editor draft summary, and executes only after `Enter`. | Fake-editor and smoke flows are automated; real editor terminal behavior is environment-specific. |
| H-006 | Make the real editor exit with a failure or kill it from another terminal. | Aish reports the editor failure, preserves the original draft where possible, and leaves the terminal recoverable. | Failure behavior depends on the editor and terminal. |
| H-007 | Use real `fzf` pickers for history, file paths with spaces, templates, git branches, and environment variables. Confirm and cancel each picker. | Confirmed choices insert the expected shell-safe value; canceled pickers preserve the previous draft. | Fake picker cancellation is automated; real `fzf` layout and key handling still need human verification. |
| H-008 | Run full-screen or interactive passthrough programs such as `vim`, `nvim`, `top`, `node`, `ssh`, or nested `tmux`. Use their normal keybindings, then exit. | Aish forwards keys to the program and recovers its prompt after the program exits. | `less` and `python3` REPL smokes are automated; broad alternate-screen and job-control behavior is not portable enough for one tmux assertion. |
| H-009 | Test a disposable real OpenAI-compatible endpoint and API key. Submit an AI prompt that returns command JSON. | Aish displays command candidates, redacts secrets in status/log output, and never auto-executes returned commands. | Real network, auth, provider errors, and rate limits should not be part of the default automated suite. |
| H-010 | With an isolated `GNUPGHOME`, disposable passphrase-protected test key, and isolated `AISH_HOME`, run `#encrypt on <fingerprint>`, `#key set`, relaunch without the environment API key, rotate to a second key if available, and run `#encrypt off`. | GPG/pinentry gets terminal control safely, secrets are not printed, encrypted files are written as `*.jsonl.gpg`, stored API key fallback works only after successful decrypt, and plaintext/encrypted state claims are accurate. | Fake-GPG automation covers command boundaries and storage migration; real pinentry, agent caching, and passphrase UX require a real terminal. |
| H-011 | Validate fish backend behavior on macOS and representative Linux distributions with different fish versions. | Fish either passes the same user-visible workflows as bash/zsh or remains clearly marked experimental with known issues. | Fish startup and interactive behavior can differ by platform/version; default automation keeps fish opt-in. |
| H-012 | Run the smoke workflow on real macOS and Linux terminals: Terminal.app, iTerm2, Alacritty, GNOME Terminal, and Konsole where available. | Prompt redraw, Unicode, completion, paste, and passthrough behavior remain stable. | Cross-terminal rendering and input behavior cannot be fully proven on one developer machine. |
| H-013 | Intentionally use a disposable but production-shaped `$HOME` without `AISH_HOME`. Optionally inspect a real personal `~/.aish` only after backing it up. | Aish creates or reuses `$HOME/.aish` without touching unrelated files; history/templates/config persist where expected. | Automated tests use disposable homes; personal home migration is user-risky and must be deliberate. |
| H-014 | Configure a real private git remote over SSH or HTTPS and run `#push` with disposable Aish data. | Aish uses conservative pull/add/commit/push behavior, does not create scheduler files, and does not rewrite history. | Local bare remotes are automated; real auth prompts and remote hosting behavior need human judgment. |
| H-015 | Create a real sync conflict in a disposable remote, inspect the failure, then recover manually. | Aish reports the conflict/failure and does not auto-resolve, delete tracked files, or rewrite remote history. | Deterministic conflict failure is automated; human review verifies the message is actionable. |
| H-016 | Use accessibility settings such as high contrast, large fonts, unusual fonts, or screen zoom while triggering completion, modes, and editor drafts. | Important text remains visible and non-overlapping; no UI relies only on subtle color. | Accessibility perception must be checked by a human. |
| H-017 | Force an abnormal interruption, such as closing the terminal window during Aish or killing an interactive child process. | The terminal is recoverable, and `stty sane` plus relaunch returns to normal if the OS terminated the process mid-raw-mode. | OS and terminal shutdown behavior varies; automation can only cover controlled exits. |

## Recording Failures

When a human-only test fails, record:

- Aish commit hash.
- Operating system and terminal emulator.
- Backend shell and version.
- Whether `AISH_HOME` or default `$HOME/.aish` was used.
- Exact command/key sequence.
- Expected behavior from this file.
- Actual visible behavior.
- Whether the prompt remained usable afterward.

Any reproducible failure should get an automated regression at the highest practical layer: Rust for pure logic, expect for byte-stream terminal interaction, or tmux capture for final rendered screen state.
