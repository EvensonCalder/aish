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
- Backend-driven passthrough for interactive commands through the persistent PTY shell.
- Conservative Git sync configuration and manual sync flow.
- GPG-backed key storage and encrypted managed JSONL storage.
- Real-terminal regression coverage through `tmux`.

Validation still pending or manual-only:

- Fish support is opt-in until behavior is validated across macOS and representative Linux distributions.
- Cross-platform validation for newly reported interactive passthrough programs remains ongoing.
- Real passphrase/pinentry, live AI provider behavior, and real remote authentication remain manual validation areas.

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

Submitted draft text from the prompt, external editor, and paste review enters the same Aish submit parser. If the submitted text starts with line-leading `#`, it is handled as an Aish private command, note, or AI prompt. Text parsed as ordinary shell input is sent to the backend shell unchanged.

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

Keybindings are configurable in `config.toml`. Each action takes an array of key sequences; each sequence is one key or a two-key chord. Use an empty array to disable an action.

```toml
[keybindings]
history_search = ["Ctrl-P"]
file_picker = ["Ctrl-G Ctrl-F"]
external_editor = ["Ctrl-X Ctrl-E", "Ctrl-O"]
```

Supported action names include `clear_or_cancel`, `exit_or_delete`, `clear_screen`, `move_start`, `move_end`, `delete_to_start`, `delete_to_end`, `delete_previous_word`, `delete_next_word`, `move_previous_word`, `move_next_word`, `move_left`, `move_right_or_accept_completion`, `previous_item`, `next_item`, `delete_previous_char`, `delete_next_char`, `cancel`, `complete_or_cycle`, `submit`, `history_search`, `external_editor`, `file_picker`, `template_picker`, `git_branch_picker`, and `env_var_picker`.

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
- Completion lexes shell-like words for matching while preserving the original history/template text for display and insertion. Quoted or escaped arguments such as `"hello world"`, `a"b c"d`, and `hello\ world` remain one shell word when accepted.
- Filesystem path completion matches against the shell word value, so quoted or backslash-escaped spaces work while typing. Accepted path replacements are escaped or quote-closed for the current word style instead of inserting raw shell metacharacters. `~/` is treated as HOME only when the leading `~` is unquoted and unescaped.
- Path completion can resolve missing intermediate directory components by exact directory descent, directory-prefix matches, and directory typo correction when fuzzy completion is enabled. Hidden entries are shown after visible entries for the same query.
- Typo correction is separate and uses `completion.typo_threshold_percent`; accepting a typo candidate replaces the mistyped command with the corrected command.
- `# ` AI prompts stay quiet. `#cmd` input only offers Aish private command names, and private command arguments use the same completion UI for nested subcommands such as `#completion mode tab` or `#encrypt rewrite-history plan`.

Completion sources:

- First token: templates, regular history, then PATH executables.
- Non-first token: structural template matches, structural history suffixes, template placeholders, history arguments, and filesystem paths.
- After a trailing space, Aish uses structural template/history matches and does not show unrelated filesystem entries for the empty token.
- Template completions use newest stored templates first.
- Paths preserve directory prefixes and mark directories with `/`. Matching local directories are kept ahead of lower-priority argument/history fallbacks, and recent directory scans are cached briefly while typing.
- With `completion.fuzzy=true`, Aish can also correct local directory typos such as `./srd` to `./src/` and intermediate path typos such as `srd/ma` to `src/main.rs`.
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

