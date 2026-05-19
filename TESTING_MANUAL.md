# Aish Distributed Testing Guide

This guide explains how to run Aish validation on a real machine and how to
write a useful report. It is meant for a human tester or for an AI assistant
guiding a human one step at a time.

Use these documents together:

- `FULL_TESTS.md`: complete catalog of automated and manual feature checks.
- `MANUAL_TESTS.md`: human-only checklist for terminal, shell, GPG, auth, and
  visual behavior that automation cannot fully prove.
- `TESTS.md`: maintainer-facing coverage map and latest test inventory.

Aish is a PTY-based command input layer, not a shell. The backend shell owns
shell syntax, shell state, job control, and command execution. A good platform
test must therefore verify both Aish UI behavior and backend shell transparency.

## Assistant Protocol

When guiding a tester:

1. Ask for one action at a time.
2. Stop whenever the tester must run a command, press keys, inspect output, or
   paste a result.
3. Use isolated `AISH_HOME` by default.
4. Do not ask for personal API keys, personal GPG keys, or a personal
   `~/.aish`.
5. Do not require GitHub or remote git authentication. Local bare repositories
   are enough for normal sync validation.
6. Treat terminal wedges, lost input, unintended execution, secret leakage,
   stuck raw mode, and passthrough failures as high-priority issues.
7. Record exact commands, keys, visible output, terminal screenshots when useful,
   and recovery steps.
8. At the end, write the report in English.

Ready-to-use prompt for a tester to give another AI:

```text
I need to test Aish on my machine. Use TESTING_MANUAL.md, FULL_TESTS.md, and MANUAL_TESTS.md from the repository.

Guide me step by step. Do not send many steps at once. Whenever I need to run a command, press keys, inspect output, or paste a result, stop and wait.

Use isolated AISH_HOME directories by default. Do not ask for personal API keys, personal GPG keys, or a personal ~/.aish unless I explicitly opt in. Do not require GitHub authentication. At the end, write a report in English with platform details, automated test results, manual test results, failures, recovery steps, skipped checks, and final assessment.
```

## Required Scope

Required for a normal platform pass:

- Linux or macOS real terminal.
- Bash and zsh backend smoke.
- Default automated tests that match installed tools.
- Manual smoke for foreground passthrough and terminal recovery.

Recommended:

- One Debian/Ubuntu-family Linux and one non-Debian Linux such as Fedora or
  openEuler when available.
- At least one macOS terminal when available.
- `tmux`, `expect`, `fzf`, `python3`, `gpg`, and fish.

Fish is supported at runtime, but fish test coverage is opt-in because fish may
not be installed and fish behavior differs across versions. Native Windows is
not a primary target for this PTY implementation; use WSL unless native Windows
support is explicitly being evaluated.

## Report Template

Create a report named:

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
| Source package has .git metadata | Yes / No |
| OS and version |  |
| Kernel |  |
| CPU architecture |  |
| Locale |  |
| Tester is root | Yes / No |
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
| Isolated AISH_HOME used |  |
| Disposable HOME used |  |
| Real remote auth tested | Yes / No |
| Real pinentry tested | Yes / No |

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
| fish opt-in pty_backend |  |  |
| fish opt-in tmux smoke |  |  |
| cargo clippy --all-targets -- -D warnings |  |  |
| git diff --check |  |  |

## Manual Test Summary

| Area | Result | Notes |
| --- | --- | --- |
| Bash rc compatibility |  |  |
| Zsh rc compatibility |  |  |
| Fish compatibility, if tested |  |  |
| Nested shell foreground behavior |  |  |
| Stdin prompts and write-protected rm |  |  |
| Full-screen passthrough |  |  |
| Editor and paste |  |  |
| fzf pickers |  |  |
| Completion visual quality |  |  |
| Private list/export privacy |  |  |
| Real GPG/pinentry |  |  |
| Real AI endpoint, if tested |  |  |
| Sync local bare remote |  |  |
| Sync real remote auth, if tested |  |  |
| Cross-terminal rendering |  |  |
| Production-shaped HOME |  |  |
| Abnormal recovery |  |  |

