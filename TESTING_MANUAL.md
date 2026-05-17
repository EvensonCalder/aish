# Aish Distributed Testing Manual

This manual explains how to guide a tester through Aish validation on a real machine. It is written so a human can follow it directly or give it to an AI assistant and ask for step-by-step guidance.

Use `FULL_TESTS.md` as the complete checklist. Use this file as the process guide.

## AI Assistant Protocol

When guiding a human tester, follow these rules:

1. Ask for one action at a time.
2. Stop whenever the tester must run a command, press keys, inspect the terminal, or paste output.
3. Do not send a long batch of steps unless the tester explicitly asks for a batch.
4. Record exact command output, screenshots, visible symptoms, and recovery steps.
5. Use isolated `AISH_HOME` directories by default.
6. Do not ask the tester to use personal `~/.aish` until isolated tests pass.
7. Do not ask for real API keys, real private git remotes, or real GPG keys unless the tester explicitly chooses those optional tests.
8. Treat hangs, terminal wedges, unintended execution, secret leakage, and lost input as high-priority failures.
9. At the end, write a human-readable report in English using the template in this manual.

Ready-to-use prompt for a tester to give another AI:

```text
I need to test Aish on my machine. Use TESTING_MANUAL.md and FULL_TESTS.md from the repository.

Guide me step by step. Do not send many steps at once. Whenever I need to run a command, press keys, inspect a terminal, or paste output, stop and wait for my result before continuing.

Use an isolated AISH_HOME by default. Record my platform, tool versions, automated test results, manual test results, failures, recovery steps, and skipped optional checks. At the end, write a human test report in English.
```

## What The Tester Needs

Required:

- macOS or Linux terminal.
- Git.
- Rust toolchain with `cargo`.
- A local copy of the Aish repository.
- Bash and zsh for baseline shell compatibility testing.

Recommended:

- `tmux` for screen-capture integration tests.
- `expect` for expect-driven terminal scenarios.
- `fzf` for picker tests.
- `python3` for REPL passthrough tests.
- `gpg` for current encryption checks and passthrough recovery.
- Fish for opt-in fish backend compatibility testing.

Native Windows is not a primary target for this PTY implementation. Windows testers should use WSL unless native Windows support is explicitly added later.

## Report File

Create or ask the AI assistant to maintain a report named:

```text
AISH_TEST_REPORT_<platform>_<date>.md
```

Use this structure:

```markdown
# Aish Test Report

## Basic Information

| Item | Value |
| --- | --- |
| Tester |  |
| Date |  |
| Aish commit |  |
| OS and version |  |
| Kernel |  |
| CPU architecture |  |
| Terminal emulator and version |  |
| Backend shell(s) tested |  |
| Rust version |  |
| Cargo version |  |
| Git version |  |
| tmux version |  |
| expect version |  |
| fzf version |  |
| fish version, if tested |  |
| Python version |  |
| GPG version |  |
| Isolated AISH_HOME |  |
| Disposable HOME used |  |

## Automated Test Summary

| Test | Result | Notes |
| --- | --- | --- |
| cargo fmt --check |  |  |
| cargo build |  |  |
| cargo test --lib |  |  |
| cargo test --test draft_execution |  |  |
| cargo test --test first_run |  |  |
| cargo test --test pty_backend |  |  |
| cargo test --test expect_runner |  |  |
| cargo test --test tmux_capture |  |  |
| cargo clippy --all-targets -- -D warnings |  |  |
| git diff --check |  |  |
| fish opt-in tests |  |  |

## Manual Test Summary

| Area | Result | Notes |
| --- | --- | --- |
| Startup and storage |  |  |
| Ordinary shell commands |  |  |
| Bash backend |  |  |
| Zsh backend |  |  |
| Fish backend |  |  |
| Prompt editing |  |  |
| Modes and history |  |  |
| Completion |  |  |
| Continuation |  |  |
| Private commands and notes |  |  |
| Templates |  |  |
| Editor and paste |  |  |
| fzf pickers |  |  |
| AI and context |  |  |
| Interactive passthrough |  |  |
| Sync |  |  |
| Encryption and GPG |  |  |
| Visual/accessibility checks |  |  |

## Detailed Results

Add notes for each executed test ID from FULL_TESTS.md.

## Failures

Use the failure template from FULL_TESTS.md for each failure.

## Skipped Tests

List every skipped test and the reason.

## Final Assessment

State whether this platform is acceptable, acceptable with issues, or blocked.
```

