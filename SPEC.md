# Aish SPEC

> **Aish Is not a SHell.**  
> Aish is a smart command-input layer on top of a real shell running inside a PTY.

Aish does **not** try to replace Bash, Zsh, or Fish. The backend shell remains responsible for command execution, shell syntax, environment state, process control, aliases, functions, job control, and user shell configuration. Aish owns the interactive input experience: editable drafts, read-only history browsing, AI-generated command browsing, templates, safe AI prompting, completion, search, encryption, and conservative synchronization.

---

## 1. Core principles

1. **Aish is not a shell.** It is a PTY-based command editor and assistant.
2. **Ordinary commands are sent to the real shell unchanged.**
3. **Line-leading `#` in the Aish prompt is reserved for Aish** and is never sent to the backend shell, except for explicitly executed context commands after `<`.
4. **AI never silently edits or executes commands.** AI can generate candidates; users execute one command at a time.
5. **History and AI results are read-only sources.** Any edit copies the selected item into draft mode.
6. **Draft is the only editable command state.**
7. **Multi-line pasted content does not enter draft by default.** It opens an editor-review flow or executes with warning, depending on config.
8. **Only direct prompt input parses Aish commands.** Editor, paste-review,
history, template, and AI-mode command drafts are submitted to the backend shell
as shell text; a leading `#` there is shell syntax.
9. **Encryption prioritizes confidentiality over speed.** When encrypted, Aish does not persist plaintext search indexes.
10. **Git sync is conservative.** Sync only auto-resolves Aish-managed append-only JSONL conflicts where both sides can be preserved; it never auto-resolves unmanaged conflicts, never rewrites history, and never runs `git rm --cached` automatically. Encrypted-storage Git history rewrite is a separate explicit command with destructive confirmation.
11. **Backend shell history is not Aish history.** Aish stores executed commands in its own managed history and must suppress Bash, Zsh, and Fish native history in the backend shell so Aish-submitted commands are not written to `~/.bash_history`, `~/.zsh_history`, Fish history files, or their in-memory native history lists.
12. **End-to-end behavior must be tested as users experience it.** Every user-visible feature needs expect-driven coverage in addition to Rust unit/integration tests, including rendering, redraw boundaries, PTY output framing, shell continuation, keybindings, mode transitions, error paths, and regression cases.

---

## 1.1 End-to-end test requirements

Expect tests are first-class acceptance tests for Aish. Unit tests may prove pure parsing or state transitions, but they are not enough for terminal behavior.

Required coverage rules:

- Every private `#` command must have at least one expect scenario for success or safe failure.
- Every keybinding that changes user-visible state must have expect coverage or a documented reason why it cannot be exercised outside a pure Rust terminal test.
- Every prompt redraw path must be covered by a screen-level assertion, including command output followed by prompt redraw, clear-screen behavior, completion panels, mode switches, continuation prompts, and editor/paste returns.
- Every PTY output framing fix must have a regression test proving output remains visible in the terminal, not just present in an internal string.
- For prompt/output regressions, a screen-level assertion must validate final visible terminal state. Byte-stream assertions alone are insufficient when later redraw, clear, or cursor movement can visually erase already-emitted output.
- Every backend shell integration change must have Rust PTY coverage and, where practical, an expect scenario through the real binary.
- If a real manual-use bug is missed by existing expect-byte-stream tests, add a persistent real-terminal capture script, such as a `tmux`-driven pane capture workflow, and use that as the acceptance regression.
- `tmux capture-pane -p` trims trailing spaces from captured lines. Tmux tests must validate final visible screen content, but trailing-space-sensitive prompt assertions require expect byte-stream checks or Rust rendering tests.
- Every safety feature must have both a direct test and an end-to-end test for the user-visible behavior.
- Every new bug fix must add a regression test that fails for the observed bug, preferably at the highest layer where the bug was visible to the user.
- Expect scenarios must use isolated `AISH_HOME`, avoid network access, avoid persistent user-home side effects, and cleanly exit.
- Expect scenarios that drive real interactive terminals should be serialized unless a scenario explicitly proves concurrent behavior. Parallel execution can introduce scheduler and terminal races that do not match normal user operation.

The test suite should model real user workflows and edge cases, not just happy-path command strings.

---

## 2. High-level architecture

```text
Keyboard / terminal
        |
        v
+-------------------------------+
| Aish frontend                  |
| - input editor                 |
| - mode state machine           |
| - completion and pickers       |
| - AI command generation        |
| - history/template storage     |
| - encryption/sync/logging      |
+---------------+---------------+
                |
                | PTY master
                v
+---------------+---------------+
| PTY slave                     |
| backend shell: bash/zsh/fish  |
+---------------+---------------+
                |
                v
        real commands/programs
```

Aish should default to the user's `$SHELL`, falling back to `/bin/bash` if unavailable. The backend shell runs persistently in a PTY so that `cd`, `export`, aliases, functions, `source`, job control, and interactive programs behave like a normal shell.

---

## 3. Modes

Aish has three primary prompt modes and several temporary operational modes.

### 3.1 Primary modes

Default prompt symbols:

```text
<user>@<host> <dir> >   draft mode
<user>@<host> <dir> $   history mode
<user>@<host> <dir> %   AI mode
```

Users can customize prompt shape and symbols.

#### `>` Draft mode

Draft mode is the only editable command-entry mode.

Behavior:

- New input happens here.
- Editing happens here.
- Executing a draft sends its content to the backend shell unchanged.
- Ordinary executed drafts are appended to regular history, then the active prompt returns to a new blank draft.
- Up/down navigation browses saved draft entries only; regular history is browsed in history mode.
- Entering draft mode through empty `Tab` always opens a blank draft prompt.
- Startup loads draft history but must not auto-fill the prompt with the previous draft; `Up` restores saved drafts explicitly.
- `Up` from a blank draft restores the newest saved draft when draft persistence is enabled.
- `Up` / `Down` move older/newer through saved drafts.
- `Down` from the newest saved draft opens a blank draft.
- `Down` from a non-empty new draft saves the current draft and opens a blank draft without executing it.
- If draft content is empty, `Tab` switches modes.
- If draft content is non-empty, `Tab` opens or accepts completion according to completion configuration.

#### `$` History mode

History mode browses regular executed history.

Behavior:

- Read-only.
- Up/down navigates regular history only.
- `Enter` executes the selected history item.
- The re-executed command is appended as a new regular history entry.
- Any modification attempt copies the selected item into draft mode and then applies the edit.
- Cursor movement does not count as modification.

#### `%` AI mode

AI mode browses AI-generated command items.

Behavior:

- Read-only.
- Up/down navigates AI-generated command items.
- AI results are grouped internally as sessions, but the user browses command items in execution order.
- Entering AI mode through empty `Tab` preserves the current AI item pointer if it is still valid.
- A new AI query refreshes the AI item pointer to the first command item in the new session.
- `Enter` executes only the currently selected AI command.
- If execution succeeds and there is a next command in the same AI session, Aish selects the next AI command.
- If execution succeeds and this is the last command in the session, Aish returns to `>` draft mode.
- If execution fails, Aish stays on the current AI command and does not advance.
- Executed AI commands are appended to regular history with `source = "ai"`.
- Any modification attempt copies the selected item into draft mode and then applies the edit.
- Cursor movement does not count as modification.
- There is no execute-all shortcut. In particular, **no `Alt-Enter` execute remaining commands** feature.

### 3.2 Temporary modes

#### CommandRunning mode

Entered after a command is submitted to the backend shell.

- Aish observes PTY output.
- `Ctrl-C` sends interrupt behavior to the PTY foreground process group.
- When shell integration or prompt marker reports command completion, Aish returns to the appropriate primary mode.

#### Passthrough mode

Used for interactive programs that need to own terminal input, such as `vim`, `nvim`, `ssh`, `top`, `less`, `fzf`, `python`, `node`, `psql`, `sudo`/`doas` password prompts, or similar.

