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
8. **Editor-submitted content is raw shell input.** Aish does not parse line-leading `#` inside external editor content.
9. **Encryption prioritizes confidentiality over speed.** When encrypted, Aish does not persist plaintext search indexes.
10. **Git sync is conservative.** Aish never auto-resolves conflicts and never rewrites history or runs `git rm --cached` automatically.
11. **End-to-end behavior must be tested as users experience it.** Every user-visible feature needs expect-driven coverage in addition to Rust unit/integration tests, including rendering, redraw boundaries, PTY output framing, shell continuation, keybindings, mode transitions, error paths, and regression cases.

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
- Executed commands are appended to regular history after execution.
- Up/down navigation may show both draft entries and regular history entries.
- If draft content is empty, `Tab` switches modes.
- If draft content is non-empty, `Tab` opens completion.

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

Used for interactive programs launched by the backend shell, such as `vim`, `nvim`, `ssh`, `top`, `less`, `fzf`, `python`, `node`, `psql`, or similar.

- Keyboard input is forwarded to the PTY.
- Output is displayed as-is.
- Aish keybindings are mostly disabled.
- Return to normal mode occurs when the backend shell prompt is detected again.

#### ExternalEditor mode

Triggered by `Ctrl-X Ctrl-E` or configured editor actions.

- Opens the configured editor with current draft content.
- If invoked from history or AI mode, selected read-only item is copied to draft first.
- On save and exit, editor content replaces the current draft buffer.
- By default, content is **not executed automatically**. The user presses `Enter` to execute.
- Optional config can enable execute-after-save for users who want that behavior.
- Content from this mode is raw shell input; Aish does not parse line-leading `#` inside it.

#### PasteReviewEditor mode

Legacy name for the multi-line paste review state. In the current design, this is represented as an opaque editor draft rather than a separate inline editor.

- Multi-line paste with `paste.multiline = "editor"` becomes an opaque editor draft.
- The main prompt shows only the editor draft summary.
- `Ctrl-X Ctrl-E` can open the external editor for that content.
- `Enter` submits the raw editor draft to the backend shell.
- `paste.multiline = "execute"` with `confirm_execute = true` also uses the editor draft as the confirmation step.

#### Picker modes

Temporary UI modes for selecting files, history, templates, git branches, environment variables, or other insertable values.

- Picker result is inserted into draft at cursor, replaces current token, appends as an argument, or replaces the whole line depending on picker action.
- Picker shortcuts should avoid conflicting with common readline keys by default.

#### UnlockPassthrough mode

Used when GPG or pinentry needs interactive terminal control.

- Aish temporarily pauses normal UI.
- The GPG/pinentry interaction receives the terminal.
- After completion, Aish restores raw mode and redraws.

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
# TODO: ...              Aish note/comment
# NOTE: ...              Aish note/comment
# FIXME: ...             Aish note/comment
```

This reservation applies only to direct Aish prompt input. It does **not** apply to raw content submitted from ExternalEditor mode.

### 5.3 Aish notes/comments

Aish should recognize common note/comment prefixes and swallow them instead of sending them to the shell or AI:

```text
# TODO: deploy later
#TODO: deploy later
# NOTE: remember this
# FIXME: clean this up
# HACK: temporary workaround
# XXX: revisit this
```

Recommended behavior:

- Store as a note event or note history item.
- Do not execute.
- Do not call AI.
- Make notes searchable via history search, but do not treat them as executable commands.

### 5.4 Private command syntax

Private commands use `#<name>` with no required space after `#`:

```text
#model gpt-4.1
#base-url https://example.com/v1
#env-key OPENAI_API_KEY
#key set
#encrypt on
#history 20000
#context 65536
#mt rename mv {from} {to}
```

Unknown private commands should never be sent to the shell. Aish should show an error and possible suggestions.

### 5.5 AI prompt syntax

AI prompts use `# ` followed by arbitrary prompt text:

```text
# how to set remote for my git repo?
# generate a command to find files larger than 100MB
```

AI-generated results enter `%` AI mode.

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
- A trailing odd backslash, such as `echo foo \`, is treated as a line continuation because interactive shells continue those inputs even if non-interactive syntax checks accept a synthetic final newline.
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
#mt deploy \
#mt   rsync -avz {from} {user}@{host}:{to}
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

Editor content is raw shell input. Aish does not parse line-leading `#` inside it, does not escape it as `##`, and does not rewrite multi-line content or backslash continuations. The backend shell interprets the content exactly as submitted.

`execute_after_save = false` means editor exit only writes back an editor draft. It does not execute. If `execute_after_save = true`, Aish executes only after a successful editor exit status and preserves the same raw editor-draft semantics.