## Step 1: Confirm Repository And Commit

Ask the tester to open a terminal in the Aish repository root.

Run:

```sh
pwd
git rev-parse --show-toplevel
git rev-parse HEAD
git status --short
```

Stop and record the output.

Expected:

- The top-level path is the Aish repository.
- The commit hash is recorded.
- The worktree state is recorded. Uncommitted changes are allowed only if the tester intentionally tests them.

## Step 2: Identify Platform

Run:

```sh
uname -a
```

Stop and record the output.

Then choose the OS-specific command.

For macOS:

```sh
sw_vers
```

For Linux:

```sh
cat /etc/os-release
```

Stop and record the output.

If `/etc/os-release` is missing, use:

```sh
lsb_release -a
```

## Step 3: Identify Terminal Emulator

Ask the tester to record the terminal emulator and version from the terminal application's About dialog or version command.

Examples:

- Terminal.app on macOS.
- iTerm2.
- Ghostty.
- Alacritty.
- GNOME Terminal.
- Konsole.
- Windows Terminal with WSL.

Stop and record the terminal name and version.

## Step 4: Check Required Tools

Run one command at a time and record each output.

```sh
git --version
```

```sh
rustc --version
```

```sh
cargo --version
```

```sh
bash --version
```

```sh
zsh --version
```

Optional tools:

```sh
tmux -V
```

```sh
expect -v
```

```sh
fzf --version
```

```sh
fish --version
```

```sh
python3 --version
```

```sh
gpg --version
```

Stop after missing tools and record which tests will be skipped or which installation step is needed.

## Step 5: Install Missing Tools

If Rust is missing, install Rust with rustup unless the tester's platform policy requires a package manager:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

After installation, open a new terminal or load the cargo environment according to rustup's message, then run:

```sh
rustc --version
cargo --version
```

On macOS, if linking fails because command line tools are missing, run:

```sh
xcode-select --install
```

