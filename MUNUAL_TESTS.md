# Aish Manual Test List

This file is the human-run checklist for validating Aish behavior in a real terminal. It intentionally complements automated Rust, expect, and tmux tests by describing what to do and what should be visible to a user.

Use an isolated Aish home for every manual run unless the test explicitly says otherwise.

```sh
cargo build
export AISH_MANUAL_ROOT="/tmp/aish-manual-$(date +%s)"
mkdir -p "$AISH_MANUAL_ROOT"
export AISH_HOME="$AISH_MANUAL_ROOT/home"
./target/debug/aish
```

For backend-specific checks, create `$AISH_HOME/config.toml` before launching Aish:

```toml
[shell]
backend = "/bin/bash"
```

or:

```toml
[shell]
backend = "/bin/zsh"
```

Fish is experimental and should be run only as an opt-in compatibility check until it has been validated across macOS and representative Linux distributions.

## General Rules

- Run tests in a real terminal, not inside an output-only log viewer.
- Prefer `/tmp` work directories and disposable files.
- Do not use a real API key, real remote repository, or personal `~/.aish` unless a test explicitly asks for production-home behavior.
- After each test, Aish should remain usable unless the expected behavior is exit.
- If a test starts from a clean state, exit Aish and relaunch with a new `AISH_HOME`.

## Quick Smoke

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-001 | Launch `./target/debug/aish` with a new absolute `AISH_HOME`. | Aish starts without errors, creates the Aish home, and shows a draft prompt ending in `> `. |
| M-002 | Type `echo hello` and press `Enter`. | The terminal shows `hello`, then redraws a usable `> ` prompt. |
| M-003 | Type `pwd` and press `Enter`. | The backend shell prints the current directory, and Aish returns to draft mode. |
| M-004 | Type `exit` and press `Enter`. | Aish exits cleanly and leaves the terminal usable. |
| M-005 | Relaunch Aish and press `Ctrl-D` on an empty draft. | Aish exits cleanly without printing an error. |

## First Run, Config, And Home Handling

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-010 | Launch with a missing absolute `AISH_HOME`. | Aish creates `config.toml`, `history/`, `templates/`, `logs/`, `cache/`, and `secrets/` under that home. |
| M-011 | Run `#doctor`. | Diagnostics print shell, PTY, editor/fzf, AI, and storage information; no command is sent to the backend shell. |
| M-012 | Run `#config`. | Config output includes paths, shell backend, editor config, paste config, completion config, AI config, context config, sync config, and storage paths. |
| M-013 | Run `#status`. | Status output includes mode, last status, cwd, shell, AI key source, encryption/sync state, context config, completion config, and keybinding count. |
| M-014 | Exit and inspect `$AISH_HOME/config.toml`. | The file exists and contains readable TOML with default config sections. |
| M-015 | Launch with `AISH_HOME=relative-path`. | Startup fails with a readable message saying `AISH_HOME` must be absolute; the terminal remains usable. |
| M-016 | Launch without `AISH_HOME` but with a disposable `HOME`. | Aish creates `$HOME/.aish`, starts successfully, and persists config/history there. |
| M-017 | Put malformed TOML in `$AISH_HOME/config.toml`, then launch. | Startup fails with a readable `invalid config` message that includes the config path. |
| M-018 | Set `$AISH_HOME` to a path that is an existing file, then launch. | Startup fails cleanly with a readable directory/create error. |

## Backend Shell State And Portability