- Keyboard input is forwarded to the PTY.
- Output is displayed as-is.
- Aish keybindings are mostly disabled.
- Return to normal mode occurs when the backend shell reports command completion through the shell integration marker/control protocol.
- Correctness must not depend on command-name allowlists, prompt guessing, or fixed user-command timeouts. Aish runs user commands through the persistent backend PTY shell, bridges terminal input/output while the command is foregrounded inside that PTY, and waits for the backend shell completion signal.

#### ExternalEditor mode

Triggered by `Ctrl-X Ctrl-E` or configured editor actions.

- Opens the configured editor with current draft content.
- If invoked from history or AI mode, selected read-only item is copied to draft first.
- On save and exit, editor content replaces the current draft buffer.
- By default, content is **not executed automatically**. The user presses `Enter` to execute.
- Optional config can enable execute-after-save for users who want that behavior.
- Content from this mode is submitted to the backend shell as shell text. A
  leading `#` in saved editor content is a shell comment, not an Aish command.

#### PasteReviewEditor mode

Legacy name for the multi-line paste review state. In the current design, this is represented as an opaque editor draft rather than a separate inline editor.

- Multi-line paste with `paste.multiline = "editor"` becomes an opaque editor draft.
- The main prompt shows only the editor draft summary.
- `Ctrl-X Ctrl-E` can open the external editor for that content.
- `Enter` submits the editor draft to the backend shell as shell text.
- `paste.multiline = "execute"` with `confirm_execute = true` also uses the editor draft as the confirmation step.

#### Picker modes

Temporary UI modes for selecting files, history, templates, git branches, environment variables, or other insertable values.

- Picker result is inserted into draft at cursor, replaces current token, appends as an argument, or replaces the whole line depending on picker action.
- Picker shortcuts should avoid conflicting with common readline keys by default.

#### UnlockPassthrough mode

Used when GPG or pinentry needs interactive terminal control.

- Aish temporarily pauses normal UI and clears stale completion UI.
- Direct GPG decrypt, key migration, and history rewrite operations give GPG/pinentry dedicated terminal control.
- Aish sets `GPG_TTY` when a controlling TTY is known and asks `gpg-agent` to update its startup TTY.
- After completion or failure, Aish restores raw mode and the previous Aish mode before redrawing.
- Startup encrypted history/template decrypt starts in a noninteractive background unlock attempt. If GPG needs a passphrase, Aish remains usable with locked history/templates until `#unlock` runs the dedicated GPG/pinentry passthrough.

---

## 4. Prompt customization

Default prompt variables:

```text
{user}
{host}
{cwd}
{basename}
{git_branch}
{mode}
{last_status}
```

Example config:

```toml
[prompt]
draft = "{user}@{host} {cwd} > "
history = "{user}@{host} {cwd} $ "
ai = "{user}@{host} {cwd} % "
```

The backend shell prompt should not be shown directly. Aish should use backend shell integration or an invisible prompt marker to detect shell readiness.

---

## 5. Input semantics

### 5.1 Ordinary commands

Any line not beginning with `#` in the Aish prompt is ordinary input. When executed, it is sent to the backend shell unchanged.

Examples:

```bash
git status
cargo test
kubectl get pods -n production
```

### 5.2 Line-leading `#`

Line-leading `#` in the Aish prompt is reserved for Aish. It is never sent directly to the backend shell.

Types:

```text
#<private-command> ...   Aish private command
# <prompt>               AI prompt
# <prompt> < <command>   AI prompt with context command
```

This reservation applies only to direct prompt input. External editor drafts,
paste-review drafts, history/AI selections, and template drafts are submitted to
the backend shell as shell text; a leading `#` in those paths is shell syntax.

### 5.3 Reserved hash-line behavior

Common comment-like prefixes such as `# TODO:` and `# NOTE:` are not special
annotation syntax. A line using `#<name>` private-command form must dispatch
to that private command or report an unknown Aish command. A line using
`# <prompt>` is an AI prompt by definition.

### 5.4 Private command syntax

Private commands use `#<name>` with no required space after `#`:

```text
#model gpt-4.1
#base-url https://example.com/v1
#env-key OPENAI_API_KEY
#prompt draft "{basename} > "
#prompt reset
#key set
#unlock
#encrypt on
#history 20000
#context 65536
#mt mv {from} {to}
```

Unknown private commands should never be sent to the shell. Aish should show an error and possible suggestions.

### 5.5 AI prompt syntax

AI prompts use `# ` followed by arbitrary prompt text:

```text
# how to set remote for my git repo?
# generate a command to find files larger than 100MB
```

AI-generated results enter `%` AI mode.

`# ` followed only by whitespace and `Enter` opens the configured editor for a multi-line AI prompt body. `Ctrl-X Ctrl-E` on a `# ...` AI prompt also opens this AI prompt editor, using the current prompt body as the initial editor content. On save, Aish returns an opaque AI prompt draft summary; `Enter` sends that content to the AI pipeline, not to the backend shell.

### 5.6 Pseudo-pipe context syntax

Aish supports a pseudo-pipe using `<`:

```text
# how to set remote for my git repo? < git -h && git remote -h
```

Meaning:

- The left side is an AI prompt.
- The right side is a shell command used only to collect context for AI.
- The right side may be executed by Aish after applying context feature rules.
- The context command itself is not the generated command.

Configuration:

```toml
[context]
enabled = true
confirm = true
max_bytes = 65536
```

Behavior:

- If `context.enabled = false`, `<` context collection is disabled.
- If `context.confirm = true`, Aish asks before executing the context command.
- If `context.confirm = false`, Aish may execute context commands without prompting, but should still force confirmation or block clearly dangerous commands.
- Output is capped at `context.max_bytes` bytes.
- Context commands are timeout-limited; timed-out subprocesses should be terminated, including their process group where supported.
- Truncation must be disclosed to AI and optionally to the user.

Confirmation example:

```text
aish will run this command to collect context:

  git -h && git remote -h

Run context command? [Y/n]
```

### 5.7 Continuation rules

Aish supports visual continuation for `#` prompt and `#mt` template creation.

Ordinary draft submission should also feel like a real shell when the typed input is incomplete:

- Incomplete quote input such as `echo "` or `echo '` remains in Aish as a continuation draft instead of sending a partial command to the backend shell.
- The continuation prompt mirrors shell conventions (`dquote> `, `quote> `, or generic `> `) without leaking backend `PS2`/`PROMPT2` prompts into displayed command output.
- A trailing odd backslash, such as `echo foo \`, is treated as a line continuation because interactive shells continue those inputs.
- Aish continuation detection is intentionally lexical and conservative: it covers unfinished single/double quotes and odd trailing backslashes without trying to fully parse Bash, Zsh, or Fish syntax. Complex shell grammar remains the backend shell's responsibility.
- `Ctrl-C` or `Esc` clears a continuation draft and returns to a normal prompt without wedging backend shell state.
- Completed multi-line input is submitted as one command string and stored faithfully in history.

AI prompt with context:

```text
# how to set remote for my git repo? < \
#   git -h && \
#   git remote -h
```

Template creation:

```text
#mt rsync -avz {from} \
#mt   {user}@{host}:{to}
```

Internal parsing removes the repeated visual prefixes (`# ` or `#mt `) before parsing content.

---

## 6. External editor and paste behavior

### 6.1 Editor command selection

Editor resolution order:

1. `config.editor.command`
2. `$VISUAL`
3. `$EDITOR`
4. `nvim`
5. `vim`
6. `vi`

Recommended config:

```toml
[editor]
command = ["nvim"]
execute_after_save = false
```

`execute_after_save = false` is the safe default.

### 6.2 `Ctrl-X Ctrl-E`

Default behavior:

1. Open editor with current draft buffer.
2. User edits and saves.
3. Aish reads saved content.
4. Content becomes an editor draft.
5. User presses `Enter` to execute.

If invoked from history or AI mode:

1. Copy selected item to draft.
2. Open editor with copied content.
3. Save back to draft.