On Debian or Ubuntu, common tools can be installed with:

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config git tmux expect fzf zsh fish python3 gpg
```

On Fedora:

```sh
sudo dnf install -y gcc pkgconf-pkg-config git tmux expect fzf zsh fish python3 gnupg2
```

On Arch Linux:

```sh
sudo pacman -S --needed base-devel pkgconf git tmux expect fzf zsh fish python gpgme gnupg
```

Stop after installation and rerun the version checks for installed tools.

## Step 6: Build Aish

Run:

```sh
cargo build
```

Stop and record whether the build passed.

Expected:

- The command finishes successfully.
- `target/debug/aish` exists.

If the build fails, record the full error, platform, Rust version, and stop unless the tester wants to fix local dependencies.

## Step 7: Run Automated Tests

Start with the smaller required tests. Run one command at a time.

```sh
cargo fmt --check
```

```sh
cargo test --lib
```

```sh
cargo test --test draft_execution -- --nocapture
```

```sh
cargo test --test first_run -- --nocapture
```

```sh
cargo test --test pty_backend -- --nocapture
```

Stop after each command and record pass, fail, or skip.

If the tester has `expect`, run:

```sh
cargo test --test expect_runner -- --test-threads=1 --nocapture
```

If the tester has `tmux`, run:

```sh
cargo test --test tmux_capture -- --test-threads=1 --nocapture
```

If the tester has fish and agrees to opt-in fish validation, run:

```sh
AISH_TEST_FISH=1 cargo test --test pty_backend -- --nocapture
```

```sh
AISH_TEST_FISH=1 cargo test --test tmux_capture tmux_common_shell_workflow_matches_fish_backend_real_terminal_screen -- --nocapture
```

Finish with:

```sh
cargo clippy --all-targets -- -D warnings
```

```sh
git diff --check
```

Expected:

- Required tests pass.
- Missing optional tools are recorded as skipped.
- Fish failures are recorded with fish version, OS, and terminal details.

## Step 8: Start An Isolated Manual Session

Run:

```sh
export AISH_MANUAL_ROOT="/tmp/aish-manual-$(date +%s)"
mkdir -p "$AISH_MANUAL_ROOT"
export AISH_HOME="$AISH_MANUAL_ROOT/home"
echo "$AISH_HOME"
./target/debug/aish
```

Stop when the Aish prompt appears.

Expected:

- Aish starts with a prompt similar to `<user>@ <dir> >`.
- The exact prompt may differ because prompt config and path are platform-dependent.

If the prompt does not appear, record the terminal output and stop.

## Step 9: Run First Manual Smoke Commands

Inside Aish, run one command at a time:

```text
#doctor
```

```text
#status
```

```text
pwd
```

```text
cd /tmp
```

```text
pwd
```

```text
printf 'alpha\nbeta\ngamma\n'
```

Expected:

- Private commands print diagnostics and return to a usable prompt.
- `cd /tmp` persists.
- `printf` prints exactly three lines.

Stop after these commands and record output or symptoms.

## Step 10: Test Ordinary Command Behavior

Use the `CMD-*` section of `FULL_TESTS.md`.

Guide the tester through a small group at a time:

1. Basic output and stderr.
2. Exit status using backend-specific syntax.
3. Files with spaces.
4. Pipelines.
5. Unicode.
6. Interrupt and clear-screen behavior.

Stop after each group and record results.

## Step 11: Test Bash And Zsh Backends

Exit Aish first:

```text
#exit
```

For bash:

```sh
SHELL=/bin/bash AISH_HOME="$AISH_MANUAL_ROOT/bash-home" ./target/debug/aish
```

Inside Aish, run the `SH-001` to `SH-003` tests from `FULL_TESTS.md`, then exit.

For zsh:

```sh
SHELL=/bin/zsh AISH_HOME="$AISH_MANUAL_ROOT/zsh-home" ./target/debug/aish
```

Inside Aish, run the `SH-004` to `SH-006` tests from `FULL_TESTS.md`, then exit.

Expected:

- Bash and zsh both preserve cwd and environment state.
- Aish prompt, completion basics, and command execution do not depend on a specific backend shell.

Stop and record results for each backend.

## Step 12: Optional Fish Backend

Only run this if `fish --version` works and the tester agrees to opt-in compatibility testing.

Run:

```sh
SHELL="$(command -v fish)" AISH_HOME="$AISH_MANUAL_ROOT/fish-home" ./target/debug/aish
```

Inside Aish, run the `SH-007` to `SH-009` tests from `FULL_TESTS.md`.

Expected:

- Fish either matches the bash/zsh user-visible behavior for the tested workflows or produces a clearly recorded compatibility issue.
- Record fish version, OS, terminal, and exact visible output for any difference.

Stop and record results.

## Step 13: Prompt Editing, Modes, And History

Use `EDIT-*` and `MODE-*` from `FULL_TESTS.md`.

Guide the tester in small groups:

- Cursor movement and text insertion.
- Deletion keys.
- Word movement.
- `Esc`, `Ctrl-D`, and unsupported `Ctrl-X` chord.
- Empty `Tab` mode cycling.
- History browsing, editing, execution, and persistence.

Stop after each group and record results.

## Step 14: Completion

Use `COMP-*` from `FULL_TESTS.md`.

Important expected behavior:

- Inline completion is enabled by default and updates while typing.
- `completion.max_results` controls only below-prompt rows.
- Below-prompt rows should not duplicate the active inline suggestion.
- `completion.inline=false` makes non-empty `Tab` accept the first ranked candidate directly.
- `completion.tab_accept=full` accepts the whole inline suffix.
- `completion.tab_accept=word` accepts only through the next whitespace boundary.
- Long rows should stay within terminal width and elide with `...`.

Stop after each of these groups:

1. Live inline suggestion.
2. Full accept mode.
3. Word accept mode.
4. Inline disabled mode.
5. Below-prompt row count.
6. Narrow terminal behavior.
7. Bash/zsh consistency.
8. Optional fish consistency.

Record screenshots or descriptions for visual checks.

## Step 15: Continuation

Use `CONT-*` from `FULL_TESTS.md`.

Test double quotes, single quotes, trailing backslash, and `Ctrl-C` cancellation.

Expected:

- Aish handles continuation coherently.
- Backend secondary prompt noise does not leak into final output.
- `Ctrl-C` cancels continuation and returns to a usable prompt.

Stop and record results.

## Step 16: Private Commands, Notes, And Templates

Use `PRIV-*` and `TPL-*` from `FULL_TESTS.md`.

Run in small groups:

- `#help`, unknown command, `#log`.
- AI config commands without real secrets.
- Notes.
- Encryption commands without real keys.
- Template create/list/show/use/replace/remove.