Run these checks with `/bin/bash` and `/bin/zsh` as configured backends. Fish may be tested separately as experimental.

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-030 | Configure backend `/bin/bash`, launch, run `echo $0` or `printf 'backend:%s\n' "$BASH_VERSION"`. | Bash-specific output is visible, and Aish prompt returns. |
| M-031 | Configure backend `/bin/zsh`, launch, run `printf 'backend:%s\n' "$ZSH_VERSION"`. | Zsh-specific output is visible, and Aish prompt returns. |
| M-032 | Run `mkdir -p /tmp/aish-manual-work && cd /tmp/aish-manual-work`, then run `pwd`. | `pwd` prints `/tmp/aish-manual-work`; cwd persists across commands. |
| M-033 | Run `export AISH_MANUAL_VALUE=visible`, then `printenv AISH_MANUAL_VALUE`. | The second command prints `visible`; shell environment state persists. |
| M-034 | Run `false`, then `#status`. | `#status` reports `last_status=1` or the backend failure status; Aish remains usable. |
| M-035 | Run `printf 'alpha\nbeta\n' > input.txt`, then `cat input.txt | grep beta`. | Output shows `beta`; pipes and files are owned by the backend shell. |
| M-036 | Run `printf 'quoted:%s\n' 'value with spaces'`. | Output is exactly `quoted:value with spaces`. |
| M-037 | Press `Ctrl-C` at an empty prompt. | Draft stays empty, mode returns to draft, and the prompt remains usable. |
| M-038 | Run `history | tail` in bash or zsh after several commands. | User commands appear; Aish internal marker commands should not appear where shell support allows suppression. |

## Prompt, Editing, And Keybindings

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-050 | Type `abc`, press `Left` twice, type `X`. | Draft becomes `aXbc`; cursor/editing works inside the line. |
| M-051 | Type `echo one two`, press `Ctrl-A`, type `printf `. | Text is inserted at the start of the draft. |
| M-052 | Type `echo one two`, press `Ctrl-E`, type ` three`. | Text is appended at the end of the draft. |
| M-053 | Type `echo one two`, press `Alt-B`. | Cursor moves to the previous word boundary. |
| M-054 | Type `echo one two`, press `Alt-F`. | Cursor moves to the next word boundary. |
| M-055 | Type `echo one two`, press `Ctrl-W`. | The previous word is deleted. |
| M-056 | Type `echo one two`, press `Ctrl-U`. | Text before the cursor is deleted. |
| M-057 | Type `echo one two`, press `Ctrl-K`. | Text after the cursor is deleted. |
| M-058 | Type `echo should-not-run`, press `Esc`. | Draft is cleared and no shell command runs. |
| M-059 | Type `echo before-clear`, press `Enter`, then press `Ctrl-L`, then run `echo after-clear`. | Screen is cleared/redrawn; `after-clear` runs normally. |
| M-060 | Press `Ctrl-X`, then an unsupported chord such as `Ctrl-G`. | The prefix is canceled; draft remains usable. |
| M-061 | Run `#help`. | Help lists private commands and keybindings, including completion, context, editor, picker, template, sync, and exit commands. |

## Modes And History

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-080 | At an empty draft prompt, press `Tab`. | Prompt changes from draft `>` to history `$` mode if history is available; otherwise mode still cycles predictably. |
| M-081 | Press empty `Tab` repeatedly. | Modes cycle through draft `>`, history `$`, AI `%`, then back to draft. |
| M-082 | Run `echo history-one`, then press empty `Tab` to history mode. | The newest history command is shown in read-only history mode. |
| M-083 | In history mode, press `Up` and `Down` after several commands. | Selection moves older/newer without editing the draft. |
| M-084 | In history mode with `echo history-one` selected, press `Enter`. | The selected history command executes again and output appears. |
| M-085 | In history mode, type `X`. | The selected command is copied to draft mode first, then `X` edits the copied draft. |
| M-086 | Run several commands, then exit and relaunch with the same `AISH_HOME`. | History is restored and newest commands are available in history mode. |
| M-087 | Type a draft without executing it, exit normally, then relaunch. | If draft persistence is enabled, the non-empty draft is restored. |
| M-088 | Run `#history 1`, then inspect history mode. | Combined regular/AI command history is trimmed to the requested limit; invalid counts are rejected with usage text. |