## Failures

Use the failure template from this guide for every failure.

## Skipped Tests

List each skipped test and why.

## Final Assessment

Acceptable / Acceptable with issues / Blocked
```

## Step 1: Identify Repository And Commit

Run from the Aish repository root:

```sh
pwd
git rev-parse --show-toplevel
git rev-parse HEAD
git status --short
```

If the source was unpacked from a zip and has no `.git`, record that clearly and
record any visible version, archive name, or checksum instead. Lack of `.git`
metadata is not a product failure, but reports are harder to compare.

Expected:

- The tester is in the Aish repository root.
- The commit hash or source-package limitation is recorded.
- The worktree state is recorded.

## Step 2: Identify Platform

Run:

```sh
uname -a
locale
id -u
```

For Linux:

```sh
cat /etc/os-release
```

If `/etc/os-release` is missing:

```sh
lsb_release -a
```

For macOS:

```sh
sw_vers
```

Ask the tester to record the terminal emulator name and version from the About
dialog or a version command. Examples: Terminal.app, iTerm2, Ghostty, Alacritty,
GNOME Terminal, Konsole, Windows Terminal with WSL.

For Fedora/openEuler-family systems, also record whether the tester is root and
which package manager supplied `git`, `tmux`, `expect`, `fish`, and `gpg`.

## Step 3: Check Tool Versions

Run one command at a time and record output or missing-tool notes:

```sh
git --version
rustc --version
cargo --version
bash --version
zsh --version
tmux -V
expect -v
fzf --version
fish --version
python3 --version
gpg --version
```

Missing optional tools should be recorded as `SKIP`, not treated as product
failures.

Common package commands:

```sh
# Debian / Ubuntu
sudo apt-get update
sudo apt-get install -y build-essential pkg-config git tmux expect fzf zsh fish python3 gpg
```

```sh
# Fedora / openEuler-style systems
sudo dnf install -y gcc pkgconf-pkg-config git tmux expect fzf zsh fish python3 gnupg2
```

```sh
# Arch Linux
sudo pacman -S --needed base-devel pkgconf git tmux expect fzf zsh fish python gpgme gnupg
```

Use rustup for Rust unless the platform policy requires a system Rust:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Step 4: Build And Run Automated Tests

Start with:

```sh
cargo build
cargo fmt --check
cargo test --lib
cargo test --test draft_execution -- --nocapture
cargo test --test first_run -- --nocapture
cargo test --test pty_backend -- --nocapture
```

If `expect` is installed:

```sh
cargo test --test expect_runner -- --test-threads=1 --nocapture
```

If `tmux` is installed:

```sh
cargo test --test tmux_capture -- --test-threads=1 --nocapture
```

If fish is installed and the tester agrees to opt-in fish validation:

```sh
AISH_TEST_FISH=1 cargo test --test pty_backend -- --nocapture
AISH_TEST_FISH=1 cargo test --test tmux_capture tmux_common_shell_workflow_matches_fish_backend_real_terminal_screen -- --nocapture
AISH_TEST_FISH=1 cargo test --test tmux_capture tmux_backend_rc_inheritance_matches_fish_real_terminal_screen -- --nocapture
AISH_TEST_FISH=1 cargo test --test tmux_capture tmux_rm_write_protected_prompt_waits_for_user_input_fish_backend -- --nocapture
```

Finish with:

```sh
cargo clippy --all-targets -- -D warnings
git diff --check
```

Expected:

- Required tests pass.
- Optional-tool skips are explicit.
- Fish failures include fish version, OS, terminal, and exact output.
- `git init` wording differences or missing GitHub credentials are not treated
  as product failures by themselves.

## Step 5: Start Isolated Manual Sessions

Create an isolated root:

```sh
export AISH_MANUAL_ROOT="/tmp/aish-manual-$(date +%s)"
mkdir -p "$AISH_MANUAL_ROOT"
```

Default launch:

```sh
export AISH_HOME="$AISH_MANUAL_ROOT/home"
./target/debug/aish
```

Backend-specific launches:

```sh
HOME="$AISH_MANUAL_ROOT/bash-user" SHELL=/bin/bash AISH_HOME="$AISH_MANUAL_ROOT/bash-home" ./target/debug/aish
HOME="$AISH_MANUAL_ROOT/zsh-user" SHELL=/bin/zsh AISH_HOME="$AISH_MANUAL_ROOT/zsh-home" ./target/debug/aish
HOME="$AISH_MANUAL_ROOT/fish-user" SHELL="$(command -v fish)" AISH_HOME="$AISH_MANUAL_ROOT/fish-home" ./target/debug/aish
```

Only run the fish command when `command -v fish` succeeds.

First smoke commands inside Aish:

```text
#doctor
#status
pwd
cd /tmp
pwd
printf 'alpha\nbeta\ngamma\n'
clear
exit
```

Expected:

- Private commands print diagnostics and return to a usable prompt.
- `cd /tmp` persists.
- `printf` prints exactly three lines.
- `clear` redraws the prompt at the correct position.
- Plain `exit` exits cleanly.

## Step 6: Shell Rc Compatibility

Run `MANUAL_TESTS.md` H-001, H-002, and H-003.

Minimum bash rc content for a disposable `HOME/.bashrc`:

```sh
alias aish_manual_alias='printf bash-alias-ok\\n'
aish_manual_func(){ printf bash-function-ok\\n; }
export PATH="$HOME/bin:$PATH"
PROMPT_COMMAND='export AISH_MANUAL_PROMPT_COMMAND=ran; printf bash-prompt-noise\\n'
PS0=$'bash-ps0-noise\n'
```

Minimum zsh rc content for a disposable `HOME/.zshrc`:

```sh
alias aish_manual_alias='printf zsh-alias-ok\\n'
aish_manual_func(){ printf zsh-function-ok\\n; }
export PATH="$HOME/bin:$PATH"
aish_manual_preexec(){ export AISH_MANUAL_ZSH_PREEXEC="$1"; printf zsh-preexec-noise\\n; }
aish_manual_precmd(){ export AISH_MANUAL_ZSH_PRECMD=ran; printf zsh-precmd-noise\\n; }
typeset -ga preexec_functions precmd_functions
preexec_functions+=(aish_manual_preexec)
precmd_functions+=(aish_manual_precmd)
```

Minimum fish config for a disposable `HOME/.config/fish/config.fish`:

```fish
alias aish_manual_alias 'printf fish-alias-ok\n'
function aish_manual_func
    printf fish-function-ok\n
