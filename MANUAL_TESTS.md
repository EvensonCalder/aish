# Aish Manual Test Checklist

This file lists the checks that still need a human in a real terminal. It is not
the full regression suite. Deterministic behavior belongs in Rust, expect, or
tmux automation; when a manual failure can be reproduced, add an automated
regression at the lowest practical layer.

Use `FULL_TESTS.md` for the complete catalog and `TESTING_MANUAL.md` for the
step-by-step distributed testing guide.

## Current Baseline

The default automated suite now covers ordinary shell commands, bash/zsh backend
state, shell rc hook inheritance, `clear` redraw, `exit`, prompt editing,
completion mechanics, private commands, templates, editor smoke, paste review,
local bare-repo sync, fake-GPG encryption migration, stdin-oriented passthrough,
unknown TUI passthrough, and the write-protected `rm` prompt regression.

Fish runtime support is not opt-in, but fish automated tests are opt-in because
fish may be missing and cross-platform fish behavior still needs wider version
coverage. Native Windows is not a primary target for this PTY implementation;
use WSL unless native Windows support is explicitly being tested.

Before running human checks, run at least:

```sh
cargo fmt --check
cargo build
cargo test --test pty_backend -- --nocapture
cargo test --test expect_runner -- --test-threads=1 --nocapture
cargo test --test tmux_capture -- --test-threads=1 --nocapture
git diff --check
```

Recommended opt-in fish automation when fish is installed:

```sh
AISH_TEST_FISH=1 cargo test --test pty_backend -- --nocapture
AISH_TEST_FISH=1 cargo test --test tmux_capture tmux_common_shell_workflow_matches_fish_backend_real_terminal_screen -- --nocapture
AISH_TEST_FISH=1 cargo test --test tmux_capture tmux_backend_rc_inheritance_matches_fish_real_terminal_screen -- --nocapture
AISH_TEST_FISH=1 cargo test --test tmux_capture tmux_rm_write_protected_prompt_waits_for_user_input_fish_backend -- --nocapture
```

Use an isolated home for normal manual runs:

```sh
cargo build
export AISH_MANUAL_ROOT="/tmp/aish-manual-$(date +%s)"
mkdir -p "$AISH_MANUAL_ROOT"
export AISH_HOME="$AISH_MANUAL_ROOT/home"
./target/debug/aish
```

For shell-specific checks, set both a disposable `HOME` and an isolated
`AISH_HOME` so user rc files cannot damage personal state.

## Human-Only Checks