## Command Execution And Continuation

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-100 | Type `echo "` and press `Enter`. | Aish enters continuation mode with a shell-style continuation prompt; the command is not executed yet. |
| M-101 | Continue the previous test by typing `done"` and pressing `Enter`. | The completed command runs and prints `done`; prompt returns to draft. |
| M-102 | Type `echo '` and press `Enter`, then type `single'` and press `Enter`. | Single-quote continuation works and prints `single`. |
| M-103 | Type `echo backslash \` with a trailing backslash and press `Enter`, then type `continued` and press `Enter`. | Backslash continuation keeps the draft until the command is complete, then executes through the backend shell. |
| M-104 | Start an unfinished quote continuation, then press `Ctrl-C`. | Continuation is canceled, draft clears, and a new `> ` prompt is usable. |
| M-105 | Run a command that prints without a trailing newline, such as `printf no-newline`. | Output remains visible and the next prompt is correctly redrawn without corrupting the terminal. |
| M-106 | Run `printf 'stdout-ok\n'; printf 'stderr-ok\n' >&2`. | Both stdout and stderr lines remain visible before the next prompt. |
| M-107 | Run `printf 'unicode:%s\n' 'café-你好'`. | Unicode output is visible and prompt redraw remains correct. |

## Private Commands And Notes

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-120 | Run `#unknown-command`. | Aish prints an unknown-command message or suggestion; no backend command runs. |
| M-121 | Run `#completion`. | Completion config prints `completion.max_results`, `completion.ignore_spaces`, `completion.template_first`, `completion.inline`, and `completion.tab_accept`. |
| M-122 | Run `#editor`. | Editor configuration or resolution status prints; Aish does not launch an editor unless the editor keybinding is used. |
| M-123 | Run `#log 5`. | Recent event log entries print, or a readable no-log/no-events message appears. |
| M-124 | Run `# NOTE: manual note`. | The line is stored as a note and is not sent to the backend shell; no shell error appears. |
| M-125 | Run `# TODO: manual todo`. | The note is swallowed by Aish and the prompt returns. |
| M-126 | Run `#key`. | Usage text prints for `#key set | #key clear`. |
| M-127 | Run `#key set`. | A safe placeholder message prints; no secret is stored. |
| M-128 | Run `#key clear`. | If an encrypted key file exists, it is removed; otherwise a safe no-key message prints. |
| M-129 | Run `#encrypt on`. | A placeholder/warning behavior appears; Aish must not claim full encrypted storage is active. |
| M-130 | Run `#encrypt off`. | A safe placeholder/no-op behavior appears; no data migration occurs. |

## Completion

Use a clean work directory:

```text
mkdir -p /tmp/aish-manual-completion
cd /tmp/aish-manual-completion
touch alpha-one.txt alpha-two.txt unique-target.txt
```

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-150 | Type `cat unique-tar`, press `Tab`. | Inline ghost suggestion shows the untyped suffix for `unique-target.txt`; the below-prompt panel shows a labeled file candidate. Draft text is not changed yet. |
| M-151 | After M-150, press `Tab` again. | The draft becomes `cat unique-target.txt`; cursor is at the end. |
| M-152 | Type `cat alpha-`, press `Tab`. | A below-prompt panel shows at most `completion.max_results` labeled candidates; inline ghost shows the highest-ranked candidate suffix. |
| M-153 | Press `Ctrl-C` after a completion panel is visible. | Draft and panel clear; prompt remains usable. |
| M-154 | Type `zzzzzz-no-match`, press `Tab`. | A below-prompt `no completions` message appears; draft text remains unchanged. |
| M-155 | Type `cat unique-tar`, press `Right` at end of line. | Completion is accepted according to `completion.tab_accept`; draft becomes `cat unique-target.txt`. |
| M-156 | Type `cat unique-target.txt`, move cursor to the middle, press `Right`. | Cursor moves right normally; completion is not accepted. |
| M-157 | Run `#completion max 1`, then type `cat alpha-` and press `Tab`. | Only one below-prompt candidate row is shown, but inline acceptance still uses the best full candidate. |
| M-158 | Run `#completion max 0`. | Aish rejects the value with `completion max results must be greater than 0`; previous config remains unchanged. |
| M-159 | Run `#completion inline off`, then type `cat unique-tar` and press `Tab`. | Inline guidance is disabled, and `Tab` accepts the first ranked candidate directly. |
| M-160 | Run `#completion inline maybe`. | Aish rejects the value with usage text and does not change `completion.inline`. |
| M-161 | Run `#completion inline on`, then `#completion tab-accept full`. Type a prefix for a history command with spaces, press `Tab` twice. | Full accept inserts the complete remaining suggestion, including later words. |
| M-162 | Run `#completion tab-accept word`. Type a prefix for a history command with spaces, press `Tab` twice. | Word accept inserts only through the next whitespace boundary or next shell word; the rest of the suggestion remains unaccepted. |
| M-163 | Run `#completion tab-accept line`. | Aish rejects the unsupported mode with usage text and does not change `completion.tab_accept`. |
| M-164 | Create a very long filename, narrow the terminal, type its prefix, and press `Tab`. | Candidate rows remain on one line and elide at the right edge with ASCII `...`; rows do not wrap. |
| M-165 | Store a template with `#mt git-save git add . && git commit`, then type `git` and press `Tab`. | Template candidates rank before matching history commands and executables. |
| M-166 | Run `echo history-arg-value`, then type `echo history-` and press `Tab`. | Non-first-token completion can suggest `history-arg-value` from command history. |
| M-167 | Create directory `src/` and file `src/main.rs`, then type `cat src/` and press `Tab`. | Path candidates appear; directories use trailing `/`, regular files do not. |

## Templates

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-180 | Run `#mt greet echo hello`. | Aish prints `template stored: greet`. |
| M-181 | Run `#template list`. | The stored `greet` template appears. |
| M-182 | Run `#template show greet`. | The newest matching template body `echo hello` prints. |
| M-183 | Run `#template use greet`. | Draft becomes `echo hello` without executing immediately. |
| M-184 | Press `Enter` after M-183. | The template command executes and prints `hello`. |
| M-185 | Run `#mt ask echo {message}` then `#template use ask`, then press `Enter` without editing. | Execution is blocked with an unresolved-placeholder message. |
| M-186 | After M-185, edit `{message}` into `manual-ok` and press `Enter`. | Command runs and prints `manual-ok`; template-protection is cleared by editing inside the placeholder. |
| M-187 | Run `#template replace greet echo replaced`, then `#template show greet`. | The template body is replaced with `echo replaced`. |
| M-188 | Run `#template rm greet`, then `#template list`. | `greet` is no longer listed. |
| M-189 | Exit and relaunch with the same `AISH_HOME`, then list templates. | Templates persist across restarts unless removed. |

## Editor And Paste Review

These tests are easiest with a temporary editor script:

```sh
cat > /tmp/aish-manual-editor.sh <<'SH'
#!/bin/sh
printf 'echo editor-ok\n' > "$1"
SH
chmod +x /tmp/aish-manual-editor.sh
```

Set editor config before launch:

```toml
[editor]
command = ["/tmp/aish-manual-editor.sh"]
execute_after_save = false
```

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-200 | Press `Ctrl-X Ctrl-E`. | The configured editor runs; saved content returns as an opaque editor draft summary. |
| M-201 | Press `Enter` after M-200. | Editor content is submitted intentionally and prints `editor-ok`. |
| M-202 | Configure an editor script that exits non-zero, then press `Ctrl-X Ctrl-E`. | Aish reports editor failure and preserves the original draft. |
| M-203 | Paste a single-line command such as `echo pasted-one`. | Text is inserted into the draft; it does not execute until `Enter`. |
| M-204 | Paste multiple lines with default paste config. | Aish creates an opaque editor-review draft; pasted commands are not silently executed. |
| M-205 | Press `Enter` on the editor-review draft. | Reviewed multi-line content is submitted intentionally. |
| M-206 | Return editor content beginning with `# echo raw-editor-content`, then press `Enter`. | Editor-returned content bypasses Aish private-command parsing and is submitted as shell content. |

