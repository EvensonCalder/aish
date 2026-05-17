# Aish Full Test Checklist

This file is the complete distributed test catalog for Aish. It is meant for human testers, AI-guided testers, and maintainers who need to decide whether a build is reliable on a specific platform.

Aish is a PTY-based command input layer, not a shell. The backend shell owns shell syntax and command execution. Tests must therefore verify two things separately:

- Aish behavior: prompt editing, completion, history, AI mode, templates, pickers, passthrough, storage, logging, and sync.
- Backend behavior: commands are passed to bash, zsh, or fish without Aish changing shell semantics.

## Scope

Required platform scope for distributed testing:

- macOS on at least Terminal.app or iTerm2/Ghostty/Alacritty.
- Linux on at least one Debian/Ubuntu-family system and one non-Debian family system when possible.
- Bash and zsh are required backend baselines.
- Fish is supported as opt-in validation until enough macOS and Linux version coverage proves it is stable.
- Native Windows is not a primary target for this PTY implementation. Windows testing should use WSL unless a future native Windows design is added.

Rules:

- Use English for all reports.
- Use an isolated `AISH_HOME` for normal testing.
- Do not test with a personal `~/.aish` until isolated testing passes.
- Record the exact commit hash and platform details.
- Mark each result as `PASS`, `FAIL`, `SKIP`, or `N/A`.
- Every reproducible failure should include enough detail to create an automated regression test.

## Report Metadata

Use this header in every report:

```markdown
# Aish Test Report

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
```

Severity guide:

| Severity | Meaning |
| --- | --- |
| Critical | Aish wedges the terminal, cannot exit, corrupts command input, loses user data, leaks secrets, or executes without explicit user action. |
| High | A core workflow fails: startup, command execution, completion, editing, picker recovery, passthrough, or sync safety. |
| Medium | A feature works incorrectly but has a clear workaround. |
| Low | Visual polish, wording, minor layout, or diagnostics issue. |

## Automated Test Commands

Run these from the repository root.

| ID | Command | Expected Result |
| --- | --- | --- |
| AUTO-001 | `cargo fmt --check` | Formatting check passes. |
| AUTO-002 | `cargo build` | Debug binary builds successfully at `target/debug/aish`. |
| AUTO-003 | `cargo test --lib` | Library unit tests pass. |
| AUTO-004 | `cargo test --test draft_execution -- --nocapture` | Draft execution integration tests pass. |
| AUTO-005 | `cargo test --test first_run -- --nocapture` | First-run integration tests pass. |
| AUTO-006 | `cargo test --test pty_backend -- --nocapture` | Bash and zsh PTY tests pass; unavailable optional shells skip cleanly. |
| AUTO-007 | `AISH_TEST_FISH=1 cargo test --test pty_backend -- --nocapture` | Fish PTY tests pass when fish is installed, or the failure is recorded as a fish compatibility bug. |
| AUTO-008 | `cargo test --test expect_runner -- --test-threads=1 --nocapture` | Expect-driven terminal scenarios pass serially. |
| AUTO-009 | `cargo test --test tmux_capture -- --test-threads=1 --nocapture` | Tmux screen-capture tests pass serially. |
| AUTO-010 | `AISH_TEST_FISH=1 cargo test --test tmux_capture tmux_common_shell_workflow_matches_fish_backend_real_terminal_screen -- --nocapture` | Fish tmux smoke passes when fish is installed, or the failure is recorded as a fish compatibility issue. |
| AUTO-011 | `cargo clippy --all-targets -- -D warnings` | Clippy passes without warnings. |
| AUTO-012 | `git diff --check` | No whitespace errors. |
| AUTO-013 | `cargo test -- --list` | Test discovery completes and the reported inventory can be compared with `TESTS.md`. |

Recommended full verification for feature changes:

```sh
cargo fmt --check
cargo build
cargo test --lib
cargo test --test draft_execution -- --nocapture
cargo test --test first_run -- --nocapture
cargo test --test pty_backend -- --nocapture
cargo test --test expect_runner -- --test-threads=1 --nocapture
cargo test --test tmux_capture -- --test-threads=1 --nocapture
cargo clippy --all-targets -- -D warnings
git diff --check
```

Optional fish verification:

```sh
AISH_TEST_FISH=1 cargo test --test pty_backend -- --nocapture
AISH_TEST_FISH=1 cargo test --test tmux_capture tmux_common_shell_workflow_matches_fish_backend_real_terminal_screen -- --nocapture
```