end
set -gx PATH "$HOME/bin" $PATH
function aish_manual_preexec --on-event fish_preexec
    set -gx AISH_MANUAL_FISH_PREEXEC $argv[1]
    printf fish-preexec-noise\n
end
function aish_manual_postexec --on-event fish_postexec
    set -gx AISH_MANUAL_FISH_POSTEXEC ran
    printf fish-postexec-noise\n
end
function fish_prompt
    printf 'fish-user-prompt> '
end
```

Before launching each backend, set `TEST_HOME` to the disposable backend home
you are preparing, such as `$AISH_MANUAL_ROOT/bash-user`, then create the path
command there:

```sh
mkdir -p "$TEST_HOME/bin"
printf '#!/bin/sh\nprintf path-command-ok\\\\n\n' > "$TEST_HOME/bin/aish-manual-path"
chmod +x "$TEST_HOME/bin/aish-manual-path"
```

Inside each backend, run the alias, function, a command from `$HOME/bin`,
backend-specific status check after `false`, `clear`, and `exit`.

Expected:

- User shell state is available.
- Hook side effects are preserved where meaningful.
- Hook prompt noise does not appear as command output.
- Aish prompt remains coherent after `clear` and command failures.

## Step 7: Foreground Passthrough

Run `MANUAL_TESTS.md` H-004, H-005, and H-006.

Critical sequence:

```text
bash
bash
clear
sleep 10
```

Press `Ctrl-C`, run a simple command, then `exit` one shell layer at a time.
Repeat with zsh and fish when installed.

Stdin prompt checks:

```text
cat
grep needle
sed 's/a/A/'
awk '{print $1}'
```

Use `Ctrl-D` to finish each command.

Write-protected prompt check:

```text
mkdir -p "$AISH_MANUAL_ROOT/rm-check"
cd "$AISH_MANUAL_ROOT/rm-check"
printf data > protected.txt
chmod 444 protected.txt
rm protected.txt
```

Expected:

- The `rm` prompt is visible before the tester answers.
- Answering `n` preserves the file; answering `y` removes it.
- Aish does not consume keys meant for the foreground child.
- Aish prompt returns only after the child exits.

Full-screen examples:

```text
less README.md
vim
top
python3
node
tmux
ssh invalid.example
```

Skip tools that are unavailable. Use safe/disposable targets.

## Step 8: Editing, Completion, Paste, And Pickers

Run `MANUAL_TESTS.md` H-007 through H-010.

Editor:

```sh
export EDITOR=vim
```

Inside Aish, type a draft, press `Ctrl-X Ctrl-E`, save, return, inspect the
draft summary, then press `Enter` only if the command is safe. Repeat with
editor content beginning with `#`.