## Pickers And `fzf`

These checks require `fzf` or a fake `fzf` placed earlier in `PATH`.

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-220 | Press `Ctrl-R`, select a history item, and confirm. | The selected command replaces the draft without shell quoting. |
| M-221 | Press `Ctrl-R`, cancel the picker. | The original draft is preserved. |
| M-222 | Press `Ctrl-X Ctrl-F`, select a file path containing spaces. | The selected file replaces the current token with shell-safe quoting. |
| M-223 | Press `Ctrl-X Ctrl-F`, cancel the picker. | The original draft is preserved. |
| M-224 | Press `Ctrl-X Ctrl-T`, select a template. | The newest selected template body is copied into a protected draft. |
| M-225 | Press `Ctrl-X Ctrl-B` inside a git repository and select a branch. | The selected branch replaces the current token with shell-safe quoting. |
| M-226 | Press `Ctrl-X Ctrl-V`, select an environment variable. | The selected variable is inserted as a shell-compatible reference such as `$NAME`. |

## AI Config And Safe AI Prompting

Do not use a real secret for manual smoke tests.

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-240 | Run `#model test-model`. | Aish persists and reports `ai.model=test-model`. |
| M-241 | Run `#base-url https://example.invalid/v1`. | Aish persists the normalized final chat-completions URL ending in `/chat/completions`. |
| M-242 | Run `#env-key AISH_MANUAL_FAKE_KEY`. | Aish reports key source as environment-configured, but does not print key contents. |
| M-243 | Run `#status`. | AI model/final URL/key source are shown with secrets redacted or represented as source only. |
| M-244 | Type `# explain this command` with no AI URL/key configured. | Aish reports a readable AI config error and does not crash. |
| M-245 | If using a disposable compatible test endpoint, submit an AI prompt that returns command JSON. | Aish displays command items in AI mode and does not auto-execute them. |
| M-246 | In AI mode, press `Enter` on a selected command item. | Only the selected command executes; Aish advances to the next command item or returns to draft. |
| M-247 | In AI mode, type a character. | The selected AI command is copied to draft first, then edited. |

## Context Pseudo-Pipe

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-260 | Run `#context`. | Current context config prints. |
| M-261 | Run `#context off`, then `# summarize < echo hidden`. | Context capture is disabled; the context command does not run for AI context. |
| M-262 | Run `#context on`, then `# summarize < printf 'context-ok\n'`. | Aish asks for confirmation before running the context command. |
| M-263 | Answer `n` to the confirmation from M-262. | Context command is skipped, an event is logged, and no AI request is made from that context. |
| M-264 | Run `#context confirm off`, then use a safe context command. | Safe context command runs without confirmation when context is enabled and confirmation is off. |
| M-265 | Use a dangerous context command such as `# explain < rm -rf /tmp/aish-never-run`. | Aish refuses or requires confirmation despite `confirm off`; the dangerous command is not silently executed. |
| M-266 | Run `#context 16`, then context-capture output longer than 16 bytes. | Truncation is disclosed and captured context is byte-limited. |
| M-267 | Context-capture output containing token-shaped secrets. | Common secret-shaped values are redacted before being included in the AI prompt. |

## Sync