## Isolated Manual Launch

Use this for almost every manual test:

```sh
cargo build
export AISH_MANUAL_ROOT="/tmp/aish-manual-$(date +%s)"
mkdir -p "$AISH_MANUAL_ROOT"
export AISH_HOME="$AISH_MANUAL_ROOT/home"
./target/debug/aish
```

For a specific backend shell, launch with `SHELL` set:

```sh
SHELL=/bin/bash AISH_HOME="$AISH_MANUAL_ROOT/bash-home" ./target/debug/aish
SHELL=/bin/zsh AISH_HOME="$AISH_MANUAL_ROOT/zsh-home" ./target/debug/aish
SHELL="$(command -v fish)" AISH_HOME="$AISH_MANUAL_ROOT/fish-home" ./target/debug/aish
```

Only run the fish command when `command -v fish` succeeds.

## Basic Platform Tests

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| PLAT-001 | Record `uname -a`. | OS, kernel, and architecture are captured. | Paste output. |
| PLAT-002 | On macOS, record `sw_vers`. On Linux, record `/etc/os-release` or `lsb_release -a`. | Distribution and release are captured. | Paste output. |
| PLAT-003 | Record `$SHELL` and the backend shell versions for bash, zsh, and fish if installed. | Shell paths and versions are known. | Paste output. |
| PLAT-004 | Record terminal emulator name and version. | The report names the real terminal that rendered Aish. | Paste version or describe UI source. |
| PLAT-005 | Record `rustc --version`, `cargo --version`, `git --version`, `tmux -V`, `fzf --version`, `python3 --version`, and `gpg --version` when available. | Tool availability is clear. Missing optional tools are documented. | Paste output or missing-tool notes. |

## Startup And Storage

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| START-001 | Launch Aish with a new isolated `AISH_HOME`. | A prompt appears and Aish creates its directory layout. | Prompt screenshot or text plus `find "$AISH_HOME" -maxdepth 2 -type d`. |
| START-002 | Run `#doctor`. | The command reports active paths, shell backend, and status without corrupting the prompt. | Paste output. |
| START-003 | Run `#status`. | Runtime status is shown and secrets are not printed. | Paste output. |
| START-004 | Run `#config`. | Config is readable and matches expected defaults. | Paste output. |
| START-005 | Exit with `#exit`, relaunch with the same `AISH_HOME`, and run `#status`. | Aish restarts cleanly and keeps persisted config/history files. | Paste output. |
| START-006 | Launch with `AISH_HOME` set to a relative path. | Startup fails with a readable error. | Paste error. |
| START-007 | Launch with `AISH_HOME` pointing at an existing regular file. | Startup fails with a readable error and does not overwrite the file. | Paste error and file listing. |
| START-008 | Use a disposable `HOME` with `AISH_HOME` unset. | Aish creates `$HOME/.aish` and does not touch unrelated files. | Paste `find "$HOME" -maxdepth 2`. |
| START-009 | Create a saved draft, exit, relaunch with the same `AISH_HOME`, then press `Up`. | Startup prompt is blank; `Up` restores the newest saved draft explicitly. | Paste prompt behavior. |

## Ordinary Shell Commands

These tests verify that Aish sends ordinary commands to the backend shell without becoming a shell itself.

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| CMD-001 | Run `pwd`. | Output is the current working directory. | Paste output. |
| CMD-002 | Run `cd /tmp`, then `pwd`. | The directory change persists in the backend shell. | Paste output. |
| CMD-003 | Run `printf 'alpha\nbeta\ngamma\n'`. | Three lines are printed exactly once. | Paste output. |
| CMD-004 | Run `echo stdout-ok; echo stderr-ok >&2`. | Both stdout and stderr remain visible before the next prompt. | Paste output. |
| CMD-005 | Run `false`, then check status with backend syntax. For bash/zsh: `echo status:$?`. For fish: `echo status:$status`. | The backend shell reports the failed exit status. | Paste output. |
| CMD-006 | Run `mkdir -p "$AISH_MANUAL_ROOT/work"`, `touch "$AISH_MANUAL_ROOT/work/a file.txt"`, then `ls "$AISH_MANUAL_ROOT/work"`. | File names with spaces survive shell quoting. | Paste output. |
| CMD-007 | Run `cat "$AISH_MANUAL_ROOT/work/a file.txt"`. | Empty file prints no unexpected text and prompt returns. | Paste output or describe. |
| CMD-008 | Run `printf 'red\nblue\nred\n' | grep red`. | Pipeline behavior is owned by the backend shell and works normally. | Paste output. |
| CMD-009 | Run `printf '3\n1\n2\n' | sort`. | Sorted output appears and prompt returns. | Paste output. |
| CMD-010 | Run `printf 'one two\n' | wc -w`. | Output reports two words. | Paste output. |
| CMD-011 | Run `echo unicode-OK-cafe-resume-check-✓`. | Unicode input and output render without corruption. | Paste output or screenshot. |
| CMD-012 | Run a command that produces many lines, such as `seq 1 80` where available. | Output scrolls normally and the prompt returns usable. | Paste last visible lines. |
| CMD-013 | Run `sleep 5`, press `Ctrl-C`. | The command is interrupted and Aish returns to a usable prompt. | Describe result. |
| CMD-014 | Run `clear`. | Screen clears and the prompt redraws at the top without an extra blank line. | Screenshot or describe. |
| CMD-015 | Press `Ctrl-L`. | Screen clears and prompt remains usable. | Screenshot or describe. |