`#env-key` accepts shell-compatible variable names such as `OPENAI_API_KEY`.

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
#history list [>|>> <path> | | <command>]
#history search <query>
#history <count>
#ai list [>|>> <path> | | <command>]
#ai search <query>
#draft list [>|>> <path> | | <command>]
#draft search <query>
#mt <template-body>
#template list [>|>> <path> | | <command>]
#template search <query>
#template find <query>
#template show <id>
#template use <id> [key=value ...]
#template rm <id>
#template replace <id> <template-body>
#template remote add <name> <git-url>
#template remote list
#template remote rm <name>
#template publish <name> [--encrypt <key>]
#template fetch <name>
#template analyze <name> [query]
#template import <name> <id|all>
```

Sync:

```text
#set-remote <git-url>
#sync now
#sync resolve-union
#sync continue
#sync abort
#sync <schedule>
#sync off
#sync startup on|off
#sync exit on|off
#sync ai on|off
#sync history on|off
#sync templates on|off
#sync drafts on|off
```

Encryption:

```text
#key set
#key clear
#unlock
#encrypt on [key-fingerprint|unique-email]
#encrypt rotate <key-fingerprint|unique-email>
#encrypt unlock-mode lazy|prompt
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

List pseudo-pipe exports use this form:

```text
#history list > /tmp/aish-history.txt
#template list | grep deploy
```

`#history list`, `#ai list`, `#draft list`, and `#template list` print one command or template body per line. With `>` or `>>`, Aish writes the list directly to the target file with private file permissions where supported. With `|`, Aish feeds the list to the shell command on stdin. Both export forms require confirmation because the output can contain private history, AI output, drafts, templates, or secrets.

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

Saved editor content returns as an opaque editor draft. It is not executed until you press `Enter`, and pressing `Enter` runs it through the same submit parser used for direct prompt input.

For AI prompts, type `# ` and press `Enter` to open the editor for a multi-line AI instruction. `Ctrl-X Ctrl-E` on an existing `# ...` prompt opens the same AI prompt editor and preserves the current prompt body. The returned draft shows an AI prompt summary; pressing `Enter` sends it to the AI pipeline rather than to the backend shell.

Multiline paste defaults to editor-review behavior. Aish shows a compact draft summary and a bounded escaped preview instead of rendering the full pasted content inline. This prevents accidental execution and keeps the prompt usable. `Ctrl-X Ctrl-E` opens the full pasted content in the editor, and `Enter` submits the editor draft through the same parser used for direct prompt input.

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

Use `#template list` for one template body per line, `#template search <query>` for matching bodies, or `#template find <query>` when you need the stable `tpl-...` ID for `show`, `use`, `rm`, or `replace`.

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

Aish runs user commands through the persistent backend PTY shell and forwards terminal input while the command is running. On Unix, the backend shell runs in Aish's own PTY slave with a controlling terminal and a dedicated control fd for shell integration markers. Aish waits for the backend shell's ready/marker signal instead of using command-name matching or a fixed user-command deadline, so editors, pagers, REPLs, password prompts, stdin readers, and unknown terminal programs keep shell state and return to the Aish prompt when the backend shell reports completion.

Some full-screen programs may still expose terminal-specific edge cases because Aish translates frontend key events into PTY bytes instead of handing the controlling terminal directly to a child process. New real-world failures should get a tmux regression test.

## Sync

Sync is designed for the common two-machine case where both computers use the
same Aish Git remote. Aish writes a `README.md` notice into that sync data
repository so the remote is clearly identifiable as Aish-managed data.

Implemented:

- Persist remote and sync category config.
- Sync AI history, shell history and notes, templates, and drafts by default.
- Keep `config.toml`, cache, logs, secrets, and temporary files local by default.
- Write `README.md` into the sync data repository as a warning/guide for anyone opening the remote when it is absent or already Aish-managed.
- Write `.aish-sync.toml` into the sync data repository as non-secret metadata. It records the private sync content categories and, when encryption is enabled, the single full GPG fingerprint that current synced data must use.
- Persist a conservative subset of periodic schedules checked at startup.
- Persist explicit startup and exit sync triggers.
- Run `#sync now` against a configured Git remote.
- Stage managed enabled paths automatically, commit only when staged content changed, fetch the selected remote branch once into an isolated runtime cache, merge the cached `FETCH_HEAD`, then push once. If the remote changes before that push lands, Aish stops and asks you to rerun `#sync now` instead of starting a second fetch/merge/push round.
- For encrypted sync, decrypt every enabled managed `*.jsonl.gpg` file before staging, committing, or pushing. If GPG cannot decrypt the data on this machine, sync stops with the path and key-resolution guidance.
- Treat an empty bare/GitHub remote as a normal first-sync target: skip the cached merge when the remote has no branch, then push with upstream setup.
- Use an explicit selected remote branch for cached merges instead of relying on local Git branch tracking.
- Inspect remote sync metadata through the isolated runtime remote cache so a stale local `.aish-sync.toml` cannot be mistaken for the remote repository's current encryption state.
- Prefer the remote default branch when deciding the sync branch, then align the local sync branch before committing.
- Clear stale sync locks left by dead Aish processes before refusing a new sync.
- Stop an individual Git sync step after 60 seconds, report the timeout, release the sync lock, and return to the Aish prompt.
- Warn when existing Aish-managed files are present but excluded because their sync category is disabled.
- Retry cached merge with `--allow-unrelated-histories` when an existing local sync repository is connected to a populated remote with separate history.
- If `.aish-sync.toml` disagrees with local content category settings, warn and use the repository settings as the private sync authority. Existing local files excluded by those settings are left alone and only warned about.
- Stop before pushing if remote sync metadata disagrees with the local encryption config, or if local encrypted sync is configured with an email/selector instead of a full fingerprint.
- Use Git's union merge driver for plaintext Aish JSONL files so independent appends usually merge by keeping both sides.
- Before staging local data, compare enabled managed JSONL record counts against the fetched remote cache, for example `history/regular records local=3 remote=1 (local +2)`, without running another fetch. After merging remote updates, report managed JSONL record-count changes such as `history/regular records 1 -> 2 (+1)`. If an enabled managed record count decreases during the merge, automatically restore the JSONL union so neither side's records are deleted; if that restoration fails, sync stops before pushing.
- Offer `#sync resolve-union`, `#sync continue`, and `#sync abort` when a conflict still needs a user choice.
- Log sync failures without leaking secret-like values.

Aish does not:

- Auto-resolve encrypted `*.jsonl.gpg` conflicts, because text union can corrupt ciphertext.
- Rewrite history as part of sync. Encrypted storage history rewrite is a separate `#encrypt rewrite-history ... --confirm-rewrite-history` flow.
- Run `git rm --cached` automatically.
- Create scheduler files.
- Remove user-managed files.

Encryption key conflicts are deliberate stop points. If one machine and the
sync repository disagree on the fingerprint, choose one full fingerprint, make
sure the machine has the private key needed to decrypt the existing local and
repository data, run `#unlock` if needed, then run `#encrypt rotate
<chosen-full-key-fingerprint>` and `#sync now`. If this machine cannot decrypt
the data, import the needed private key or resolve the rotation on another
machine that can decrypt it.

Sync does not have a long-running scheduler. The supported automatic triggers are:

- periodic startup check: `#sync <schedule>` runs the same sync flow as `#sync now` at startup only when the saved interval is due. Supported forms are `@hourly`, `@daily`, `*/N * * * *`, `0 */N * * *`, `0 0 * * *`, and `0 0 */N * *`; unsupported schedules are logged and do not run git.
- every startup: `#sync startup on` runs the same sync flow once when Aish starts.
- exit: `#sync exit on` runs the same sync flow during the exit durability boundary.

Sync command boundaries:

- `#set-remote <git-url>` only saves the private sync remote. It does not initialize Git, fetch, merge, commit, or push.
- `#sync` prints current sync/encryption status and does not run Git.
- `#sync now` is the only manual sync run command. It verifies enabled managed data, stages enabled managed files, commits when staged content changed, merges remote updates, verifies/counts the merged data, then pushes.
- `#sync resolve-union` is only for an interrupted merge with plaintext Aish-managed JSONL conflicts. It keeps both sides, stages the resolved files, commits, and pushes.
- `#sync continue` is only for a merge that the user resolved and staged manually. It commits and pushes the interrupted sync.
- `#sync abort` is only for an interrupted merge or rebase. It cancels that Git operation.
- `#sync <schedule>`, `#sync off`, `#sync startup|exit on|off`, and `#sync ai|history|templates|drafts on|off` only update sync settings.

Local bare Git repositories work as remotes:

```sh
git init --bare ~/aish-sync.git
```

Then run this inside Aish:

```text
#set-remote /Users/you/aish-sync.git
#sync now
```

## Encryption Status

Aish uses the `gpg` command-line tool for encrypted local storage.

Current behavior:

- Configure `[encryption].key_fingerprint` in `config.toml`, or pass a key selector once with `#encrypt on <key-fingerprint>`.
- A full GPG key fingerprint is the stable key identity. An email/user ID is accepted only when GPG resolves it to exactly one public key.
- `#encrypt on` migrates managed history, notes, drafts, AI history, and templates to `*.jsonl.gpg` files and removes the plaintext JSONL files after successful encryption.
- If encryption is already enabled and the target fingerprint changes, Aish decrypts the existing managed encrypted files and re-encrypts them for the new fingerprint.
- `#encrypt rotate <key>` explicitly re-encrypts current managed storage for a new fingerprint. If `secrets/key.json.gpg` exists, the stored API key is re-encrypted for the new fingerprint too.
- `#encrypt unlock-mode lazy|prompt` selects startup behavior. `lazy` starts immediately and unlocks old encrypted history/templates in the background when possible; `prompt` requires interactive GPG/pinentry unlock before the Aish prompt opens.
- Future writes go to encrypted JSONL files while encryption is enabled. Normal history, draft, note, AI, and template appends are queued through a serialized background encrypted writer so command output and prompt redraws do not wait for GPG. Appends update one complete encrypted JSONL payload from the unlocked plaintext cache rather than concatenating multiple GPG messages.
- Aish flushes pending encrypted writes before exit, `#sync now`, `#history`, `#encrypt off`, key rotation, and confirmed history rewrite. A background write completion wakes the frontend tick path and refreshes live completion UI.
- Direct decrypt operations that may need a passphrase, including stored-key fallback, `#encrypt off`, key rotation, and confirmed history rewrite, enter a dedicated unlock passthrough state. Aish clears stale completion UI, yields terminal control to GPG/pinentry, sets `GPG_TTY` when possible, and restores the previous Aish mode when the operation completes or fails. Current-storage encryption changes snapshot managed files first and restore them if a migration step fails.
- In lazy startup mode, Aish tries to unlock history/templates in the background using noninteractive GPG so startup does not block on passphrase entry. If `gpg-agent` can decrypt without prompting, history/templates load automatically. If a passphrase is needed, old history/templates stay locked, history/AI views can show `history is still unlocking...`, and `#unlock` runs the interactive GPG/pinentry passthrough.
- Commands entered before lazy startup unlock completes remain usable. If old encrypted storage is still locked, new appends are buffered in memory and merged into encrypted storage when `#unlock` succeeds; old data-dependent views stay locked until then.
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

During sync, the non-secret `.aish-sync.toml` file is committed to the sync
repository and records that fingerprint plus the repository's content category
settings. `config.toml` remains local and is not committed.

### Template Sharing

The private sync remote is dynamic personal state for one user across machines.
Template sharing is a separate, more static publishing/import flow using named
Git remotes. A template remote must never stage private history, AI prompts,
drafts, notes, config, logs, cache, or secrets.

Commands:

```text
#template remote add shared git@github.com:you/aish-templates.git
#template remote list
#template publish shared
#template publish shared --encrypt friend@example.com
#template fetch shared
#template analyze shared [query]
#template import shared <id|all>
```