Expected:

- Line-leading `#` private commands are not sent to the backend shell.
- Recognized note lines are stored as notes.
- Templates block unresolved placeholders.
- Encryption commands without configured keys fail safely and do not print secrets.

Stop and record results.

## Step 17: Editor And Paste

Use `EDITOR-*` from `FULL_TESTS.md`.

Before testing, choose a real editor:

```sh
export EDITOR=vim
```

or:

```sh
export EDITOR=nvim
```

Then start Aish with the same isolated home or a new isolated home.

Test:

- `Ctrl-X Ctrl-E` save-and-return.
- Editor execution only after `Enter`.
- Editor failure or external kill if practical.
- Single-line clipboard paste.
- Multiline clipboard paste.
- Editor content beginning with `#`.

Expected:

- No pasted or editor content executes before explicit confirmation or `Enter`.
- The prompt recovers after editor success and failure.
- Real clipboard behavior is recorded because terminals differ.

Stop after each operation and record results.

## Step 18: Fzf Pickers

Only run this section if `fzf` is installed.

Use `FZF-*` from `FULL_TESTS.md`.

Test each picker in confirm and cancel paths:

- History picker: `Ctrl-R`.
- File picker: `Ctrl-X Ctrl-F`.
- Template picker: `Ctrl-X Ctrl-T`.
- Git branch picker: `Ctrl-X Ctrl-B`.
- Environment variable picker: `Ctrl-X Ctrl-V`.

Expected:

- Confirmed selections insert or replace the intended text.
- Canceled pickers preserve the previous draft.
- Paths and environment variables are inserted safely.

Stop after each picker and record results.

## Step 19: AI And Context

Use `AI-*` from `FULL_TESTS.md`.

Default safe path:

- Test config commands.
- Test provider failure with a disposable invalid endpoint.
- Test context confirmation behavior with safe local commands.
- Do not use real API keys unless the tester explicitly chooses a disposable endpoint and key.

Expected:

- AI failures are readable.
- AI output never auto-executes.
- Context commands require confirmation by default.
- Dangerous context patterns remain protected.
- Secret-shaped values are redacted.

Stop and record sanitized output.

## Step 20: Interactive And Stdin Passthrough

Use `PASS-*` from `FULL_TESTS.md`.

Start with the critical stdin and REPL checks:

- `python3`.
- `cat`.
- `grep`.
- `sed`.
- `awk`.
- `less README.md` if available.
- `gpg` with no arguments or `gpg --version`.

Then test optional full-screen or network-adjacent tools only when safe:

- `vim` or `nvim`.
- `top`.
- `node`.
- Nested `tmux` or `screen`.
- Disposable or intentionally invalid `ssh`.

Expected:

- Programs that expect stdin or terminal control do not wedge Aish.
- `Ctrl-C`, `Ctrl-D`, the program's normal quit command, or process exit returns to a usable Aish prompt.

Stop after each program and record:

- How it was exited.
- Whether prompt returned.
- Whether `Ctrl-C`, `Ctrl-D`, `Ctrl-L`, `#exit`, or `stty sane` was needed.

## Step 21: Sync

Use `SYNC-*` from `FULL_TESTS.md`.

Only use disposable repositories.

Suggested local setup outside Aish:

```sh
export AISH_SYNC_REMOTE="$AISH_MANUAL_ROOT/aish-sync-remote.git"
git init --bare "$AISH_SYNC_REMOTE"
```