| ID | Area | What To Do | Expected Behavior |
| --- | --- | --- | --- |
| H-001 | Bash rc compatibility | Launch with a disposable bash `HOME` whose `.bashrc` defines aliases, functions, `PATH`, `PROMPT_COMMAND`, and `PS0`. Run the alias, function, a command that reads the modified `PATH`, `clear`, and plain `exit`. | User shell state works, prompt-command side effects are preserved where meaningful, hook/PS0 noise is not displayed as command output, `clear` leaves the prompt at the correct position, and `exit` ends Aish cleanly without a backend PTY error. |
| H-002 | Zsh rc compatibility | Launch with a disposable zsh `HOME` whose `.zshrc` defines aliases, functions, `PATH`, direct `preexec`/`precmd`, and `preexec_functions`/`precmd_functions`. Run the same command/state checks as bash. | User zsh hooks still run, hook output does not leak into Aish command output, command start/finish reporting remains stable, `clear` redraws correctly, and `exit` is clean. |
| H-003 | Fish opt-in compatibility | If fish is installed, launch with a disposable fish `HOME` and `SHELL="$(command -v fish)"`. Use `config.fish` with aliases/functions, `fish_preexec`, `fish_postexec`, prompt functions, and `PATH`. Repeat command, `clear`, and `exit` checks. | Fish either matches bash/zsh user-visible behavior or the exact fish version/platform difference is recorded. A fish-only failure is a compatibility issue, not a default-suite failure unless fish was explicitly required for the platform. |
| H-004 | Nested shell foreground behavior | From Aish, run `bash`; inside it run `bash`, `zsh`, or fish if installed; run `clear`, `sleep 10` then `Ctrl-C`, a few normal commands, then exit each layer one at a time. | The foreground program owns input while active. Aish does not interpret child-shell keys, does not rely on prompt guessing, and recovers only after the foreground shell exits. |
| H-005 | TTY stdin prompts | Run stdin/confirmation commands such as `cat`, `grep needle`, `sed 's/a/A/'`, `awk '{print $1}'`, and a disposable write-protected `rm` prompt. Use EOF or the program's expected answer to exit. | Prompts are visible before the answer is typed, child echo/input behavior is normal, EOF works, and Aish prompt returns usable. |
| H-006 | Full-screen and alternate-screen programs | Run real interactive tools available on the machine: `vim`/`nvim`, `less`, `top`, `fzf`, `node` or another REPL, `ssh` to a disposable or intentionally invalid target, and nested `tmux`/`screen` if available. | The program receives normal keys, alternate screen state is restored, no timeout is needed, and Aish redraws a usable prompt after the child exits. |
| H-007 | Real editor flow | Use the normal editor through `Ctrl-X Ctrl-E`. Test save-and-return, content beginning with `#`, a large draft, and editor failure or kill from another terminal if practical. | Editor content does not execute until explicit `Enter`; editor-submitted `#` text follows the editor/raw-shell path; failures preserve the previous draft where possible and leave the terminal recoverable. |
| H-008 | Real OS clipboard paste | Paste one single-line command and then several lines from the OS clipboard in at least one real terminal emulator. | Single-line paste inserts without executing. Multiline paste enters the review/editor flow and never silently executes. |
| H-009 | Real `fzf` pickers | With real `fzf`, confirm and cancel history, file, template, git-branch, and environment-variable pickers, including paths with spaces. | Confirmed choices insert shell-safe text; canceled pickers preserve the previous draft and terminal layout. |
| H-010 | Completion visual quality | In light and dark themes, type prefixes that show inline completion, below-prompt rows, long candidates near the right edge, and `Tab` full/word acceptance. Repeat in a narrow terminal and with a large font if possible. | Suggestions are readable but clearly uncommitted, text does not overlap, rows elide with `...`, and acceptance behavior feels predictable. |
| H-011 | Private history/template/AI/draft listing privacy | Create disposable history, drafts, and templates; create AI history only if a disposable endpoint is available. Run `#history list`, `#ai list`, `#draft list`, `#template list`, each matching `search`, and the `>` / `|` export forms. Accept and reject confirmations. | List/search output is one item per line, including the empty-list case. Export or pipe of private content asks for confirmation first, refusal writes/runs nothing, and accepted output goes only to the requested file or command. |
| H-012 | Real GPG pinentry and encryption lifecycle | With isolated `GNUPGHOME`, disposable passphrase-protected test keys, and isolated `AISH_HOME`, test `#encrypt on <fingerprint>`, `#encrypt unlock-mode prompt`, restart and unlock, `#key set`, optional key rotation, `#encrypt off`, and direct `gpg` passthrough. | Pinentry gets terminal control safely, secrets are not printed, encrypted storage uses `*.jsonl.gpg`, plaintext managed JSONL files disappear after successful encryption, unlock failures are readable, and terminal state recovers after every prompt. |
| H-013 | Real AI endpoint and context privacy | Only with a disposable OpenAI-compatible endpoint/key, submit AI prompts that return command JSON and run safe context pseudo-pipes with confirmation on and off. | Returned commands are shown for selection and never auto-execute. Context output is redacted/truncated as configured, and dangerous context commands still require confirmation. |
| H-014 | Sync with real auth, optional | Use a non-production SSH or HTTPS remote only when the tester intentionally has credentials. Also test the local bare-repo path if real auth is unavailable. Create an unmanaged conflict and recover manually; with encrypted sync enabled, create independent managed history appends and confirm Aish auto-unions the encrypted JSONL payload. Toggle `#sync quiet on|off` and leave a periodic schedule enabled during normal typing. | Lack of GitHub or remote credentials is not a failure. With real auth, prompts do not wedge Aish. Manual/startup/periodic sync keeps the prompt usable, routine quiet sync stays silent, failures remain visible, Aish only auto-resolves managed append-only JSONL conflicts, does not auto-resolve unmanaged conflicts, does not delete tracked files, does not create scheduler files, and does not rewrite remote history. |
| H-015 | Fedora/openEuler and non-Debian Linux smoke | On Fedora-family systems, including openEuler when available, run the default automated suite plus bash/zsh real-terminal smoke. Record whether the tester is root, package versions, locale, and terminal emulator. | Git output wording or missing GitHub auth is not treated as product failure. Startup errors, unwritable-home errors, `rm` prompts, shell rc hooks, `clear`, and passthrough behavior are recorded exactly if they differ. |
| H-016 | Cross-terminal and cross-platform rendering | Repeat a short smoke workflow in representative terminals: Terminal.app, iTerm2, Ghostty, Alacritty, GNOME Terminal, Konsole, and WSL terminal hosts where available. | Prompt redraw, Unicode, completion, paste, and passthrough behavior remain stable across real terminal emulators. |
| H-017 | Production-shaped home | After isolated tests pass, use a disposable normal-looking `HOME` with `AISH_HOME` unset. Optionally inspect a personal `~/.aish` only after backing it up and without destructive sync/encryption. | Aish creates or reuses `$HOME/.aish` only, does not touch unrelated files, and persists history/templates/config/logs as expected. |
| H-018 | Backend native history privacy | With disposable bash, zsh, and fish homes, create preexisting native history files and rc configs that enable normal native history saving. Launch Aish, run unique commands, run the shell-native history query and forced save command for each backend (`history`/`history -a`, `fc -l`/`fc -W`, `history search`/`history save`). | Unique Aish-submitted commands appear in Aish regular history only. They do not appear in backend native history query output, native history files stay unchanged, and preexisting native history content is not deleted. |
| H-019 | Abnormal interruption and recovery | Close the terminal while Aish is running, kill an interactive child from another terminal, kill GPG/pinentry if used, and recover with `Ctrl-C`, `Ctrl-D`, `Ctrl-L`, `stty sane`, or `reset` as needed. | The recovery path is clear. Any terminal wedge, lost input, stuck raw mode, or child process leak is recorded as high priority. |

## Failure Report Requirements

For every failure, record:

- Aish commit hash, or state that the source package has no `.git` metadata.
- Operating system, kernel, architecture, terminal emulator, and locale.
- Backend shell path and version.
- Whether the tester is root.
- Whether `AISH_HOME`, disposable `HOME`, or personal `$HOME/.aish` was used.
- Exact command/key sequence.
- Expected behavior from this file.
- Actual visible behavior, including screenshots for rendering issues.
- Whether the prompt remained usable afterward and what recovery step worked.

Any reproducible failure should become an automated regression unless it depends
on real credentials, real pinentry UI, human visual judgment, or terminal
emulator rendering that cannot be captured reliably.