Template remotes are cached under `cache/template-remotes/<name>/repo`. Aish
publishes only `README.md`, `.aish-template-remote.toml`, and
`templates/templates.jsonl` or `templates/templates.jsonl.gpg`. Publishing keeps
existing remote templates by stable template ID and adds local templates, so a
shared template repository is not pruned just because one machine has fewer
templates. If no local templates exist, publishing still initializes the remote
with a README, metadata, and an empty template payload that the owner can edit
later.

`#template publish` and `#template fetch` take one remote-ref snapshot, fetch the
selected branch into the named local template cache, then work from that cache.
If a publish push is rejected because the remote changed, Aish refreshes that
snapshot once, merges by template ID again, and retries the push. `#template
analyze` and `#template import` do not contact the remote; they read only the
last fetched review cache.

By default, template payloads are plaintext. `#template publish <name> --encrypt
<key>` encrypts only the template payload for the chosen GPG recipient; the
remote README and metadata remain readable so importers can identify the remote.
Fetching, analyzing, and importing encrypted template remotes require the
matching private key on the local machine.

Fetching only updates the review cache. `#template analyze` compares fetched
templates with local templates and marks each fetched template as `new` or
`present`. Local templates change only after `#template import`. Imports are
deduplicated by stable template ID/body hash. Existing local templates are
reported as already present and are not overwritten. If a template remote
appears to be a private Aish sync repository, Aish refuses to use it and asks
for a separate template remote.

Template sharing command boundaries:

- `#template remote add|list|rm` only manages named template remote config. It does not fetch, publish, or import templates. Removing a remote, or repointing an existing remote name to a different URL, clears that name's local review cache so later analyze/import cannot read stale templates from the old URL.
- `#template publish <name>` writes local templates to the named template-only remote. Plaintext is the default; `--encrypt <key>` encrypts only the payload for the chosen recipient.
- `#template fetch <name>` updates only the local review cache for that remote.
- `#template analyze <name> [query]` reads the fetched cache and local template store, reports `new` or `present`, and does not write local templates.
- `#template import <name> <id|all>` is the only template sharing command that changes the local template store. It appends missing templates and does not overwrite existing ones.

To rotate to a new key:

```text
#encrypt rotate FEDCBA9876543210FEDCBA9876543210FEDCBA98
```

This rewrites current managed storage by decrypting with the available old private key and encrypting to the new fingerprint. If a stored API key exists, it is re-encrypted for the new fingerprint. Git history is not rewritten automatically.

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

If encrypted history/templates need a passphrase after startup:

```text
#unlock
```

To require GPG/pinentry before the first prompt on future launches:

```text
#encrypt unlock-mode prompt
```

To return to nonblocking startup with explicit unlock:

```text
#encrypt unlock-mode lazy
```

To stop using the stored key:

```text
#key clear
```

To decrypt managed storage back to plaintext and write plaintext files from then on:

```text
#encrypt off
```

Known limits: lazy startup does not automatically pop up pinentry while you are typing; run `#unlock` when passphrase entry is needed, or switch to `#encrypt unlock-mode prompt` if startup must require the passphrase. Direct decrypt operations already use the dedicated GPG/pinentry passthrough path. Aish warns that Git history can contain plaintext data or data encrypted for an older key; history rewrite is available only through the explicit confirmed command above.

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
  key.json.gpg
cache/
  runtime/
.gitignore
```

Use `#doctor`, `#status`, and `#config` to inspect the active paths and runtime settings.

On Unix-like systems, Aish creates managed storage directories with private directory permissions and writes config/history/encrypted storage files with private file permissions where supported.

## Code Organization

The app module root in `src/app.rs` wires together focused runtime modules:

- `src/app/state.rs`: `AppState`, output ring state, storage-backed draft/history/template persistence, and encrypted writer lifecycle.
- `src/app/editor_state.rs`: editor roundtrips and paste/editor draft replacement state.
- `src/app/selection_state.rs`: history/AI selection, draft clearing, key-prefix state, and unlock passthrough mode transitions.
- `src/app/bootstrap.rs`: startup layout/config/history/template loading and terminal launch.
- `src/app/completion_runtime.rs`: AppState completion request/cache orchestration.
- `src/app/config_commands.rs`: `#model`, `#base-url`, `#env-key`, `#context`, `#paste`, and `#completion` config mutations.
- `src/app/context_prompt.rs`: AI prompt submission, context collection confirmation, and contextual prompt building.
- `src/app/encryption_commands.rs`: GPG key storage, `#encrypt`, current-storage rotation, and confirmed history rewrite.
- `src/app/startup_unlock.rs`: lazy and prompt encrypted startup unlock loading and encrypted cache preparation.
- `src/app/execution.rs`: draft submission, backend-driven command passthrough, PTY output forwarding, and command recording.
- `src/app/history_ops.rs`: history trimming and encrypted/plain AI history loading helpers.
- `src/app/template_args.rs`: template subcommand argument parsing.
- `src/app/event_log.rs`: event log display for `#log`.
- `src/app/reports.rs`: `#status`, `#config`, `#doctor`, `#editor`, and encryption/sync status output.
- `src/app/sync_commands.rs`: `#set-remote`, `#sync`, startup/exit sync triggers, and git step handling.
- `src/config.rs`: public config module facade and config tests.
- `src/config/`: config model types, directory layout, private file permissions, root path resolution, file IO, and normalization.
- `src/completion.rs`: completion orchestration across templates, history, paths, private commands, and typo tiers.
- `src/completion/`: focused completion helpers for shared types, indexed history/template words, matching rules, token parsing, path/PATH scanning, private command completion, and rendering/acceptance.
- `src/encryption/keys.rs`: GPG public key listing, fingerprint normalization, and selector ambiguity handling.
- `src/terminal.rs`: terminal event loop, key/paste handling, and picker/editor boundaries.
- `src/terminal/completion_ui.rs`: live completion display, inline suffixes, Tab/Right acceptance, and completion panel state transitions.
- `src/terminal/render.rs`: prompt redraw positioning, render anchors, cursor placement, and screen-area reservation.
- `src/pty.rs`: PTY backend lifecycle, command execution, read loop, foreground input bridge, and streaming event callbacks.
- `src/pty/`: Unix `openpty` backend, control fd setup, shell launch setup, randomized marker protocol, marker parsing, output filtering, and continuation syntax helpers.

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
cargo test --test expect_runner -- --nocapture
cargo test --test tmux_capture -- --nocapture
cargo clippy --all-targets -- -D warnings
git diff --check
cargo build
```

Shorter all-Rust pass:

```sh
cargo test
```

Current active inventory:

- 612 library unit tests.
- 28 draft execution integration tests.
- 1 first-run integration test.
- 33 PTY integration tests, with bash/zsh active by default and fish-specific cases opt-in.
- 120 expect-driven end-to-end interactive scenarios.
- 50 tmux screen-capture integration tests.

Expect and tmux tests launch real terminal sessions with isolated Aish homes and per-test artifact directories. Both harnesses run with bounded parallelism by default; use `AISH_EXPECT_TEST_JOBS=1` or `AISH_TMUX_TEST_JOBS=1` when debugging a timing-sensitive failure. Long-running scenarios time out by default; override with `AISH_EXPECT_TEST_TIMEOUT_SECS` or `AISH_TMUX_TEST_TIMEOUT_SECS` only for diagnosis.

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

If a command appears to wait for stdin or take over the terminal incorrectly, test it in a normal shell and then in Aish. Aish should keep backend PTY commands usable when they wait for stdin or claim alternate screen; new real-world failures should get a tmux regression test.