Clipboard:

- Paste one safe single-line command.
- Paste several safe lines.

Expected:

- Nothing executes before explicit confirmation or `Enter`.
- Multiline paste enters review/editor flow.

Fzf:

- `Ctrl-R` history picker.
- `Ctrl-X Ctrl-F` file picker.
- `Ctrl-X Ctrl-T` template picker.
- `Ctrl-X Ctrl-B` git branch picker.
- `Ctrl-X Ctrl-V` environment picker.

Confirm and cancel each picker where practical.

Completion visuals:

- Test light and dark themes.
- Test narrow terminal width.
- Test large font or high contrast.
- Test `#completion tab-accept full` and `#completion tab-accept word`.

## Step 9: Private Listing And Privacy Exports

Run `MANUAL_TESTS.md` H-011.

Create disposable items:

```text
echo history-manual-one
#mt echo template-manual {value}
```

Create a draft entry by typing `echo draft-manual-one`, pressing `Down` to save
the draft without executing it, then returning to a blank prompt. If a disposable
AI endpoint is not configured, `#ai list` and `#ai search` may be empty; still
verify that the commands are handled by Aish and do not go to the backend shell.

Run:

```text
#history list
#history search history-manual
#draft list
#draft search manual
#template list
#template search template-manual
#ai list
#ai search anything
```

Then test private export confirmation:

```text
#history list > history-export.txt
#history list | wc -l
```

Expected:

- List/search output is one line per item.
- Export/pipe asks for confirmation before writing or running.
- `n`, `Esc`, or `Ctrl-C` skips export.
- Accepted export writes only to the requested target or command.

## Step 10: GPG And Encryption

Run `MANUAL_TESTS.md` H-012 only with isolated state:

```sh
export GNUPGHOME="$AISH_MANUAL_ROOT/gnupg"
mkdir -p "$GNUPGHOME"
chmod 700 "$GNUPGHOME"
```

Use disposable passphrase-protected keys. Do not use personal keys.

Inside Aish:

```text
#encrypt on <fingerprint>
#encrypt unlock-mode prompt
exit
```

Relaunch with the same isolated `AISH_HOME`, then:

```text
#unlock
#key set
#encrypt rotate <second-fingerprint>
#encrypt off
gpg --version
```

Skip key rotation if only one disposable key exists.

Expected:

- Pinentry owns the terminal when it appears.
- Secrets and passphrases are not printed.
- Unlock failure is readable and recoverable.
- Encrypted managed storage uses `*.jsonl.gpg`.
- Plaintext managed JSONL files are absent while encryption is on after a
  successful migration/flush.
- `#encrypt off` restores plaintext managed storage and keeps the prompt usable.

## Step 11: AI And Context

Run `MANUAL_TESTS.md` H-013 only with disposable credentials.

Safe default path:

```text
#model test-model
#base-url http://127.0.0.1:9/v1
#env-key AISH_TEST_KEY
#context confirm on
# explain this < echo safe-context
```

Optional real endpoint path:

- Use a disposable OpenAI-compatible endpoint and key.
- Submit an AI prompt that returns command JSON.
- Browse candidates and execute only a safe selected command.

Expected:

- Provider errors are readable.
- Generated commands never auto-execute.
- Context pseudo-pipes ask for confirmation by default.
- Dangerous context commands still require confirmation even if normal
  confirmation is disabled.
- Secret-shaped data is redacted in logs/status.

## Step 12: Sync

Normal sync testing uses a local bare repository and does not require GitHub:

```sh
export AISH_SYNC_REMOTE="$AISH_MANUAL_ROOT/aish-sync-remote.git"
git init --bare "$AISH_SYNC_REMOTE"
```

Inside Aish:

```text
#set-remote <path-to-bare-repo>
#status
#push
```

Create disposable history/templates/notes before pushing.

Expected:

- Aish commits and pushes only managed enabled data.
- Repeated push with no changes does not create unnecessary commits.
- Conflict failures are readable and conservative.
- No scheduler files are created.

Only test a real private SSH/HTTPS remote when the tester explicitly has
non-production credentials. Missing GitHub authentication is not a product
failure.

## Step 13: Cross-Terminal, Home, And Recovery

Run `MANUAL_TESTS.md` H-015 through H-018 as platform time allows.

For production-shaped home, use a disposable `HOME`:

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
exit
```

Inspect:

```sh
find "$AISH_TEST_HOME" -maxdepth 3 -type f -o -type d
```

Expected:

- Aish creates `$HOME/.aish`.
- No unrelated files are touched.
- Relaunch with the same disposable `HOME` loads state.

Emergency recovery after terminal trouble:

1. Try `Ctrl-C`.
2. Try `Ctrl-D`.
3. Try `Ctrl-L`.
4. Kill the Aish or child process from another terminal.
5. Run:

```sh
stty sane
reset
```

Record which step worked.

## Failure Template

Use this for every failure:

````markdown
## Failure: <short title>

| Field | Value |
| --- | --- |
| Test ID |  |
| Severity | Critical / High / Medium / Low |
| Aish commit or source package |  |
| OS and terminal |  |
| Locale |  |
| Root user | Yes / No |
| Backend shell |  |
| AISH_HOME or HOME |  |
| Reproducible | Yes / No / Unknown |

### Steps

1.
2.
3.

### Expected


### Actual


### Recovery

Did the prompt remain usable? Did `Ctrl-C`, `Ctrl-D`, `Ctrl-L`, `exit`,
`#exit`, `stty sane`, or `reset` recover the terminal?

### Logs Or Output

```text

```
````

Severity guide:

| Severity | Meaning |
| --- | --- |
| Critical | Terminal wedge, unintended execution, command input corruption, user data loss, secret leakage, or stuck raw mode. |
| High | Startup, command execution, backend shell state, completion, editor/paste safety, passthrough, encryption, or sync safety fails. |
| Medium | Feature behaves incorrectly but has a clear workaround. |
| Low | Cosmetic rendering, wording, minor layout, or diagnostics issue. |

## Final Assessment

Use `Blocked` when startup, build, normal command execution, terminal recovery,
bash/zsh baseline, real passthrough, encryption safety, or sync safety fails.

Use `Acceptable with issues` when the platform mostly works but has optional
tool failures, fish-only failures, visual polish issues, missing credentials, or
non-critical feature differences.

Use `Acceptable` only when required automated tests pass, bash/zsh manual smoke
passes, critical passthrough works, and all skips are clearly justified.

Send maintainers:

- Final report.
- Commit hash or source package limitation.
- Full failure details.
- Screenshots for visual/rendering issues.
- Exact shell, terminal, OS, locale, and tool versions.
- Whether the problem reproduces in bash, zsh, fish, or all backends.
- Whether the problem reproduces with isolated `AISH_HOME`.
- The shortest known reproduction sequence.