Editor drafts are opaque in the main prompt. Aish should show a summary such as line count and byte count instead of rendering the full content inline. `Ctrl-X Ctrl-E` edits the editor draft again. `Enter` executes it.

AI prompt editor drafts are a separate editor-draft subtype. They use the same opaque summary pattern, but `Enter` sends the editor content to the AI request path. They must not be treated as raw shell input, and `editor.execute_after_save` must not auto-send them.

Editor content is visually opaque in the prompt, and submission sends the saved
content to the backend shell exactly as shell text. If the saved content starts
with `#`, the backend shell treats it as shell syntax.

`execute_after_save = false` means editor exit only writes back an editor draft.
It does not execute. If `execute_after_save = true`, Aish submits only after a
successful editor exit status and preserves the same shell-submission semantics
as pressing `Enter` on that editor draft.

### 6.3 Multi-line paste

Aish should enable bracketed paste mode.

Single-line paste:

- Insert at cursor in draft mode.
- If in read-only history or AI mode, copy selected item to draft first, then paste.

Multi-line paste:

- Must not execute by default.
- Default behavior creates an opaque editor draft using the pasted content.
- Alternative behavior can be direct execution after warning.
- Paste preview shows bounded, escaped preview content below the opaque draft summary without placing raw multi-line paste inline in the draft prompt.

Config:

```toml
[paste]
multiline = "editor"       # editor | execute | discard
confirm_execute = true
preview = true
preview_lines = 3
preview_bytes = 240
```

`multiline = "editor"`:

1. Convert pasted content to an editor draft.
2. Show only the editor draft summary in the main prompt.
3. `Ctrl-X Ctrl-E` can reopen the content in the external editor.
4. `Enter` submits the editor draft through the normal parser; ordinary shell content goes to the backend shell.
5. Do not auto-execute on paste.
6. If `preview = true`, show an escaped preview capped by `preview_lines` and `preview_bytes`.

`multiline = "execute"`:

1. If `confirm_execute = true`, convert pasted content to an editor draft and wait for `Enter`.
2. If `confirm_execute = false`, submit pasted content immediately through the normal parser.
3. Store executed content in history as the exact submitted command string.

`multiline = "discard"`:

- Ignore multi-line paste and show a message.

### 6.4 History semantics for multi-line raw submissions

History stores exactly what Aish submits to the backend shell.

For v0.1, a multi-line editor draft is stored as one complete history command string. Backslash continuations, comments, heredocs, and quoted newlines remain part of that single history item.

Future shell-aware splitting can be added as a configurable enhancement, but it must not be the default unless it can preserve shell semantics reliably. A splitter would need to handle:

- Split on newlines that end a complete shell command.
- Preserve line continuations ending with `\` as part of one logical command.
- Preserve quoted multi-line strings as part of one logical command.
- Preserve heredoc blocks as part of one logical command.
- Ignore blank lines.
- Preserve comment-only lines when faithfully storing a multi-line draft for replay.

Until that exists, history must prefer faithful replay over prettier browsing.

---

## 7. History model

### 7.1 History categories

Aish stores at least three categories:

```text
regular history   executed shell commands
draft history     unfinished or user-created drafts
AI history        AI-generated command items grouped by session
```

Optional non-executable annotations may be stored separately in the future, but
there is no implicit `# TODO:`/`# NOTE:` input syntax for them.

### 7.2 Read-only source rule

Regular history and AI history are read-only browsing sources.

- Editing a history item copies it to draft.
- Editing an AI item copies it to draft.
- Moving the cursor does not count as editing.
- Executing a history or AI item appends a new regular history entry.

### 7.3 Draft behavior

Draft mode is a writable draft history. Draft entries are separate from regular executed command history.

Draft browsing behavior:

- `Up` from a blank draft restores the newest saved draft from the loaded draft history.
- `Up` / `Down` move older/newer through saved drafts.
- `Down` from the newest saved draft clears the prompt and stays in draft mode.
- `Down` from a non-empty new draft saves the current draft when draft persistence is enabled, then clears the prompt.
- Navigating away from an edited saved draft stores the edited text as a new draft entry instead of mutating the old entry.
- If saving fails, Aish must leave the current draft intact.
- Pressing `Enter` on an ordinary draft executes a copy of the draft, appends that copy to regular history, and returns the active prompt to a new blank draft. The saved draft entry remains available through draft history when draft persistence is enabled.

Draft persistence:

```toml
[draft]
persist = true
sync = false
```

Drafts may be encrypted when encryption is on. Draft sync should be user-configurable.

### 7.4 AI session storage

AI sessions should be compact.

Example `history/ai.jsonl` entry:

```json
{
  "id": "a_20260511_120001",
  "t": 1778526001,
  "prompt": "set git global user name and email",
  "ctx": true,
  "model": "model-name",
  "items": [
    {"kind": "command", "text": "git config --global user.name \"{name:your git user name}\""},
    {"kind": "command", "text": "git config --global user.email \"{email:your git email}\""}
  ]
}
```

Command boundaries are determined by JSON `items`, not by newlines inside an item.

### 7.5 Native shell history boundary

Aish is the owner of command history for commands submitted through the Aish prompt, editor flow, history mode, AI mode, and template execution. The persistent backend shell must not treat those commands as ordinary interactive shell history.

Backend requirements:

- Bash backend sessions disable native history, unset `HISTFILE`, keep `HISTSIZE=0`, and clear the in-memory native history list after startup and around prompt-ready handling.
- Zsh backend sessions use an empty private history stack and keep `HISTFILE` unset with `HISTSIZE=0` and `SAVEHIST=0`; append/share history options from user rc files must be neutralized after rc loading.
- Fish backend sessions use Fish private mode when available, set an empty `fish_history` value, and clear the current session history after startup and post-execution.
- Aish internal marker commands and user-submitted commands should be absent from backend shell native history queries such as Bash `history`, Zsh `fc -l`, and Fish `history search`.
- Forcing a native history flush from inside the backend shell, such as Bash `history -a`, Zsh `fc -W`, or Fish `history save`, must not append Aish-submitted commands to the user's native history files.
- Aish must not delete or rewrite preexisting user shell history files. The suppression boundary applies to the backend shell session used by Aish.

### 7.6 History limit

Command:

```text
#history <count>
```

Meaning:

- Limits total stored command items across regular history and AI command items.
- AI sessions count by item count, not by session count.
- If all items in an AI session are trimmed, the session can be removed.

### 7.7 Failed commands

Failed commands should be stored with exit code. Failed commands are useful for later search and AI repair prompts.

---

## 8. AI behavior

### 8.1 AI request policy

AI requests should use only explicit user prompts or explicit configured features. Aish should not silently upload command history, file contents, secrets, or large outputs.

Default context limits:

```toml
[context]
max_bytes = 65536
```

Aish should redact common secret patterns before sending context where feasible.

### 8.2 Thinking/reasoning output policy

All AI requests must discard thinking/reasoning results.

Aish should only accept final structured output. It should not store, display, or rely on chain-of-thought style reasoning.

### 8.3 AI output schema

AI should return valid JSON only.

Schema:

```json
{
  "items": [
    {
      "kind": "command",
      "text": "shell command here"
    },
    {
      "kind": "template",
      "name": "template-name",
      "text": "template command here"
    }
  ]
}
```

Rules:

- `items` must be ordered by intended execution order.
- Items must be directly relevant to the user's request.
- `kind = "command"` means a candidate command for `%` AI mode.
- `kind = "template"` means a suggested template candidate, not automatically saved.
- Template suggestions are not persisted unless the user explicitly creates a template with `#mt`.
- Placeholders use `{name}` or `{name:description}`.
- Generic filler words such as `something`, `file`, `path`, `pattern`, `name`, or `value` should become brace placeholders in generated command text instead of literal shell arguments.
- Commands containing unresolved placeholders must not execute until replaced.

Example user prompt:

```text
# global set git email and username
```

Expected AI output:

```json
{
  "items": [
    {
      "kind": "command",
      "text": "git config --global user.name \"{name:your git user name}\""
    },
    {
      "kind": "command",
      "text": "git config --global user.email \"{email:your git email}\""
    }
  ]
}
```

### 8.4 AI execution behavior

- `Enter` executes only the currently displayed AI command.
- If it succeeds, Aish moves to the next command in the same session.
- If it fails, Aish stays on the failed command.
- If the last command succeeds, Aish returns to draft mode.
- No execute-all shortcut is provided by default.

---

## 9. Templates

### 9.1 Creation

Only explicit `#mt` creates templates.

```text
#mt <template-body>
```

Examples:

```text
#mt mv {from} {to}
#mt git config --global user.name "{name}" && git config --global user.email "{email}"
```

Multi-line:

```text
#mt rsync -avz {from} \
#mt   {user}@{host}:{to}
```

Aish prints a stable `tpl-...` content-hash ID for stored templates. Users use that ID for exact operations:

```text
#template find <query>
#template list [>|>> <path> | | <command>]
#template search <query>
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

`#template list` prints one template body per line newest-first. `#template search <query>` prints matching bodies. The `list` form supports the same privacy-confirmed `>` / `>>` / `|` export pseudo-pipe as `#history list`, `#ai list`, and `#draft list`. `#template find <query>` is for operations that need stable `tpl-...` IDs. AI may suggest template items, but Aish never auto-saves them.

### 9.2 Placeholder syntax

Supported placeholders:

```text
{name}
{name:description}
{name...}
```

Recommended internal representation:

```text
Segment::Text("mv ")
Segment::Placeholder { name: "from", description: null, value: null }
Segment::Text(" ")
Segment::Placeholder { name: "to", description: null, value: null }
```

### 9.3 Placeholder editing behavior

- From outside a placeholder, Backspace/Delete can remove the entire placeholder span.
- If the placeholder is followed by one space, external deletion may remove the placeholder and the following space together.
- If the user edits inside the placeholder, it becomes expanded/editable content and no longer triggers whole-placeholder deletion.
- Commands with unresolved placeholders must not execute.

---

## 10. Completion and ghost suggestions

### 10.1 Completion sources

Completion should follow this order:

```text
Current token looks like a path:
  path completion

Current token is the first token:
  template candidates
  history command candidates
  executables from PATH

Current token is not the first token:
  structural template candidates
  structural history candidates
  template placeholder candidates
  history argument candidates
  file/path candidates
```

For non-first-token completion:

- Structural template candidates are shown before structural history candidates.
- Structural template candidates are ordered newest template first.
- When a structural template candidate exists, lower-priority generic placeholder, history-argument, and path fallbacks should not be mixed into that completion result set.
- Matching directory candidates are shown before lower-priority generic argument/history fallbacks so local navigation remains easy to trigger.
- Template placeholders can be matched by typing the placeholder name without braces; accepting the candidate inserts the raw `{placeholder}` form.
- History argument candidates are shown before file path candidates after structural and placeholder candidates.
- History argument candidates are ordered newest to oldest.
- File/path candidates must accurately represent the underlying filesystem entry.
- File/path matching should use the shell word value for lookup while preserving safe shell input on acceptance. Quoted and backslash-escaped typed paths must continue to match, and accepted replacements must escape unquoted shell metacharacters or close the user's current quote style. Leading `~/` should expand to HOME only when the `~` is unquoted and unescaped; quoted or escaped `~/` is a literal path component.
- File/path matching may resolve missing intermediate directory components component-wise. It should prefer exact existing directory components, then directory-prefix matches, then directory typo correction when `completion.fuzzy = true`; accepted replacements must still preserve shell-safe quoting/escaping for Bash, Zsh, and Fish.
- Hidden path entries should be ordered after visible entries for the same path query without hiding them entirely.
- Regular files must not be presented as directories.
- Symbolic links that resolve to directories should be presented as directory candidates.
- Directory candidates should use a trailing `/` so users can distinguish them from files.
- Directory scans may be cached briefly while the user types, but cache staleness must be bounded and filesystem changes must eventually be observed.

### 10.2 Template/history matching

For command completion:

- `completion.mode = "auto"` enables live completion hints while the user types.
- `completion.mode = "tab"` disables live hints while typing; the first non-empty `Tab` starts completion and displays hints, and the next `Tab` accepts the visible inline suggestion or first ranked displayed/cached candidate.
- `completion.mode = "off"` disables all Aish completion candidates, live completion UI, and non-empty `Tab` acceptance.
- `completion.enabled` and `completion.inline` remain legacy compatibility fields. When `completion.mode` is absent, `enabled=false` maps to `off`, `enabled=true` with `inline=true` maps to `auto`, and `enabled=true` with `inline=false` maps to `tab`. The legacy `inline=false` field selects tab-triggered completion; it does not disable the inline hint that can be shown after an explicit `Tab`. When `completion.mode` is present, it is authoritative and Aish normalizes the legacy fields to match it.
- `completion.fuzzy = false` disables typo-correction/fuzzy work while preserving fast prefix, path, template, and structural history completion.
- Template candidates are shown before history candidates.
- History candidates are ordered newest to oldest.
- Matching ignores spaces when configured.
- The below-prompt panel displays at most `completion.max_results` candidates.
- `completion.coalesce_ms` controls live UI refresh coalescing for layered background completion. The default is `50` ms; `0` disables coalescing and refreshes each changed tier immediately. First-token executable-only live hints may wait for this same window so lower-priority PATH matches do not flash before higher-priority history results arrive.
- `completion.display_delay_ms` controls auto-mode display debounce after the latest edit. Matching may run and update the pending candidate cache during this delay, but Aish must not redraw completion UI before the delay expires. The default is `120` ms; `0` disables this display debounce.
- Empty tokens and candidates with zero matching positions are not displayed.
- `completion.match_threshold_percent` is a structural word-position match rate, not character-level typo correction. For example, `git stx` matches `git status --short` at one of two typed positions, so the default `50` threshold can show it.
- Structural matches pass when the word-position match rate is greater than or equal to the configured threshold.
- Completion matching should lex shell-like words and perform quote removal only for comparison. Displayed and accepted history/template candidates must preserve their original raw words so quoted and escaped arguments remain shell-equivalent to the stored command. This lexical layer handles single quotes, double quotes, mixed quoted segments, and backslash-escaped characters, but it must not evaluate shell expansions, globbing, command substitution, or variables.
- Typo correction is separate from structural matching and uses `completion.typo_threshold_percent`; the default threshold is `80`. Accepting a typo candidate replaces the mistyped command with the corrected template/history command, or replaces the current path token with a corrected local directory path.
- Generic prefix matching must not treat partial character overlap such as `stx` versus `status` as a match. That belongs to the typo tier.
- A `# ` AI prompt must not trigger completion. A `#cmd` token may only show Aish private command candidates. Private command arguments and nested subcommands should use the same completion display and accept path as ordinary completion.

### 10.3 Live inline completion and Tab behavior