### 6.3 Multi-line paste

Aish should enable bracketed paste mode.

Single-line paste:

- Insert at cursor in draft mode.
- If in read-only history or AI mode, copy selected item to draft first, then paste.

Multi-line paste:

- Must not execute by default.
- Default behavior creates an opaque editor draft using the pasted content.
- Alternative behavior can be direct execution after warning.

Config:

```toml
[paste]
multiline = "editor"       # editor | execute | discard
confirm_execute = true
```

`multiline = "editor"`:

1. Convert pasted content to an editor draft.
2. Show only the editor draft summary in the main prompt.
3. `Ctrl-X Ctrl-E` can reopen the content in the external editor.
4. `Enter` submits the raw editor draft to the backend shell.
5. Do not auto-execute on paste.

`multiline = "execute"`:

1. If `confirm_execute = true`, convert pasted content to an editor draft and wait for `Enter`.
2. If `confirm_execute = false`, submit pasted content immediately as raw editor-draft content.
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
- Store comment-only lines as notes if note storage is enabled, not as commands.

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

Optional notes may be stored separately or inside draft history as non-executable note items.

### 7.2 Read-only source rule

Regular history and AI history are read-only browsing sources.

- Editing a history item copies it to draft.
- Editing an AI item copies it to draft.
- Moving the cursor does not count as editing.
- Executing a history or AI item appends a new regular history entry.

### 7.3 Draft behavior

Draft mode can show draft entries and regular history during up/down navigation.

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

### 7.5 History limit

Command:

```text
#history <count>
```

Meaning:

- Limits total stored command items across regular history and AI command items.
- AI sessions count by item count, not by session count.
- If all items in an AI session are trimmed, the session can be removed.

### 7.6 Failed commands

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
#mt <name> <template>
```

Examples:

```text
#mt rename mv {from} {to}
#mt git-user git config --global user.name "{name}" && git config --global user.email "{email}"
```

Multi-line:

```text
#mt deploy \
#mt   rsync -avz {from} {user}@{host}:{to}
```

AI may suggest template items, but Aish never auto-saves them.

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
  history argument candidates
  file/path candidates
  template placeholder candidates
```

For non-first-token completion:

- History argument candidates are shown before file/path candidates.
- History argument candidates are ordered newest to oldest.
- File/path candidates must accurately represent the underlying filesystem entry.
- Regular files must not be presented as directories.
- Directory candidates should use a trailing `/` so users can distinguish them from files.

### 10.2 Template/history matching

For command completion:

- Template candidates are shown before history candidates.
- History candidates are ordered newest to oldest.
- Matching ignores spaces when configured.
- Display at most `completion.max_results` candidates.

### 10.3 Tab behavior

- Empty draft `Tab` switches modes.
- Non-empty draft `Tab` computes completion candidates for the current token.
- If there are no candidates, Aish displays `no completions` below the prompt and redraws the current draft.
- If there is exactly one candidate, Aish accepts it immediately.
- If there are multiple candidates, Aish displays at most `completion.max_results` candidates below the prompt and redraws the current draft.
- Candidate display must not append text to the active prompt line.


Config:

```toml
[completion]
max_results = 5
ignore_spaces = true
template_first = true
```

Command:

```text
#completion max 8
```

### 10.4 Ghost suggestions

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
Alt-B         previous word/token
Alt-F         next word/token
Alt-Left      previous token
Alt-Right     next token
Ctrl-R        history search
Tab           completion or empty-input mode switch
Esc           cancel current menu/suggestion/search
Up/Down       mode-specific navigation
```

### 12.2 Advanced default shortcuts

```text
Ctrl-X Ctrl-E  external editor
Ctrl-X Ctrl-F  file/path picker
Ctrl-X Ctrl-T  template picker
Ctrl-X Ctrl-B  git branch picker
Ctrl-X Ctrl-V  environment variable picker
```

All keybindings must be configurable.

---

## 13. Storage layout

All files live under `~/.aish`.

Recommended layout:

```text
~/.aish/
  config.toml
  state.json

  history/
    regular.jsonl
    ai.jsonl
    draft.jsonl
    notes.jsonl

  templates/
    templates.jsonl

  secrets/
    key.json.gpg

  logs/
    events.jsonl

  cache/
    runtime/
    completion.sqlite

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
chat_completions_url = "https://api.example.com/v1/chat/completions"
model = "model-name"
env_key = "OPENAI_API_KEY"

[context]
enabled = true
confirm = true
max_bytes = 65536

[completion]
max_results = 5
ignore_spaces = true
template_first = true

[editor]
command = ["nvim"]
execute_after_save = false

[paste]
multiline = "editor"
confirm_execute = true