## Backend Shell Independence

Run this section once with bash and once with zsh. Run fish as opt-in compatibility validation.

| ID | Backend | What To Do | Expected Behavior |
| --- | --- | --- | --- |
| SH-001 | bash | Launch with `SHELL=/bin/bash`, then run `echo bash-ok`. | Command executes and prompt returns. |
| SH-002 | bash | Run `export AISH_BACKEND_TEST=bash-ok`, then `echo "$AISH_BACKEND_TEST"`. | Environment state persists in bash. |
| SH-003 | bash | Run `aish_test_func(){ echo bash-function-ok; }`, then `aish_test_func`. | Bash function persists. |
| SH-004 | zsh | Launch with `SHELL=/bin/zsh`, then run `echo zsh-ok`. | Command executes and prompt returns. |
| SH-005 | zsh | Run `export AISH_BACKEND_TEST=zsh-ok`, then `echo "$AISH_BACKEND_TEST"`. | Environment state persists in zsh. |
| SH-006 | zsh | Run `aish_test_func(){ echo zsh-function-ok; }`, then `aish_test_func`. | Zsh function persists. |
| SH-007 | fish | Launch with `SHELL="$(command -v fish)"`, then run `echo fish-ok`. | Command executes and prompt returns, or a compatibility issue is recorded. |
| SH-008 | fish | Run `set -gx AISH_BACKEND_TEST fish-ok`, then `echo $AISH_BACKEND_TEST`. | Environment state persists in fish, or a compatibility issue is recorded. |
| SH-009 | fish | Run `function aish_test_func; echo fish-function-ok; end`, then `aish_test_func`. | Fish function persists, or a compatibility issue is recorded. |
| SH-010 | all tested shells | Run `cd /tmp`, `pwd`, and a simple command after prompt redraw. | Aish remains backend independent for persistent shell state and screen rendering. |

## Prompt Editing And Keybindings

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| EDIT-001 | Type `echo ac`, move left, insert `b`, and press `Enter`. | Command becomes `echo abc` and prints `abc`. | Paste output. |
| EDIT-002 | Type `echo start end`, use `Ctrl-A` and insert `printf `. | Cursor movement works and the final command reflects the edit. | Paste output. |
| EDIT-003 | Type `echo remove-me keep`, use `Ctrl-W` once, and execute. | Previous word is removed according to readline-style behavior. | Paste output. |
| EDIT-004 | Type text, use `Ctrl-U`. | Text before cursor is cleared and prompt remains usable. | Describe result. |
| EDIT-005 | Type text, use `Ctrl-K`. | Text after cursor is cleared and prompt remains usable. | Describe result. |
| EDIT-006 | Use `Alt-B` and `Alt-F` or `Alt-Left` and `Alt-Right` across words. | Word movement is predictable and does not corrupt the line. | Describe result. |
| EDIT-007 | Type `echo drop keep`, move to `drop`, press `Alt-Delete`, and execute. | The next word is removed and the command prints `keep`. | Paste output. |
| EDIT-008 | Type a draft and press `Esc`. | Draft clears and Aish returns to draft mode. | Describe result. |
| EDIT-009 | Press `Ctrl-D` on an empty draft. | Aish exits cleanly and terminal state is restored. | Shell prompt returns. |
| EDIT-010 | Press `Ctrl-X` followed by an unsupported key. | Chord cancels without corrupting the draft or terminal. | Describe result. |
| EDIT-011 | Type `echo first-draft`, press `Down`, type `echo second-draft`, press `Down`, then use `Up`, `Up`, `Down`, `Down`. Type and execute `echo after-draft`. | `Up` / `Down` browse second, first, second, then a blank draft. The final command does not append to stale draft text, and both saved drafts appear in draft JSONL. | Paste output and draft JSONL evidence. |