Inside Aish:

```text
#set-remote <path-to-the-bare-repo>
```

Then create some Aish data and run:

```text
#push
```

Expected:

- Aish pushes managed data conservatively.
- A repeated push with no changes does not create unnecessary commits.
- Conflict tests fail safely and do not rewrite history.
- No scheduler files are created.

Stop and record output.

## Step 22: Encryption And GPG

Use `ENC-*` from `FULL_TESTS.md`.

Use isolated state only:

- Use a disposable `AISH_HOME`.
- Use an isolated `GNUPGHOME`.
- Use disposable GPG keys only.
- Do not use a personal API key or personal `~/.aish`.

Current expected state:

- Before a key is configured, `#key set` and `#encrypt on` report the missing key safely and do not print secrets.
- With a disposable key, `#encrypt on <fingerprint>` migrates managed JSONL storage to `*.jsonl.gpg` and removes plaintext JSONL files after successful encryption.
- `#key set` stores the configured environment API key encrypted when an encryption key fingerprint is configured.
- `#encrypt rotate <fingerprint>` re-encrypts current managed storage for the new key when a second disposable key is available.
- `#encrypt off` decrypts current managed storage back to plaintext and leaves the prompt usable.
- Running real `gpg` must not wedge Aish.
- Pinentry/passphrase prompts must get terminal control and recover cleanly.

Stop and record output.

## Step 23: Visual, Accessibility, And Cross-Terminal Checks

Use `TERM-*` from `FULL_TESTS.md`.

Run the most important checks:

- Light theme and dark theme inline completion readability.
- Narrow terminal long command redraw.
- Completion near the right edge.
- Resize while a draft is visible.
- Large font or high contrast setting.
- At least one additional terminal emulator when available.

Expected:

- No overlapping prompt text.
- No duplicated prompt fragments.
- Inline completion is readable but visually distinct.
- Below-prompt hints stay in one line per candidate and elide when needed.

Stop and record screenshots or notes.

## Step 24: Production-Shaped Home

Only run this after isolated tests pass.

Use a disposable `HOME`, not the tester's real home:

```sh
export AISH_TEST_HOME="/tmp/aish-default-home-$(date +%s)"
mkdir -p "$AISH_TEST_HOME"
unset AISH_HOME
HOME="$AISH_TEST_HOME" ./target/debug/aish
```

Inside Aish:

```text
#doctor
echo default-home-ok
#exit
```

Then inspect:

```sh
find "$AISH_TEST_HOME" -maxdepth 3 -type f -o -type d
```

Expected:

- Aish creates `$HOME/.aish`.
- Aish does not touch unrelated files.
- State persists if relaunched with the same disposable `HOME`.

Stop and record output.

## Step 25: Write Final Report

The AI assistant should now write the report in English.

The report must include:

- Basic information table.
- Automated test summary.
- Manual test summary.
- Detailed results by test ID or feature area.
- Failures with severity and reproduction steps.
- Skipped tests and reasons.
- Recovery notes for hangs or terminal corruption.
- Final assessment:
  - `Acceptable`
  - `Acceptable with issues`
  - `Blocked`

Final assessment guidance:

- Use `Blocked` if startup, build, normal command execution, terminal recovery, bash/zsh baseline, or critical passthrough fails.
- Use `Acceptable with issues` if the platform mostly works but has fish-only, visual, optional tool, or non-critical feature failures.
- Use `Acceptable` only when required tests pass and optional skips are clearly justified.

## Minimal Emergency Recovery Notes

If the terminal appears broken after a failure:

1. Try `Ctrl-C`.
2. Try `Ctrl-D`.
3. Try `Ctrl-L`.
4. Try closing the Aish process from another terminal.
5. In the affected terminal, run:

```sh
stty sane
reset
```

Record which recovery step worked.

## What To Send Back To Maintainers

Send:

- The final report.
- The Aish commit hash.
- Full failure details.
- Screenshots for visual/rendering issues.
- Exact shell and terminal versions.
- Whether the problem reproduces in bash, zsh, fish, or all backends.
- Whether the problem reproduces with isolated `AISH_HOME`.
- The shortest known reproduction sequence.