[draft]
persist = true
sync = false

[encryption]
enabled = false
recipient = ""

[sync]
enabled = false
remote = ""
cron = ""
sync_ai = true
sync_history = true
sync_template = true
sync_draft = false
```

### 14.1 AI URL handling

Command:

```text
#base-url <url>
```

Storage should keep the actual final request URL as `chat_completions_url`.

Normalization:

- If input ends with `/chat/completions`, save it as-is.
- If input ends with `/v1` or `/v1/`, append `/chat/completions` and save the full URL.
- Otherwise, save the input as the full request URL.

### 14.2 API key handling

Commands:

```text
#env-key <ENV_NAME>
#key set
#key clear
```

Priority:

1. Environment variable named by `#env-key`.
2. GPG-stored key from `#key set`.
3. Common fallback environment variables if configured later.

---

## 15. Encryption

Commands:

```text
#encrypt on
#encrypt off
```

Behavior:

- Encrypt regular history, AI history, draft history, notes, and templates.
- Secrets are always stored encrypted.
- When encryption is enabled, do not store plaintext search indexes.
- Decrypt asynchronously on startup so the shell is usable before history/template unlock completes.
- While unlock is pending, completion/history features can show `history is still unlocking...`.

Safety message when enabling encryption:

```text
Encryption is now enabled for future writes.
Aish will sync encrypted files from now on.
If plaintext files were already tracked by git, they may still exist in git history.
Aish will not rewrite git history or remove tracked files automatically.
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
#push
#sync <cron-expression>
#sync off
#sync ai on|off
#sync history on|off
#sync template on|off
#sync draft on|off
```

Policy:

- Aish can initialize and manage a git repository in `~/.aish`.
- Sync is conservative.
- Aish uses a lock file to prevent concurrent sync.
- Aish does not auto-resolve conflicts.
- Aish does not run `git rm --cached` automatically.
- Aish does not rewrite git history.
- If a category is disabled for sync, Aish updates future `.gitignore` behavior and warns if files may already be tracked.

Commit messages:

```text
[auto] sync 2026-05-11T10:00:00-07:00
[man] sync 2026-05-11T10:00:00-07:00
```

Startup sync check:

1. Read cron expression.
2. Compute last scheduled sync time.
3. Check whether a successful sync already covered that time.
4. Check whether managed files changed.
5. Acquire lock.
6. Run conservative sync.
7. Log success/failure.

Recommended conservative sync:

```text
git pull --rebase
git add managed files
git commit -m "[auto] sync <time>"
git push
```

If conflict occurs:

- Abort automatic sync.
- Log error.
- Show user a short warning.
- Require manual resolution.

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
#help
#status
#config
#doctor

#model <name>
#base-url <url>
#env-key <ENV_NAME>
#key set
#key clear

#context on|off
#context <bytes>
#context confirm on|off

#completion max <count>

#history <count>
#log <count>

#mt <name> <template>
#template list
#template rm <name>

#editor <command...>

#encrypt on|off

#set-remote <git-url>
#push
#sync <cron-expression>
#sync off
#sync ai on|off
#sync history on|off
#sync template on|off
#sync draft on|off
```

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

- Aish-owned functional shell injections such as readiness markers, status markers, or similar internal integration commands must not pollute backend shell history when Aish can prevent that safely.
- For Bash and Zsh, Aish should prefer prevention over deletion by using shell-supported history-ignore behavior for Aish-injected internal commands.
- If Aish ever needs to remove an entry after the fact, removal must be exact and conservative: only entries Aish can confidently attribute to its own injected functional commands may be touched.
- If exact attribution is not possible, Aish must leave shell history unchanged.

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
- Git sync does not auto-resolve conflicts.
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
PTY: portable-pty or nix/libc lower-level PTY
Terminal input/rendering: crossterm, ratatui if needed
Async runtime: tokio
Serialization: serde, serde_json, toml
HTTP: reqwest
JSON schema validation: serde-based validation or jsonschema crate
Storage/search: JSONL source files, optional in-memory indexes
Fuzzy search: nucleo, skim matcher, or external fzf integration
GPG: spawn gpg CLI initially
Git: spawn git CLI initially
Cron parsing: cron parser crate or internal minimal schedule parser
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
- Auto-resolve git sync conflicts.
- Guarantee secure deletion on all filesystems.
- Persist plaintext indexes when encryption is enabled.

---

## 23. MVP definition

A usable MVP includes:

1. PTY backend using `$SHELL`.
2. Draft/history/AI modes with default `>`, `$`, `%` prompts.
3. Ordinary command execution through backend shell.
4. Line-leading `#` dispatch for private commands, AI prompts, notes, and context prompt syntax.
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