## Modes And History

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| MODE-001 | Press `Tab` on an empty draft repeatedly. | Prompt cycles through draft, history, and AI modes. | Describe prompt symbols. |
| MODE-002 | Run `echo history-one` and `echo history-two`, then use history mode. | Previous commands are browsable read-only. | Describe selected item. |
| MODE-003 | In history mode, edit a selected command. | The item is copied to draft mode before editing. | Paste final output. |
| MODE-004 | Execute a selected history item with `Enter`. | The command runs and is appended as a new history entry. | Paste output. |
| MODE-005 | Exit and relaunch with the same `AISH_HOME`, then browse history. | History persists across restarts. | Describe result. |
| MODE-006 | Type a draft, exit or relaunch according to draft persistence behavior, then inspect restored draft behavior. | Draft persistence matches config and does not corrupt prompt. | Describe result. |

## Completion

Use a fresh isolated `AISH_HOME` when possible so history-based suggestions are predictable.

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| COMP-001 | Run `echo completion-alpha-value`, then type `echo completion-alpha` and pause. | Inline ghost text appears while typing when `completion.inline=true`. | Screenshot or describe. |
| COMP-002 | With inline enabled, press `Tab` on the visible suggestion. | `Tab` accepts according to `completion.tab_accept`. | Paste final command/output. |
| COMP-003 | Run `#completion tab-accept full`, then repeat a long history suggestion. | `Tab` accepts the full visible inline suggestion. | Paste output. |
| COMP-004 | Run `#completion tab-accept word`, then complete `echo word-alpha word-beta word-gamma` from `echo word`. | Each `Tab` accepts only through the next whitespace boundary. | Paste output. |
| COMP-005 | Run `#completion inline off`, type a prefix with candidates, then press `Tab`. | With inline disabled, `Tab` accepts the first ranked candidate directly. | Paste output. |
| COMP-006 | Run `#completion inline on`. | Live inline suggestions return. | Describe result. |
| COMP-007 | Run `#completion max 2`, create at least four matching history commands, then type the shared prefix. | Below-prompt hint rows are limited to two; inline suggestion is not counted as a row. | Screenshot or describe. |
| COMP-008 | Type a prefix that has no matches and press `Tab`. | No completion rows or inline ghost appear, the draft stays unchanged, and the prompt remains usable. | Describe result. |
| COMP-009 | Create files with spaces, then use path completion. | Suggested and accepted paths are shell-safe for the backend. | Paste command/output. |
| COMP-010 | In a narrow terminal, type a long prefix with long suggestions. | Below-prompt rows remain one line and elide with `...` instead of wrapping. | Screenshot. |
| COMP-011 | Move cursor to the middle of a draft and trigger completion. | Completion does not corrupt text before or after the cursor. | Paste final command/output. |
| COMP-012 | Verify bash and zsh completion behavior using the same history. | User-visible completion is Aish-owned and backend independent. | Record backend results. |
| COMP-013 | Run `#completion match-threshold 80`, type a weak partial match, then run `#completion match-threshold 30` and repeat. | The stricter threshold hides weak candidates; the lower threshold allows them. | Paste output or screenshots. |
| COMP-014 | If fish is tested, repeat a basic completion workflow. | Fish matches bash/zsh behavior or the difference is recorded. | Record result. |

## Continuation Input

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| CONT-001 | Type `echo "unterminated` and press `Enter`. | Aish enters continuation instead of showing backend secondary prompt noise. | Describe prompt. |
| CONT-002 | Finish the quote and press `Enter`. | Full command executes correctly. | Paste output. |
| CONT-003 | Start single-quote continuation and finish it. | Single-quote continuation works. | Paste output. |
| CONT-004 | Type a command ending in a trailing backslash and continue on the next line. | Backslash continuation works. | Paste output. |
| CONT-005 | Start a continuation and press `Ctrl-C`. | Continuation cancels and prompt returns usable. | Describe result. |