- Empty draft `Tab` switches modes.
- In `auto` mode, non-empty draft edits start a layered completion request for the current token and show the best available candidate as an inline ghost suggestion in dim text on the active prompt line without requiring `Tab`.
- Ordinary auto-mode refreshes may also render below-prompt candidate hints after the configured display debounce and coalescing windows. Frequent refreshes must not pollute terminal scrollback or flicker over SSH.
- In `tab` mode, ordinary typing clears any stale completion UI and does not start a live completion request. Pressing `Tab` starts the same layered request explicitly; background history and typo tiers may update the displayed hints for that request without accepting anything.
- Live completion must not scan regular history, stored templates, or PATH executables synchronously on every keypress. It sends a versioned background request using cheap cached snapshot references.
- The background worker may keep parsed history/template indexes and append-only primary-tier caches across requests. Correctness caches must retain the complete candidate set for their tier; the displayed top `completion.max_results` rows are a UI cache only and must not be used as the next request's matching input.
- Live completion is layered: cheap local path candidates can be found immediately, template/history/PATH executable results can refresh the UI when the worker finishes, and slower typo-correction results can refresh the UI after that when `completion.fuzzy = true`.
- Typo correction must not filter only from a previous request's matched candidates because edit-distance similarity is not monotonic as input grows. Typo tiers may use broad indexed pools, but each visible typo result must come from exact similarity scoring for the current input and config.
- Layered live completion refreshes may be coalesced until the final background tier arrives or `completion.coalesce_ms` elapses, whichever comes first, to avoid visible flicker while preserving non-blocking input.
- Auto-mode completion refreshes may also be delayed until `completion.display_delay_ms` after the most recent edit. This delay is only a display gate; it must not block input and must not prevent background matching from progressing.
- If the first-token immediate result set contains only PATH executable candidates and an async history tier is pending, Aish may defer drawing those executable candidates until the coalescing window resolves.
- Completion worker events carry the request id. Events for older input, different cursor positions, or stale request ids must be ignored.
- Empty non-first tokens after trailing whitespace must not run path fallback. They may show structural template candidates immediately and structural history candidates when the worker returns.
- Live inline completion also renders remaining candidates as below-prompt hints when they fit the configured display rules.
- If the user presses `Tab` and there are no candidates, Aish leaves the completion UI empty and redraws the current draft unchanged.
- The inline ghost suggestion is display-only. It must not modify the draft buffer, cursor position, history, or persisted draft until the user explicitly accepts it.
- In `auto` mode, pressing `Tab` with an inline ghost suggestion accepts the inline suggestion, not an arbitrary first row from the below-prompt panel. In normal typing flows this means the first `Tab` accepts the already-visible inline suggestion.
- Some valid ranked candidates, such as replacing `something` with the template placeholder `{something}`, cannot be rendered as an unambiguous suffix. In that case Aish may show the candidate in the live below-prompt panel; pressing `Tab` accepts the first ranked candidate rather than requiring the user to type braces manually.
- In `tab` mode, the first `Tab` displays candidates without editing the draft; a later `Tab` accepts the visible inline suggestion or first ranked displayed/cached candidate.
- `completion.tab_accept = "full"` accepts the complete selected suggestion.
- `completion.tab_accept = "word"` accepts only through the next whitespace boundary in the untyped suffix. If no whitespace boundary remains, it accepts the full suffix.
- `Right` at end-of-line may also accept the inline suggestion according to the configured accept amount. `Right` inside the line keeps ordinary cursor movement.
- The below-prompt candidate panel is informational. It displays at most `completion.max_results` candidates and must not decide how much `Tab` accepts.
- Candidate display must not permanently append text to the active prompt line.

### 10.4 Inline suggestion and panel rendering

Inline suggestions:

- The inline suggestion should be derived from the highest-ranked completion candidate after applying the same source ordering, filtering, and matching rules used for the below-prompt panel.
- The inline suggestion should show only the untyped portion of the candidate when that can be computed unambiguously.
- Inline suggestion color/style should be visually subdued, such as dim or light gray text, while preserving a usable fallback for terminals without color support.
- Redraw must restore the cursor to the user's real draft cursor, not to the end of the ghost text.

Below-prompt candidate panel:

- `completion.max_results` controls only the number of rows displayed in the below-prompt panel.
- When an inline suggestion is visible, the panel may update live while the user types and should skip the current inline candidate so the panel remains advisory.
- Candidate rows must fit within the current terminal width and must not wrap.
- Candidate rows should show the full command that would result from accepting the candidate.
- Candidate row command text should align with the prompt input column when there is room for the source label before that column.
- When a row cannot fit, Aish should elide from the left with ASCII `...`, preferring to keep whole whitespace-separated words visible.
- The panel may include source labels, but labels must not consume so much width that the useful untyped candidate portion disappears when there is still room to show it.


Config:

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

Command:

```text
#completion on|off
#completion mode auto|tab|off
#completion max 8
#completion coalesce-ms <0-1000>
#completion display-delay-ms <0-1000>
#completion inline on|off
#completion fuzzy on|off
#completion tab-accept full|word
#completion match-threshold <0-100>
#completion typo-threshold <0-100>
```

### 10.5 Ghost suggestions

Ghost suggestions are display-only.

- They do not modify the buffer unless accepted by the user.
- Accepting a suggestion should use a non-conflicting key such as `Right` at end-of-line or a configurable binding.
- AI must not silently overwrite current input.

---

## 11. Search, picker, and fzf integration

Aish should support opening selection tools while editing commands.

Default keybindings should avoid common readline conflicts by using a `Ctrl-X` prefix for advanced pickers.

Recommended defaults:

```text
Ctrl-R          history search
Ctrl-X Ctrl-F  file/path picker
Ctrl-X Ctrl-T  template picker
Ctrl-X Ctrl-B  git branch picker
Ctrl-X Ctrl-V  environment variable picker
Ctrl-X Ctrl-E  external editor
```

Notes:

- `Ctrl-P` should not be a default file-picker shortcut because it commonly means previous history in readline-style keymaps.
- Users may rebind shortcuts, including `Ctrl-P`, if desired.
- Picker insertion must shell-quote paths or values containing spaces or shell metacharacters.

Picker result action:

```text
InsertAtCursor
ReplaceCurrentToken
AppendAsArgument
ReplaceLine
```

File picker can be backed by `fzf` initially. A later internal picker can replace or augment `fzf`.

---

## 12. Keybinding policy

Aish should preserve common terminal and readline expectations in draft mode where possible, and should fully pass through keys in passthrough mode.

### 12.1 Must-preserve keys

```text
Ctrl-C        Smart Input: clear/cancel input; Running: interrupt foreground process; Passthrough: forward
Ctrl-D        Empty draft: exit Aish; non-empty draft: delete char; Passthrough: forward
Ctrl-L        clear screen/redraw
Ctrl-A        beginning of line
Ctrl-E        end of line
Ctrl-U        delete to beginning of line
Ctrl-K        delete to end of line
Ctrl-W        delete previous word/token
Alt-Backspace delete previous word/token
Alt-D         delete next word/token
Alt-Delete    delete next word/token
Alt-B         previous word/token
Alt-F         next word/token
Alt-Left      previous token
Alt-Right     next token
Ctrl-R        history search
Tab           completion or empty-input mode switch
Esc           cancel current menu/suggestion/search
Up/Down       mode-specific navigation
Left/Right    character navigation; Right may accept completion at end of draft
Backspace     delete previous character
Delete        delete next character
```

### 12.2 Advanced default shortcuts

```text
Ctrl-X Ctrl-E  external editor
Ctrl-X Ctrl-F  file/path picker
Ctrl-X Ctrl-T  template picker
Ctrl-X Ctrl-B  git branch picker
Ctrl-X Ctrl-V  environment variable picker
```

All keybindings are configurable through the `[keybindings]` config table. Each action is an array of one-key or two-key sequences, for example:

```toml
[keybindings]
history_search = ["Ctrl-P"]
file_picker = ["Ctrl-G Ctrl-F"]
external_editor = ["Ctrl-X Ctrl-E", "Ctrl-O"]
```

An empty array disables that action. Rebinding applies only to Aish-owned prompt modes; passthrough modes still forward keys to the foreground program.

Configured action names:

```text
clear_or_cancel
exit_or_delete
clear_screen
move_start
move_end
delete_to_start
delete_to_end
delete_previous_word
delete_next_word
move_previous_word
move_next_word
move_left
move_right_or_accept_completion
previous_item
next_item
delete_previous_char
delete_next_char
cancel
complete_or_cycle
submit
history_search
external_editor
file_picker
template_picker
git_branch_picker
env_var_picker
```

---

## 13. Storage layout

All files live under `~/.aish`.

Recommended layout:

```text
~/.aish/
  config.toml

  history/
    regular.jsonl
    ai.jsonl
    draft.jsonl
    notes.jsonl  # legacy compatibility

  templates/
    templates.jsonl

  secrets/
    key.json.gpg

  logs/
    events.jsonl

  cache/
    runtime/

  .gitignore
```