Use only local disposable git repositories.

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-280 | Run `#set-remote /tmp/nonexistent-aish-remote.git`. | Sync remote config persists; no network access is attempted by setting the remote. |
| M-281 | Run `#sync off`, then `#config`. | Sync is disabled in config. |
| M-282 | Run `#sync @hourly`, then `#config`. | Sync schedule persists as `@hourly`; no scheduler files are created. |
| M-283 | Run `#sync ai on`, `#sync history on`, `#sync templates on`, `#sync drafts on`. | Category toggles persist and are visible in config/status. |
| M-284 | Run invalid category usage such as `#sync unknown on`. | Usage text appears and existing sync config remains unchanged. |
| M-285 | Configure a local bare git remote and run `#push`. | Aish runs a conservative pull-rebase/add/commit/push flow for enabled managed files. |
| M-286 | Run `#push` with a missing local remote. | Aish reports a readable sync failure, logs the failure, and remains usable. |
| M-287 | Create a deterministic local conflict and run `#push`. | Aish reports the conflict/failure; it does not auto-resolve, rewrite history, or remove tracked files. |
| M-288 | Inspect the Aish home after sync commands. | No scheduler files were created by Aish. |

## Passthrough And Interactive Programs

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-300 | Run `less README.md` when `less` is installed. | Aish enters passthrough; navigation keys go to `less`; quitting `less` returns to the Aish prompt. |
| M-301 | While in passthrough, press app keybindings such as `Ctrl-R` or `Tab`. | Keys are forwarded to the interactive program, not interpreted by Aish. |
| M-302 | Run an allowlisted editor such as `vim` or `nvim` if installed. | Aish should hand off foreground interaction; after exiting, the Aish prompt should recover. |
| M-303 | Run `ssh invalid-hostname-for-manual-test` and cancel if needed. | Aish should not corrupt the prompt; after the command exits or is canceled, the prompt recovers. |
| M-304 | Run a non-allowlisted ordinary command. | Aish uses normal command-response execution, not passthrough. |

## Backend-Specific Manual Checks

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-320 | Run the Quick Smoke, Shell State, Completion, and Continuation sections with bash backend. | Behavior matches expected results; no bash prompt markers leak into visible output. |
| M-321 | Run the same sections with zsh backend. | Behavior matches expected results; no zsh prompt markers leak into visible output. |
| M-322 | Optionally run the same sections with fish backend. | Treat failures as compatibility findings; fish remains experimental until verified across platforms. |
| M-323 | Resize the terminal, then run `stty size`. | Backend child commands see the current terminal size or a correctly propagated PTY size. |
| M-324 | Use a narrow terminal and repeat completion panel checks. | Aish owns completion rendering; backend shell completion should not appear or change Aish behavior. |

## Production-Home Smoke

Use only if you intentionally want to test default home behavior.

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-340 | Unset `AISH_HOME`, set `HOME` to a disposable directory, and launch Aish. | Aish creates `$HOME/.aish` and starts. |
| M-341 | Store a template, run a command, and create a draft; exit and relaunch with the same disposable `HOME`. | Template, history, and draft behavior persist under `$HOME/.aish`. |
| M-342 | Run `#doctor`, `#config`, and `#status` in default-home mode. | Paths point to `$HOME/.aish`; behavior matches isolated `AISH_HOME` mode. |

## Exit And Cleanup

| ID | What To Do | Expected Behavior |
| --- | --- | --- |
| M-360 | Run `#exit`. | Aish exits cleanly. |
| M-361 | Relaunch and run `#quit`. | Aish exits cleanly. |
| M-362 | Start Aish, run several commands, then close with `Ctrl-D` on an empty prompt. | Aish exits cleanly and terminal echo/mode are restored. |
| M-363 | After any abnormal interruption, run `stty sane` if needed, then relaunch Aish. | Terminal should be recoverable; repeated corruption should be filed as a bug. |
| M-364 | Remove the disposable manual root directory. | No production files are affected. |

## Regression Notes To Record

When a manual test fails, record:

- Aish commit hash.
- Operating system and terminal emulator.
- Backend shell and version.
- Whether `AISH_HOME` or default `$HOME/.aish` was used.
- Exact command/key sequence.
- Expected behavior from this file.
- Actual visible behavior.
- Whether the prompt remained usable afterward.

Any bug found manually should get an automated regression test at the highest practical layer: pure Rust for logic, expect for user-visible interactive behavior, or tmux capture for final rendered terminal state.