## Private Commands, Notes, And Diagnostics

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| PRIV-001 | Run `#help`. | Help lists available commands and keybindings accurately. | Paste output. |
| PRIV-002 | Run an unknown private command such as `#does-not-exist`. | Aish reports an unknown command and does not send it to the backend shell. | Paste output. |
| PRIV-003 | Run `#history 5`. | History limit updates or reports a readable error according to command syntax. | Paste output. |
| PRIV-004 | Type `# TODO: manual test note`. | The note is stored as a note and not executed by the backend shell. | Confirm no shell error. |
| PRIV-005 | Run `#log 20`. | Recent events are shown and sensitive-looking values are redacted. | Paste output. |
| PRIV-006 | Run `#model test-model`, `#base-url http://127.0.0.1:9/v1`, and `#env-key AISH_TEST_KEY`. | AI config persists and secrets are not printed. | Paste `#config` output. |
| PRIV-007 | Run `#key set` before configuring an encryption key fingerprint. | Aish reports that the encryption key is not configured and does not print or store any secret. | Paste output. |
| PRIV-008 | Run `#key clear`. | Existing key file is removed if present; missing key is handled safely. | Paste output. |
| PRIV-009 | Run `#encrypt on` without a configured key, then run `#encrypt off` in an isolated home. | Missing-key usage is reported safely; disabling encryption either reports plaintext mode or leaves the session usable without secret output. | Paste output. |

## Templates

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| TPL-001 | Run `#mt echo hello {name}`. | Template is created and Aish prints a stable `tpl-...` ID. | Paste output and record the ID. |
| TPL-002 | Run `#template find hello`. | Matching templates are shown with their `tpl-...` IDs. | Paste output. |
| TPL-003 | Run `#template list`. | Aish reports that listing is intentionally unsupported and points to `#template find <query>` or the JSONL store. | Paste output. |
| TPL-004 | Run `#template show <id>` using the ID from TPL-001. | Template content is shown. | Paste output. |
| TPL-005 | Run `#template use <id> name=world`. | The resolved command executes only after expected confirmation or submission path and prints `hello world`. | Paste output. |
| TPL-006 | Run `#template use <id>` without `name`. | Unresolved placeholder blocks execution. | Paste output. |
| TPL-007 | Run `#template replace <id> echo hi {name}`, then use the newly printed ID. | Replaced template is used and the replacement has a new body-derived ID. | Paste output. |
| TPL-008 | Run `#template rm <id>`. | Template is removed and no stale entry remains. | Paste output. |
| TPL-009 | Exit and relaunch with the same `AISH_HOME`, then inspect templates with `#template find <query>` or the JSONL store. | Template persistence matches the previous operations. | Paste output. |
| TPL-010 | Run `#mt echo {something}`, type `echo something`, then accept completion. | Aish completes to `echo {something}` even though braces were not typed, and unresolved placeholder execution is blocked until edited. | Paste output. |
| TPL-011 | Store `#mt echo {a} {older}`, then store `#mt echo {a} {b} {c}`. Type `echo {a} {something}` and accept completion. With the default `completion.tab_accept="word"`, press `Tab` again until the full template is accepted, or set `#completion tab-accept full` first. | Newer structural template completion wins and completes to `echo {a} {b} {c}`; generic placeholder/history fallbacks do not override it. | Paste output. |

## External Editor And Paste Review

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| EDITOR-001 | Set `EDITOR` or `VISUAL` to a real editor, type a draft, then press `Ctrl-X Ctrl-E`. Save and exit. | Aish restores the terminal and shows an editor draft summary. It does not execute until `Enter`. | Describe result. |
| EDITOR-002 | Press `Enter` after editor return. | The editor content executes as raw shell input. | Paste output. |
| EDITOR-003 | Make the editor exit with failure or kill it from another terminal. | Aish reports failure, preserves the original draft where possible, and prompt remains usable. | Describe result. |
| EDITOR-004 | Paste one single-line command from the OS clipboard. | It inserts or reviews according to configured paste policy and never executes before explicit `Enter`. | Describe result. |
| EDITOR-005 | Paste several command lines from the OS clipboard. | Multiline paste enters review/editor flow and never silently executes. | Describe result. |
| EDITOR-006 | Use editor content beginning with `#`, such as `# not a private command from editor`. | Editor-submitted content bypasses private-command parsing when executed as raw shell input. | Paste output. |

## Fzf Pickers