When encryption is on:

```text
history/regular.jsonl.gpg
history/ai.jsonl.gpg
history/draft.jsonl.gpg
history/notes.jsonl.gpg
templates/templates.jsonl.gpg
```

Plaintext files should not remain on disk by default after encryption is enabled, except transient temp files that are securely removed as best effort.

Managed storage directories should be private on Unix where supported. Config, JSONL, encrypted storage, sync lock, and temporary rewrite-script files should be written with private file permissions where supported.

JSONL is preferred for append-only human-diffable storage.

---

## 14. Configuration

Example `config.toml`:

```toml
[shell]
backend = "auto" # auto means $SHELL, fallback /bin/bash

[prompt]
draft = "{user}@{host} {cwd} > "
history = "{user}@{host} {cwd} $ "
ai = "{user}@{host} {cwd} % "

[ai]
base_url = "https://api.example.com/v1/chat/completions"
model = "model-name"
env_key = "OPENAI_API_KEY"

[context]
enabled = true
confirm = true
max_bytes = 65536

[completion]
mode = "auto"
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

[editor]
command = ["nvim"]
execute_after_save = false

[paste]
multiline = "editor"
confirm_execute = true
preview = true
preview_lines = 3
preview_bytes = 240

[draft]
persist = true
sync = false

[encryption]
enabled = false
key_fingerprint = ""
startup_unlock = "lazy" # "lazy" or "prompt"
recipient = ""

[sync]
enabled = false
remote = ""
schedule = ""
startup = false
exit = false
ai = true
history = true
templates = true
drafts = true
```

### 14.1 AI URL handling

Command:

```text
#base-url <url>
```

Storage uses `[ai].base_url`.

Normalization:

- `#base-url <url>` requires an `http://` or `https://` URL.
- If the value ends with `/chat/completions`, store it as-is after trimming trailing slashes.
- Otherwise, append `/chat/completions` and store the resulting endpoint.
- Manually edited config values are normalized at request time the same way.

### 14.2 API key handling

Commands:

```text
#env-key <ENV_NAME>
#key set
#key clear
#unlock
```

Priority:

1. Environment variable named by `#env-key`.
2. GPG-stored key from `#key set`. `#key set` encrypts the current environment key value for `[encryption].key_fingerprint`.
3. Common fallback environment variables if configured later.

---

## 15. Encryption

Commands:

```text
#encrypt on
#encrypt on <key-fingerprint|unique-email>
#encrypt rotate <key-fingerprint|unique-email>
#encrypt rewrite-history plan
#encrypt rewrite-history run <key-fingerprint|unique-email> --confirm-rewrite-history
#encrypt off
#unlock
```

Behavior:

- `[encryption].key_fingerprint` supplies the stable GPG encryption key. `[encryption].recipient` is a deprecated compatibility field.
- `[encryption].startup_unlock` is either `lazy` or `prompt`. `lazy` starts Aish immediately and requires `#unlock` only when old encrypted data needs a passphrase. `prompt` performs interactive GPG/pinentry unlock before the first Aish prompt opens.
- A full fingerprint is the preferred key selector. An email or user ID is accepted only when `gpg --list-keys --with-colons --fingerprint <selector>` resolves to exactly one public key.
- `#encrypt on <key>` may set and persist the resolved fingerprint before migration.
- `#encrypt unlock-mode lazy|prompt` updates the startup unlock policy.
- If encryption is already enabled and the resolved fingerprint differs from the stored fingerprint, Aish decrypts existing managed encrypted files with GPG and re-encrypts them for the new fingerprint.
- `#encrypt rotate <key>` explicitly performs the same current-storage key rotation.
- Encrypt regular history, AI history, draft history, legacy note files, and templates.
- Encrypt template payload metadata as well as bodies. User-facing template names are not part of the product model; template IDs are stable content-hash handles used for exact operations, and no plaintext template search/list index should be persisted when encryption is enabled.
- Secrets are always stored encrypted.
- When encryption is enabled, do not store plaintext search indexes.
- Live completion must not run GPG on each keypress. Encrypted template data should be loaded into an in-memory snapshot during unlock/startup and refreshed only by explicit template mutations or reload flows.
- Normal encrypted JSONL appends should be serialized through a background writer. Foreground command execution updates in-memory state and enqueues persistence work; command output and prompt redraw must not wait for GPG encryption.
- Encrypted JSONL appends update one complete encrypted JSONL payload for each managed file, using the writer's decrypted byte cache when available. They do not concatenate multiple independent GPG messages into one file.
- The background encrypted writer must preserve write order per Aish process and fail closed after a write failure until the error is surfaced and pending writes are flushed or the process is restarted. Whole-file rewrites such as key rotation, decryption, and template removal should still use atomic encrypted file replacement.
- Operations that need durable storage or rewrite storage globally must flush pending encrypted writes first. This includes exit, `#history`, `#encrypt off`, key rotation, and confirmed history rewrite. `#sync now` must enqueue the encrypted-writer flush inside the background sync worker so the prompt does not wait for GPG.
- Encrypted-write completion and failure events are frontend background events. Draining those events should refresh live completion and redraw the prompt when needed.
- Direct GPG decrypt operations that may need pinentry should enter `UnlockPassthrough`, clear stale live completion state, yield terminal control, set `GPG_TTY` when possible, and restore raw mode and the previous Aish mode after completion or failure.
- In `startup_unlock = "lazy"` mode, startup decrypt should first use a noninteractive background GPG attempt (`--batch --pinentry-mode error`) so startup never blocks on passphrase entry or lets pinentry fight the raw-mode UI.
- If lazy startup unlock succeeds because `gpg-agent` can decrypt without prompting, Aish loads history/templates, starts or refreshes the encrypted writer with decrypted caches, refreshes completion, and redraws.
- If lazy startup unlock needs a passphrase, Aish keeps shell input usable, keeps same-session writes in memory when old encrypted data for that file is still locked, shows `history is still unlocking...` in history/AI modes when needed, and exposes `#unlock`.
- In `startup_unlock = "prompt"` mode, startup should run the interactive GPG/pinentry unlock path before entering raw-mode Aish UI. If unlock fails, startup fails rather than opening a prompt with locked storage.
- `#unlock` runs the interactive GPG/pinentry unlock path, merges loaded encrypted history/templates with any same-session in-memory locked entries, and refreshes the encrypted writer.
- `#encrypt rewrite-history plan` prints the risk and exact confirmed command for rewriting Git history.
- `#encrypt rewrite-history run <key> --confirm-rewrite-history` is destructive by design: it requires a clean worktree, creates a backup branch, rewrites the current branch's managed storage blobs by encrypting plaintext blobs and re-encrypting old-key blobs for the target fingerprint, and never pushes automatically.

Safety message when enabling encryption:

```text
Encryption is now enabled for future writes.
Aish will sync encrypted files from now on.
Git history may still contain plaintext data or encrypted data written for an older key.
Aish will not rewrite git history automatically; history rewrite requires an explicit backup and old-key re-encryption flow.
```

Aish should use atomic writes for encrypted files:

```text
write plaintext temp in private location
run gpg encrypt to .gpg.tmp
fsync where practical
rename .gpg.tmp -> .gpg
remove plaintext temp
```

---

## 16. Git sync