These require real `fzf`. If `fzf` is unavailable, mark as `SKIP`.

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| FZF-001 | Press `Ctrl-R` after creating history entries. Select an entry. | Selected history replaces or inserts as intended and can execute. | Describe result. |
| FZF-002 | Press `Ctrl-R` and cancel. | Draft is preserved and prompt remains usable. | Describe result. |
| FZF-003 | Press `Ctrl-X Ctrl-F` with files including spaces in the working tree. Select a file. | Inserted path is shell-safe. | Paste command/output. |
| FZF-004 | Press `Ctrl-X Ctrl-F` and cancel. | Draft is preserved. | Describe result. |
| FZF-005 | Press `Ctrl-X Ctrl-T` with templates available. Select and cancel in separate runs. | Selection inserts/uses the template correctly; cancellation preserves draft. | Describe result. |
| FZF-006 | In a git repository, press `Ctrl-X Ctrl-B`. Select and cancel in separate runs. | Branch picker works without corrupting draft. | Describe result. |
| FZF-007 | Set a backend environment variable, press `Ctrl-X Ctrl-V`, and select it. | Picker uses backend environment and inserts the expected variable safely. | Describe result. |
| FZF-008 | Cancel the environment picker. | Draft is preserved. | Describe result. |

## AI And Context

Network AI tests should use a disposable endpoint and disposable API key only.

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| AI-001 | Configure model, base URL, and env key with `#model`, `#base-url`, and `#env-key`. | Config persists and key values are not printed. | Paste `#config` output. |
| AI-002 | With no valid endpoint, submit a simple AI prompt. | Aish reports a readable provider/network error and prompt recovers. | Paste output. |
| AI-003 | With a disposable OpenAI-compatible endpoint, submit an AI prompt that returns command JSON. | Aish shows browsable AI command candidates and does not auto-execute them. | Paste sanitized output. |
| AI-004 | Browse AI mode and execute one command. | Only the selected command executes. | Paste output. |
| AI-005 | Run a context pseudo-pipe such as `# explain this < echo safe-context`. | Aish asks for confirmation by default and redacts secret-shaped data. | Paste output. |
| AI-006 | Run `#context confirm off`, then repeat a safe context command. | Safe context command runs without confirmation. | Paste output. |
| AI-007 | Try a dangerous context command pattern. | Aish still blocks or asks for confirmation even when confirmation is disabled. | Paste output. |
| AI-008 | Produce context larger than the configured byte limit. | Aish discloses truncation. | Paste output. |
| AI-009 | Type `# ` and press `Enter` with `EDITOR` configured. Save a multi-line prompt. | Aish returns an opaque AI prompt summary; pressing `Enter` sends it through the AI path, not to the backend shell. | Paste output or error. |
| AI-010 | Type `# explain something`, press `Ctrl-X Ctrl-E`, save, then press `Ctrl-X Ctrl-E` again. | The editor opens with the current AI prompt body each time and preserves edits. | Describe editor contents. |
| AI-011 | Ask a generic AI prompt such as `# how to echo something?`. | Generated command text uses a placeholder such as `{message}` instead of treating `something` as a literal argument. | Paste generated item. |

## Interactive And Stdin Passthrough

These tests are important because Aish must not wedge when a command expects terminal or stdin control.

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| PASS-001 | Run `python3`, type `print("python-ok")`, press `Enter`, then exit with `exit()`. | Python REPL owns input while running, prints output, exits, and Aish prompt returns. | Paste or describe. |
| PASS-002 | Run `cat`, type `cat-ok`, press `Enter`, then press `Ctrl-D`. | `cat` receives stdin and exits on EOF; Aish prompt returns. | Paste output. |
| PASS-003 | Run `grep needle`, type `needle`, press `Enter`, type `other`, press `Enter`, then press `Ctrl-D`. | `grep` receives stdin, prints matching line, exits on EOF, and prompt returns. | Paste output. |
| PASS-004 | Run `sed 's/a/A/'`, type `abc`, press `Enter`, then press `Ctrl-D`. | `sed` receives stdin and prompt returns after EOF. | Paste output. |
| PASS-005 | Run `awk '{print $1}'`, type `one two`, press `Enter`, then press `Ctrl-D`. | `awk` receives stdin and prompt returns after EOF. | Paste output. |
| PASS-006 | Run `less README.md` if `less` is available; quit with `q`. | Pager owns terminal, exits, and prompt redraws cleanly. | Describe result. |
| PASS-007 | Run `gpg` with no arguments, then interrupt or exit according to the displayed GPG behavior. | Aish does not wedge; `Ctrl-C` or EOF recovers the prompt. | Describe result. |
| PASS-008 | Run a real editor such as `vim` or `nvim`, then quit without saving. | Editor owns alternate screen and Aish prompt returns after exit. | Describe result. |
| PASS-009 | Run `top` or an equivalent process viewer, then quit. | Full-screen interactive program gets keys and Aish recovers. | Describe result. |
| PASS-010 | Run `node` or another REPL if installed, then exit normally. | REPL owns input and Aish prompt returns. | Describe result. |
| PASS-011 | Run nested `tmux` or `screen` if available, then detach/exit. | Nested terminal program does not permanently corrupt Aish. | Describe result. |
| PASS-012 | Run an SSH command to a disposable or intentionally invalid host. | Auth prompts or errors are foregrounded; prompt recovers. | Describe result. |

## Sync

Use only disposable repositories.

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| SYNC-001 | Configure a local bare git remote in `/tmp`. | Remote setup does not use network or personal credentials. | Paste commands. |
| SYNC-002 | Run `#set-remote <local-bare-repo-url>`. | Config persists the remote. | Paste `#config` output. |
| SYNC-003 | Create history/templates/notes, then run `#push`. | Aish commits only managed enabled paths and pushes successfully. | Paste output. |
| SYNC-004 | Run `#push` again with no changes. | Aish reports no changes or succeeds without creating unnecessary commits. | Paste output. |
| SYNC-005 | Create a deterministic conflict in the disposable remote, then run `#push`. | Aish reports the conflict/failure and does not auto-resolve, delete, or rewrite history. | Paste output. |
| SYNC-006 | Run `#sync off`, `#sync ai on`, `#sync history on`, `#sync templates on`, and `#sync drafts on`. | Category config persists; no scheduler files are created. | Paste output. |
| SYNC-007 | Configure a real private remote only if explicitly safe and disposable. | Auth prompts do not wedge terminal; Aish remains conservative. | Describe result. |

## Encryption And GPG

Use an isolated `AISH_HOME` and an isolated `GNUPGHOME` with disposable keys for any test that enables encryption. Never use a personal key or personal `~/.aish` for destructive encryption tests.

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| ENC-001 | Before configuring encryption, run `#key set`. | Aish reports that an encryption key is not configured and does not print or persist a secret. | Paste output. |
| ENC-002 | Run `#key clear`. | Removes an existing key file if present, or safely reports none. | Paste output. |
| ENC-003 | Configure a disposable GPG key and run `#encrypt on <fingerprint>`. | Managed history, notes, drafts, AI history, and templates migrate to `*.jsonl.gpg`; plaintext JSONL files are removed after successful encryption; the Git history warning names the explicit rewrite flow. | Paste output and file listing. |
| ENC-004 | While encryption is enabled, run commands/templates/notes, then inspect storage. | New writes are encrypted through the background writer and no plaintext managed JSONL files are left behind after flush/exit. | Paste output and file listing. |
| ENC-005 | Run real `gpg` or `gpg --version` from Aish. | GPG-related passthrough does not wedge Aish. | Paste output or describe. |
| ENC-006 | With `#env-key AISH_TEST_KEY` set and the environment variable present before launch, run `#key set`, then relaunch without the variable and submit an AI request against a disposable endpoint if available. | The API key is stored encrypted, never printed, and can be used only after GPG decrypt succeeds. | Paste sanitized output. |
| ENC-007 | Run `#encrypt rotate <second-fingerprint>` if a second disposable key exists. | Existing encrypted managed storage is decrypted and re-encrypted for the new fingerprint. | Paste output and file listing. |
| ENC-008 | Run `#encrypt off`. | Pending encrypted writes flush, managed storage decrypts back to plaintext JSONL, and future writes use plaintext files. | Paste output and file listing. |
| ENC-009 | Run `#encrypt rewrite-history plan`. | Aish prints the destructive risk, target key, scope, and the explicit confirmed run command without rewriting history. | Paste output. |