Commands:

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
#sync quiet on|off
#sync ai on|off
#sync history on|off
#sync templates on|off
#sync drafts on|off
```

Policy:

- Aish can initialize and manage a git repository in `~/.aish`.
- Sync is conservative.
- Aish uses a lock file to prevent concurrent sync.
- Aish must clear sync lock files left by dead Aish processes before refusing a new sync.
- Each Git sync step must have a finite timeout. On timeout, Aish must kill the Git child process, report the timeout, and release the sync lock.
- Manual sync commands, startup sync, and periodic sync must run in the background so the prompt remains usable. Exit sync is the blocking durability boundary.
- Background sync output should be compact: record-count differences/summaries, warnings, conflicts, final success, or failure. Routine per-step Git output should stay out of the interactive terminal.
- `sync.quiet` / `#sync quiet on` must suppress routine background sync start/success notices while keeping failures visible.
- Sync includes AI history, shell history, legacy note files, templates, and drafts by default.
- Aish stages managed enabled files automatically before every sync commit.
- When encryption is enabled, Aish must decrypt every enabled managed `*.jsonl.gpg` file before staging, committing, or pushing. If the current machine cannot decrypt the data, sync stops and reports the managed path plus key-resolution guidance.
- Aish warns, without staging, when existing Aish-managed files are excluded because their sync category is disabled.
- Aish writes `README.md` into the sync data repository so the remote is identifiable as Aish-managed data when the file is absent or already Aish-managed.
- Aish writes `.aish-sync.toml` into the sync data repository as non-secret sync metadata. It records the private sync content categories and encryption key metadata. `config.toml` stays local and must not be committed.
- Aish must inspect remote `.aish-sync.toml` through the isolated runtime remote cache, not by trusting the active `~/.aish/.aish-sync.toml` file. The active file is a generated sync output and must not be allowed to masquerade as the remote repository's current metadata.
- If repository content-category metadata disagrees with local settings, Aish warns, adopts the repository settings, persists them locally, and uses those settings for the current sync. Local existing files excluded by repository settings are not staged; Aish warns about them without deleting them.
- When encryption is enabled, `.aish-sync.toml` must contain exactly one current full 40-hex GPG key fingerprint. Local encrypted sync must use a full fingerprint, not an email/user selector.
- If local encryption state or fingerprint disagrees with the remote repository metadata, Aish must stop before pushing and tell the user how to resolve the key choice.
- If Git identity is not configured, Aish sets local-only `user.name` and `user.email` values inside the sync repository before committing.
- Empty bare or hosted Git remotes are valid first-sync targets. If the remote has no branch, Aish must skip the cached merge and let push create the branch/upstream.
- Aish must not rely on local branch tracking for remote integration. When a remote branch exists, Aish must select it explicitly, fetch it into the isolated runtime cache, and merge that cached ref.
- When the remote has a default branch, Aish should align the local sync branch to that branch before committing so one sync remote does not silently split across `master`, `main`, or another branch.
- Longer term, remote payload merging should happen in an isolated staging workspace and only publish validated results back into the active Aish home after conflicts, key metadata, and content options are resolved.
- If an existing non-Git Aish home is connected to an already-populated sync remote, Aish merges the remote default branch with `--allow-unrelated-histories` during the first sync.
- If an existing local sync repository is connected to a populated remote with separate history, Aish reports the unrelated-history case and retries the cached merge with `--allow-unrelated-histories`.
- Local bare Git repositories are valid sync remotes.
- Plaintext Aish JSONL files use Git's union merge driver so independent appends keep both sides.
- After a successful remote merge, Aish reports managed JSONL record-count changes. If an enabled managed record count decreases during the merge, sync must restore the JSONL union before pushing so neither side's records are deleted. If union restoration fails, sync must stop before pushing.
- Aish can auto-resolve remaining Aish-managed text and JSONL conflicts with `#sync resolve-union`.
- Aish must mark encrypted `*.jsonl.gpg` files as binary so Git does not text-union merge ciphertext.
- Aish can auto-resolve enabled encrypted Aish JSONL conflicts by reading Git's ours/theirs stages, decrypting both complete payloads, unioning plaintext JSONL records, re-encrypting the merged file, and staging it. If either side cannot decrypt, sync must stop before pushing.
- Aish does not run `git rm --cached` automatically.
- Sync does not rewrite git history. Encrypted-storage history rewrite is available only through `#encrypt rewrite-history run <key> --confirm-rewrite-history`.
- If a category is disabled for sync, Aish updates future `.gitignore` behavior and warns if files may already be tracked.
- Key rotation requires decrypting existing managed local and repository data with an available private key, choosing one target full fingerprint, and re-encrypting with `#encrypt rotate <key>` before syncing again. If the current machine cannot decrypt the data, it cannot resolve the key conflict safely.

Command boundaries:

- `#set-remote <git-url>` persists the private sync remote only and must not run Git.
- `#sync` reports sync/encryption status only and must not run Git.
- `#sync now` is the only manual sync run command. It must queue a background sync that verifies enabled managed data, stage enabled managed paths, commit only when staged content changed, merge remote updates, verify/count merged data, and push.
- `#sync resolve-union` queues background recovery and is valid only during an interrupted merge with Aish-managed conflicts. It must union managed plaintext files directly, union encrypted `*.jsonl.gpg` by decrypting ours/theirs and re-encrypting the merged plaintext, and refuse unmanaged conflicts.
- `#sync continue` queues background continuation and is valid only during an interrupted merge. It may auto-resolve remaining enabled encrypted Aish JSONL conflicts before committing; unmanaged conflicts still require manual resolution and staging.
- `#sync abort` queues background abort and is valid only during an interrupted merge or rebase.
- `#sync <schedule>`, `#sync off`, trigger/quiet toggles, and content-category toggles only persist sync settings.

Template sharing:

- Keep the private sync remote separate from public/shared template remotes.
- Private sync is dynamic personal state; template sharing is static published template content that is fetched, analyzed, and explicitly imported.
- Named template remotes are configured with `#template remote add <name> <git-url>`.
- Template remote caches live under `cache/template-remotes/<name>/repo`.
- Template sharing commands must never stage private history, AI prompts, drafts, legacy notes, config, logs, cache, or secrets.
- Publishing templates writes only `README.md`, `.aish-template-remote.toml`, and either `templates/templates.jsonl` or `templates/templates.jsonl.gpg`.
- Template remote README and metadata remain readable even when the template payload is encrypted.
- `#template publish <name>` publishes plaintext templates. `#template publish <name> --encrypt <key>` resolves the GPG recipient and encrypts the template payload for that recipient.
- Fetching, analyzing, and importing an encrypted template payload requires the matching local private key. This sharing encryption is independent from private sync's single repository fingerprint rule.
- Publishing with no local templates still initializes the remote with README, metadata, and an empty template payload so the owner can edit it later.
- Publishing merges by stable template ID/body hash: existing remote templates are kept and local templates are added, so publishing from one machine does not prune templates published by another machine.
- Publishing and fetching must minimize remote round trips by taking one remote-ref snapshot, fetching the selected branch into `cache/template-remotes/<name>/repo`, and then reading/writing the local cache. If a publish push is rejected because the remote changed, Aish may refresh that snapshot once, merge by template ID again, and retry.
- Fetching templates updates only the local review cache. Local templates change only after `#template import`.
- `#template analyze <name> [query]` compares fetched templates with local templates and marks each fetched template as `new` or `present`.
- `#template import <name> <id|all>` imports selected fetched templates into the local template store.
- Import deduplicates by stable template ID/body hash, reports already-present templates, and avoids overwriting local templates silently.
- Template import is usable without enabling private sync.
- If a configured template remote appears to be a private Aish sync repository, Aish refuses to use it and reports that a separate template remote is required.

Template sharing command boundaries:

- `#template remote add|list|rm` manages named template remote config only. If a name is removed or repointed to a different URL, Aish clears that name's local review cache to prevent stale imports from the old URL.
- `#template publish <name>` is the only template sharing command that writes to a template remote. Plaintext is the default; `--encrypt <key>` encrypts only the template payload for that recipient.
- `#template fetch <name>` updates only the local review cache.
- `#template analyze <name> [query]` reads the review cache and local template store, reports import status, must not contact the remote, and must not modify local templates.
- `#template import <name> <id|all>` is the only template sharing command that appends fetched templates into the local template store.

Commit messages:

```text
[auto] sync 2026-05-11T10:00:00-07:00
[man] sync 2026-05-11T10:00:00-07:00
```

Sync triggers:

1. `#sync now` queues the manual sync flow immediately and returns control to the prompt.
2. `#sync <schedule>` persists a conservative periodic interval that is checked while Aish is running. No scheduler files are created.
3. `#sync startup on|off` controls whether the manual sync flow is queued once every startup, independent of the periodic due check.
4. `#sync exit on|off` controls whether the manual sync flow runs during the exit durability boundary.
5. Multiple triggers may be enabled. A single startup invocation should not run duplicate syncs for the same trigger path.
6. Every automatic trigger must acquire the same sync lock as manual `#sync now`.
7. Every automatic trigger must use the same conservative sync plan as manual `#sync now`.
8. Log success/failure.

Aish must not create scheduler files. Periodic sync means "check whether the saved interval is due during normal frontend ticks and queue a background sync when due." Supported schedule forms are intentionally conservative: `@hourly`, `@daily`, `*/N * * * *`, `0 */N * * *`, `0 0 * * *`, and `0 0 */N * *`. Unsupported schedules are logged and do not run git.

Recommended conservative sync:

```text
git add managed files
git commit -m "[auto] sync <time>"
git fetch selected branch into isolated runtime cache
git fetch local runtime cache into active sync repository
git merge --no-edit FETCH_HEAD
verify/count merged managed records
git push
```

If conflict occurs:

- Leave the merge/rebase state intact.
- Log error.
- Show user a short warning with options.
- `#sync resolve-union` keeps both sides for Aish-managed conflicts, decrypting and re-encrypting managed `*.jsonl.gpg` files when needed, stages them, commits, and pushes.
- `#sync continue` continues after manual edits and `git add`; it also tries to auto-resolve remaining enabled encrypted Aish JSONL conflicts before committing.
- `#sync abort` cancels the interrupted merge or rebase.

---

## 17. Logging

Aish should maintain a user-facing event log.

File:

```text
~/.aish/logs/events.jsonl
```

Example:

```json
{"t":1778526001,"level":"info","msg":"AI generated 2 commands"}
{"t":1778526020,"level":"warn","msg":"Context command requires confirmation"}
{"t":1778526100,"level":"error","msg":"Sync failed: remote rejected push"}
```

Command:

```text
#log <count>
```

Behavior:

- Logs are not synchronized.
- Logs are ignored by git.
- Keep at most 1000 events by default.

---

## 18. Private command list

Initial command set:

```text
#help [commands|keys|ai|paste|completion|templates|sync|encryption|config]
#status
#config
#doctor

#prompt
#prompt draft "{basename} > "
#prompt history "{basename} $ "
#prompt ai "{basename} % "
#prompt reset

#model <name>
#base-url <url>
#env-key <ENV_NAME>
#key set
#key clear
#unlock

#context on|off
#context <bytes>
#context confirm on|off

#paste
#paste multiline editor|execute|discard
#paste confirm on|off
#paste preview on|off
#paste preview-lines <1-20>
#paste preview-bytes <1-4096>

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

#history list [>|>> <path> | | <command>]
#history search <query>
#history <count>
#ai list [>|>> <path> | | <command>]
#ai search <query>
#draft list [>|>> <path> | | <command>]
#draft search <query>
#log <count>

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

#editor

#encrypt on [key-fingerprint|unique-email]
#encrypt rotate <key-fingerprint|unique-email>
#encrypt unlock-mode lazy|prompt
#encrypt rewrite-history plan
#encrypt rewrite-history run <key-fingerprint|unique-email> --confirm-rewrite-history
#encrypt off

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

`#help` must print grouped in-terminal help for private commands, keybindings,
AI prompt forms, paste review, completion, templates, sync, encryption, and
configuration/diagnostics. `#help <topic>` prints only that topic. Unknown help
topics must show a usage line and must not reach the backend shell.

`#doctor` should check:

```text
backend shell
PTY availability
shell integration status
gpg
pinentry behavior if detectable
git
fzf
AI URL
API key source
model
encryption status
sync status
config parse validity
storage permissions
```

---

## 19. Shell integration

Aish needs to detect when backend shell is ready and when commands finish.

Implementation phases:

### Phase 1: prompt marker

- Start backend shell with a controlled prompt marker.
- Hide marker from user.
- Detect prompt readiness from PTY output.

### Phase 2: shell hooks

Implement shell-specific integration:

- Bash: `PROMPT_COMMAND` and related hooks.
- Zsh: `precmd` and `preexec`.
- Fish: prompt/event functions.

Integration should report:

```text
prompt ready
command started
command finished
exit status
current working directory
```

Shell history responsibility:

- Aish-owned functional shell injections such as readiness markers, status markers, or similar internal integration commands must not pollute backend shell history.
- Commands submitted through Aish must be stored in Aish-managed history only, not in the backend shell's native Bash, Zsh, or Fish history.
- Aish should prefer prevention over deletion by disabling or isolating native backend history after user rc loading.
- If a backend requires after-the-fact cleanup, removal must be scoped to the current backend session and must not delete preexisting user shell history.
- If exact attribution is not possible, Aish must leave preexisting shell history unchanged.

---

## 20. Security and safety

Aish should be conservative by default.

Required safety rules:

- AI output is never executed automatically.
- AI command execution is one item at a time.
- Commands with unresolved placeholders are blocked.
- `<` context command execution can be disabled.
- `<` context command confirmation is enabled by default.
- Dangerous context commands should force confirmation even if confirmation is disabled.
- Multi-line paste does not enter draft by default.
- Multi-line paste execution requires warning by default.
- Encrypted mode does not persist plaintext search index.
- Git sync only auto-resolves Aish-managed append-only JSONL conflicts where both sides can be preserved.
- Logs must not include API keys or decrypted secrets.

Dangerous command patterns for extra confirmation:

```text
sudo
rm -rf
mkfs
dd
curl ... | sh
wget ... | sh
chmod -R 777
chown -R
git reset --hard
docker system prune
kubectl delete
cat .env
cat ~/.ssh
cat *.pem
```

---

## 21. Rust implementation notes

Suggested libraries, subject to project evaluation:

```text
PTY: Unix libc openpty/setsid/TIOCSCTTY backend with a dedicated control fd
Terminal input/rendering: crossterm
Background work: standard threads and channels for completion, startup unlock, and encrypted writes
Serialization: serde, serde_json, toml
HTTP: reqwest blocking client with rustls
Storage/search: JSONL source files and in-memory snapshots/indexes
Fuzzy search: internal matcher plus external fzf picker integration
GPG: spawn gpg CLI
Git: spawn git CLI
Sync schedule parsing: internal conservative schedule subset
```

Aish should keep shell parsing minimal. It only needs enough lexing for current-token detection, quoting insertion, placeholder spans, and optional best-effort history splitting. The backend shell remains authoritative for execution semantics.

---

## 22. Non-goals

Aish v1 does not aim to:

- Implement a POSIX shell.
- Fully parse Bash/Zsh/Fish syntax.
- Replace user shell configuration.
- Automatically rewrite user commands.
- Automatically execute multiple AI-generated commands.
- Auto-resolve unmanaged git sync conflicts.
- Guarantee secure deletion on all filesystems.
- Persist plaintext indexes when encryption is enabled.

---

## 23. MVP definition

A usable MVP includes:

1. PTY backend using `$SHELL`.
2. Draft/history/AI modes with default `>`, `$`, `%` prompts.
3. Ordinary command execution through backend shell.
4. Line-leading `#` dispatch for private commands, AI prompts, and context prompt syntax.
5. Read-only history and AI mode; edits copy to draft.
6. AI JSON output parsing into AI sessions/items.
7. AI `Enter` execution rule: success advances, failure stays, last success returns to draft.
8. JSONL regular/draft/AI storage.
9. `Ctrl-X Ctrl-E` editor integration.
10. Multi-line paste editor-review behavior.
11. Basic template creation with `#mt`.
12. Basic completion: paths, templates, history, PATH executables.
13. Config for model, AI URL, env key, context bytes, completion result count.
14. Event log with `#log`.
15. `#doctor` with basic checks.