## Visual, Terminal, And Accessibility Checks

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| TERM-001 | Use at least one light theme and one dark theme. | Inline completion remains dim but readable and not confused with typed text. | Screenshots or notes. |
| TERM-002 | Resize to a narrow width and type a long command. | Prompt redraw does not duplicate prompt fragments or overlap UI. | Screenshot. |
| TERM-003 | Resize while a draft is visible. | Draft redraw remains coherent. | Screenshot or notes. |
| TERM-004 | Trigger completion near the right edge. | Inline and below-prompt hints stay within terminal width and elide with `...` when needed. | Screenshot. |
| TERM-005 | Use large font or high contrast settings. | Important text remains visible and does not rely only on subtle color. | Notes. |
| TERM-006 | Test Terminal.app, iTerm2, Ghostty, Alacritty, GNOME Terminal, or Konsole where available. | Core prompt, command, completion, paste, and passthrough behavior remain stable. | Per-terminal result table. |
| TERM-007 | Close the terminal window while Aish is running, then open a normal shell. | Terminal is recoverable; if needed, `stty sane` restores it. | Notes. |
| TERM-008 | Kill an interactive child process from another terminal. | Aish reports or recovers cleanly and prompt remains usable. | Notes. |

## Production-Shaped Home Checks

Only run after isolated tests pass.

| ID | What To Do | Expected Behavior | Evidence |
| --- | --- | --- | --- |
| HOME-001 | Use a disposable but normal-looking `HOME` with `AISH_HOME` unset. | Aish creates `$HOME/.aish` and persists config/history/templates/logs. | Paste file listing. |
| HOME-002 | Relaunch with the same disposable `HOME`. | Stored state loads correctly. | Describe result. |
| HOME-003 | If inspecting a real personal `~/.aish`, back it up first and do not run destructive sync or encryption commands. | No unrelated personal files are changed. | Notes only. |

## Human-Only Checks

These checks cannot be fully replaced by automation because they depend on human visual judgment, OS clipboard behavior, real terminal emulators, real network auth, real GPG/pinentry, or platform-specific interactive programs.

| ID | Source | What Remains Human |
| --- | --- | --- |
| HUMAN-001 | `MANUAL_TESTS.md` H-001 | Inline completion contrast and readability across themes. |
| HUMAN-002 | `MANUAL_TESTS.md` H-002 | Narrow-terminal visual polish across fonts and terminal emulators. |
| HUMAN-003 | `MANUAL_TESTS.md` H-003 | Whether full vs word `Tab` acceptance feels intuitive. |
| HUMAN-004 | `MANUAL_TESTS.md` H-004 | Real OS clipboard and bracketed paste behavior. |
| HUMAN-005 | `MANUAL_TESTS.md` H-005/H-006 | Real editor success and failure behavior. |
| HUMAN-006 | `MANUAL_TESTS.md` H-007 | Real `fzf` layout and key handling. |
| HUMAN-007 | `MANUAL_TESTS.md` H-008 | Broad full-screen program passthrough across platforms. |
| HUMAN-008 | `MANUAL_TESTS.md` H-009 | Real AI endpoint, network, auth, and rate-limit behavior. |
| HUMAN-009 | `MANUAL_TESTS.md` H-010 | Real GPG encryption and pinentry. |
| HUMAN-010 | `MANUAL_TESTS.md` H-011 | Fish behavior across macOS and Linux fish versions. |
| HUMAN-011 | `MANUAL_TESTS.md` H-012 | Cross-terminal smoke tests. |
| HUMAN-012 | `MANUAL_TESTS.md` H-013 | Production-shaped home behavior. |
| HUMAN-013 | `MANUAL_TESTS.md` H-014/H-015 | Real private git remote auth and conflict recovery. |
| HUMAN-014 | `MANUAL_TESTS.md` H-016 | Accessibility perception. |
| HUMAN-015 | `MANUAL_TESTS.md` H-017 | Abnormal terminal and child-process interruption. |

## Failure Record Template

Use this for every failure:

````markdown
## Failure: <short title>

| Field | Value |
| --- | --- |
| Test ID |  |
| Severity | Critical / High / Medium / Low |
| Aish commit |  |
| OS and terminal |  |
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

Did the prompt remain usable? Did `Ctrl-C`, `Ctrl-D`, `Ctrl-L`, `#exit`, or `stty sane` recover the terminal?

### Logs Or Output

```text

```
````

## Completion Criteria For A Distributed Test Pass

A platform test pass is acceptable when:

- Required automated tests pass or skipped tools are clearly documented.
- Bash and zsh backend manual smoke tests pass.
- Fish is either tested successfully or explicitly recorded as not tested or failing with version details.
- Basic commands, editing, completion, paste safety, pickers, templates, passthrough, sync safety, and storage tests are covered.
- Human-only visual or environment-specific checks have explicit notes.
- Every failure has severity, reproduction steps, backend shell, terminal, and recovery status.
