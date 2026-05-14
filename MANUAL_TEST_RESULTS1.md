## Aish Human Manual Test Report

### Basic Information

| Item | Value |
|---|---|
| Aish commit | `06443760ea39d0a997d0c765e59a27f7ca2bbd9e` |
| OS | macOS / Darwin 25.4.0 arm64 |
| Kernel | `Darwin Kernel Version 25.4.0: Thu Mar 19 19:32:59 PDT 2026; root:xnu-12377.101.15~1/RELEASE_ARM64_T8122` |
| Terminal | Ghostty 1.3.1 |
| Ghostty channel | stable |
| Ghostty renderer | Metal |
| Backend shell | `/bin/zsh` |
| Initial isolated `AISH_HOME` | `/tmp/aish-manual-1778766906/home` |
| Recovery isolated `AISH_HOME` | `/tmp/aish-manual-recovery-*` |
| Disposable default `HOME` | `/tmp/aish-default-home-*` |
| Editor tested | `vim` |
| `fzf` | `/opt/homebrew/bin/fzf` |

### Ghostty Version Detail

```text
Ghostty 1.3.1

Version
  - version: 1.3.1
  - channel: stable
Build Config
  - Zig version   : 0.15.2
  - build mode    : .ReleaseFast
  - app runtime   : .none
  - font engine   : .coretext
  - renderer      : renderer.generic.Renderer(renderer.Metal)
  - libxev        : kqueue
```

## Summary

| Area | Result |
|---|---|
| Startup in isolated `AISH_HOME` | Passed |
| Basic command execution | Passed |
| H-001 inline completion visual rendering | Passed visually, UX issues found |
| H-002 narrow terminal long input / completion | Failed |
| H-003 `Tab` full/word accept | Partially passed, completion model issue found |
| H-004 clipboard paste | Safety mostly passed, UX and newline risk issues found |
| H-005 real `vim` editor flow | Safe behavior passed, rendering issues found |
| H-006 editor failure handling | Passed |
| H-007 real `fzf` pickers | Mostly passed, cancellation/env issues found |
| H-008 interactive passthrough | Failed severely with `python3` |
| H-013 disposable default `$HOME/.aish` | Passed |
| H-016 accessibility observation | Passed in current theme, configurability recommended |
| Draft editing `Alt-Left` / `Alt-Right` | Passed after retry |

## Environment Setup Used

### Isolated AISH_HOME

```bash
cargo build
export AISH_MANUAL_ROOT="/tmp/aish-manual-$(date +%s)"
mkdir -p "$AISH_MANUAL_ROOT"
export AISH_HOME="$AISH_MANUAL_ROOT/home"
./target/debug/aish
```

### Disposable Default HOME Test

```bash
cd /Users/evenson/aish
export AISH_TEST_HOME="/tmp/aish-default-home-$(date +%s)"
mkdir -p "$AISH_TEST_HOME"
unset AISH_HOME
HOME="$AISH_TEST_HOME" EDITOR=vim ./target/debug/aish
```

## Detailed Results

## Startup And Basic Execution

### Result

Aish built and started successfully.

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.27s
```

Prompt appeared normally:

```text
evenson@ ~/aish >
```

Basic shell execution worked:

```bash
printf 'a\nb\nc\n'
```

Output:

```text
a
b
c
```

### Conclusion

Startup and normal command execution passed.

## H-001: Inline Completion Visual Rendering

### Steps

1. Create history:

```bash
echo visual-completion-test
```

2. Type:

```text
echo visual
```

3. Press `Tab` once.

### Observed

- Inline suggestion appeared after pressing `Tab`.
- Below-prompt candidate list appeared.
- Inline suggestion was gray / dim.
- It was clear, readable, and aligned with typed input.
- It was not confused with committed text.
- No overlap, flicker, stale rendering, or misalignment was observed.

### Result

Visual rendering passed in Ghostty.

### UX Issues

- Inline suggestion duplicated the top item in the below-prompt candidate list.
- When both inline suggestion and candidate rows are visible, the same suggestion should not be repeated.
- Inline currently appears only after `Tab`.
- Preferred model:

```text
completion.inline = "always" | "on_tab" | "off"
```

### Recommendation

```toml
[completion]
inline = "on_tab" # always | on_tab | off
```

Candidate rows should display only additional alternatives or remaining suffixes, not duplicate the inline suggestion.

## H-002: Narrow Terminal Long Input And Completion

### Steps

1. Resize Ghostty to a narrow width, approximately 50-60 columns.
2. Type a long command:

```bash
echo very-long-completion-candidate-alpha-beta-gamma-delta-epsilon-zeta
```

### Observed

Before completion was even triggered, long input redraw became corrupted.

Observed fragments looked like:

```text
evenson@ ~/aish > echo very-long-completion-candidatevenson@ ~/aish > echo very-long-completion-candidevenson@ ~/aish > echo very-long-completion-cand...
```

### Expected

- Long input should wrap or redraw cleanly.
- Prompt and draft should not be repeatedly concatenated.
- Narrow terminal rendering should remain readable enough to trigger and inspect completions.

### Recovery

`Ctrl-C` followed by `Ctrl-L` restored a clean prompt.

### Result

Failed.

## H-003: `Tab` Full Accept And Word Accept

### Full Accept

Configuration:

```text
completion.tab_accept=full
```

After inline suggestion appeared, pressing `Tab` again accepted the full suggestion:

```bash
echo visual-completion-test
```

Candidate list disappeared and no residual rendering was observed.

Result: passed.

### Word Accept

Configuration changed with:

```bash
#completion tab-accept word
```

A history command was added:

```bash
echo word-alpha word-beta word-gamma
```

Typing:

```text
echo word
```

then pressing `Tab` showed inline completion. Pressing `Tab` again accepted:

```bash
echo word-alpha
```

Candidate list disappeared and no residual rendering was observed.

Single-step word acceptance mechanically passed.

### Completion Model Issue

The intended model is:

- History suggestions remain whole-command suggestions.
- Inline suggestions should display the full remaining suffix.
- `completion.tab_accept = "word"` only controls how much of the visible suggestion is accepted per `Tab`.
- Matching should prefer history commands with the longest / most relevant same-position match across whitespace-separated words.
- Candidate rows should show only the remaining suffix and should not repeat already typed input.
- This matching model should be independent of accept mode.

Example desired behavior:

History:

```bash
command add 100 file
```

Current input:

```bash
command add 200
```

Expected:

- Aish should still consider `command add 100 file` a relevant structural match because `command` and `add` match at the same word positions.
- It should continue suggesting useful remaining suffix, such as:

```text
file
```

This should apply regardless of whether the user reached the input by manual typing, full accept and edit, word accept and edit, or paste and edit.

### Actual Problem

Aish appeared to treat history arguments as independent token candidates. After accepting or typing partial arguments, completion lost the whole-command history context.

### Result

Partially passed.

## H-004: Clipboard Paste

### Single-Line Paste

Pasted:

```bash
echo single-line-paste-test
```

Observed:

```text
[editor draft: 1 line(s), 28 byte(s); review before Enter; Ctrl-X Ctrl-E to edit, Enter to run]
' 'echo single-line-paste-test'



single-line-paste-test
```

### Result

| Check | Result |
|---|---|
| Did not auto-execute | Passed |
| Entered draft review for single-line paste | Unexpected / needs design decision |
| Draft hint clarity | Too verbose |
| Draft rendering | Strange leading quoted space |
| Output spacing | Excessive blank lines |

### Multi-Line Paste

Pasted:

```bash
echo multi-line-one
echo multi-line-two
echo multi-line-three
```

Observed summary:

```text
[editor draft: 3 line(s), 62 byte(s); review before Enter; Ctrl-X Ctrl-E to edit, Enter to run]
```

After pressing `Enter`:

```text
' 'echo multi-line-one\necho multi-line-two\necho multi-line-three'



multi-line-one


multi-line-two


multi-line-three
```

### Result

Safety passed: it did not auto-execute before review.

UX failed:

- Review hint is too long.
- Multi-line content was not clearly reviewable, only summarized.
- Rendered command had strange leading quoted space.
- Execution output had excessive blank lines.

### Additional Design Finding: Trailing Newline Paste

A likely cause of inconsistent paste behavior was pasted content containing a trailing newline.

Risk:

- A single-line command copied with trailing newline may submit immediately.
- Multi-line blocks may execute line-by-line unless intercepted.

Recommended behavior:

```toml
[paste]
enabled = true
strip_trailing_newline = true
review_empty_draft = true
review_non_empty_draft = false
review_multiline = true
detect_initial_burst = true
```

### Initial Burst Detection Design

Aish may treat abnormal high-speed input from an empty draft as paste-like input.

Suggested behavior:

- Only apply timing-based detection when draft is empty before the first character.
- If user has already typed anything, do not trigger paste review based on timing.
- Fast input in the middle of editing is normal editing.
- Whether paste-like input is folded into review should be configurable.

Suggested config:

```toml
[paste]
detect_initial_burst = true
initial_burst_threshold_ms = 30
initial_burst_min_chars = 8
review_initial_burst = true
```

## H-005: Real `vim` Editor Flow

### Steps

1. Start Aish with `EDITOR=vim`.
2. Press `Ctrl-X Ctrl-E`.
3. In `vim`, enter:

```bash
echo edited-by-vim
```

4. Save and quit with `:wq`.

### Observed

Aish returned to prompt and displayed:

```text
[editor draft: 2 line(s), 20 byte(s); review before Enter; Ctrl-X Ctrl-E to edit, Enter to run]
```

After pressing `Enter`:

```text
' 'echo edited-by-vim'



edited-by-vim
```

### Result

| Check | Result |
|---|---|
| Entered real `vim` | Passed |
| Terminal restored after `vim` | Passed |
| Did not auto-execute | Passed |
| Executed after `Enter` | Passed |
| Prompt restored | Passed |
| Draft summary | Too verbose |
| Line count | Misleading |
| Execution rendering | Failed |

### Issues

- One non-empty command line was reported as `2 line(s)`, likely because `vim` writes a trailing newline.
- User-facing summary should count non-empty command lines or command count.
- Empty editor result showed:

```text
[editor draft: 1 line(s), 0 byte(s); review before Enter; Ctrl-X Ctrl-E to edit, Enter to run]
```

Expected:

```text
[editor empty; canceled]
```

or simply return to normal prompt.

### Suggested Summary

```text
[draft: 1 line; Enter run, Ctrl-X Ctrl-E edit]
```

or:

```text
[draft: 1 non-empty line, 20 bytes; Enter run, Ctrl-X Ctrl-E edit]
```

## H-006: Editor Failure Handling

### Steps

Restart Aish from parent shell with:

```bash
export EDITOR=false
unset VISUAL
./target/debug/aish
```

Then press:

```text
Ctrl-X Ctrl-E
```

### Observed

```text
editor exited without saving draft: status=1
```

### Result

Passed.

### Additional Note

Changing `EDITOR` inside a running Aish session did not affect `Ctrl-X Ctrl-E`.

Observed:

1. Started with `EDITOR=vim`.
2. Ran inside Aish:

```bash
export EDITOR=false
```

3. Pressed `Ctrl-X Ctrl-E`.
4. Aish still launched `vim`.

Likely cause:

- Editor selection is read from Aish process environment at startup, not backend shell environment.

Recommendation:

- Document this, or provide Aish-specific editor configuration.

Suggested config:

```toml
[editor]
command = "vim"
args = []
```

## H-007: Real `fzf` Pickers

### History Picker

Prepared:

```bash
echo fzf-history-alpha
echo fzf-history-beta
```

Opened with:

```text
Ctrl-R
```

Observed:

- `fzf` opened.
- Both history items were visible.
- Selecting `echo fzf-history-beta` inserted it into draft.
- It did not auto-execute.
- Screen returned cleanly.

Result: passed.

### History Picker Cancellation

Steps:

1. Draft contained:

```bash
echo fzf-history-beta
```

2. Opened `Ctrl-R`.
3. Canceled with `Esc`.

Observed:

```text
evenson@ ~/aish > echo fzf-history-betahistory search cancelled
evenson@ ~/aish > echo fzf-history-beta
```

Result:

- Draft preservation passed.
- Prompt usability passed.
- Cancellation message rendering failed because it was concatenated with draft text.

Expected:

```text
history search cancelled
evenson@ ~/aish > echo fzf-history-beta
```

### File Picker With Spaces

Created:

```bash
printf 'picker cwd file ok\n' > "aish picker file with spaces.txt"
```

Opened with:

```text
Ctrl-X Ctrl-F
```

Selected file inserted:

```bash
'aish picker file with spaces.txt'
```

Then converted to:

```bash
cat 'aish picker file with spaces.txt'
```

Output:

```text
picker cwd file ok
```

Result: passed.

### Env Picker

When variable was exported inside Aish:

```bash
export AISH_PICKER_TEST_VAR=picker-env-ok
```

it did not appear in env picker.

When variable was exported before launching Aish:

```bash
export AISH_PICKER_TEST_VAR=picker-env-ok
./target/debug/aish
```

env picker found it and inserted:

```bash
$AISH_PICKER_TEST_VAR
```

Running:

```bash
echo $AISH_PICKER_TEST_VAR
```

produced:

```text
picker-env-ok
```

### Env Picker Design Decision

Use backend shell environment by default, with startup environment fallback.

Suggested config:

```toml
[picker.env]
source = "backend"
fallback = "startup"
timeout_ms = 500
```

Rationale:

- Users expect variables exported inside Aish to appear.
- Startup fallback keeps picker usable if backend shell is unavailable.
- Timeout avoids blocking UI indefinitely.

## H-008: Interactive Passthrough

### Python REPL Test

Ran:

```bash
python3
```

### Observed

- No Python banner.
- No `>>>` prompt.
- Keyboard input could not be entered or was not visibly accepted.
- `Ctrl-C` did not recover prompt.
- `Ctrl-D` did not recover prompt.
- The Aish session became unusable.

### Result

Failed severely.

### Subsequent Startup Failure

After closing/recovering and trying to restart with the same isolated `AISH_HOME`, Aish eventually reported:

```text
Error: timed out waiting for backend shell ready marker
```

Process check did not show obvious stale `aish` or `python3` processes.

Starting with a fresh isolated `AISH_HOME` succeeded without timeout.

### Recommendation

- Treat arbitrary interactive passthrough as high priority.
- At minimum, `python3` REPL should either work or be handled with clear unsupported behavior.
- `Ctrl-C` / `Ctrl-D` should recover or provide a reliable escape path.
- Passthrough failures should not poison future startup using the same Aish home.

## H-013: Default `$HOME/.aish`

### Steps

Started Aish without `AISH_HOME` and with disposable `HOME`:

```bash
export AISH_TEST_HOME="/tmp/aish-default-home-$(date +%s)"
mkdir -p "$AISH_TEST_HOME"
unset AISH_HOME
HOME="$AISH_TEST_HOME" EDITOR=vim ./target/debug/aish
```

### Observed

Aish started successfully and created:

```text
$HOME/.aish
```

Contents:

```text
cache/
config.toml
history/
logs/
secrets/
templates/
```

### Config Persistence

Changed:

```bash
#completion max 3
```

Restarted with same disposable `HOME`.

Then:

```bash
#completion
```

showed:

```text
completion.max_results=3
completion.ignore_spaces=true
completion.template_first=true
completion.inline=true
completion.tab_accept=full
```

### Result

Passed.

## H-016: Accessibility Observation

### Steps

Created history:

```bash
echo accessibility-completion-test
```

Typed:

```text
echo accessibility
```

Pressed `Tab`.

### Observed

- Inline suggestion was clear and readable.
- Dim / gray text was not too faint.
- Candidate list could be understood without relying only on color.
- No important information appeared to rely only on subtle color.
- No overlap, misalignment, or stale rendering was observed.

### Result

Passed in current Ghostty theme.

### Recommendation

Make completion colors / inline style configurable.

Suggested config:

```toml
[completion]
inline = "on_tab"      # always | on_tab | off
inline_style = "dim"   # dim | high_contrast | underline | none
```

## Other Notes

### Draft Editing: Alt-Left / Alt-Right

Initially suspected no effect, but after retrying:

- `Alt-Left`
- `Alt-Right`

worked in Ghostty.

No failure recorded.

## Highest Priority Issues

### 1. H-008 Interactive Passthrough Hang

Severity: high.

`python3` made the session unusable, and `Ctrl-C` / `Ctrl-D` did not recover.

### 2. H-002 Narrow Terminal Redraw Corruption

Severity: high.

Long input in narrow terminal corrupted prompt redraw before completion.

### 3. Draft Review Rendering

Severity: medium-high.

Affects paste and editor flows:

- Strange leading quoted space:

```text
' 'echo edited-by-vim'
```

- Excessive blank lines.
- Overly verbose summary.
- Misleading line counts.
- Empty draft enters review.

### 4. Completion Model

Severity: medium-high.

Current behavior appears token-oriented. Desired behavior:

- Whole-command suggestions.
- Full suffix inline display.
- Same-position structural matching.
- Accept mode only controls how much suffix is accepted.

### 5. Paste Safety With Trailing Newline

Severity: medium-high.

Aish should protect against pasted trailing newline causing immediate execution, with configurable behavior.

### 6. Picker Cancellation Rendering

Severity: medium.

Cancellation message concatenates with current draft.

### 7. Env Picker Source

Severity: medium.

Env picker should prefer live backend shell environment with startup fallback.

## Suggested Automated Regressions

| Issue | Suggested Layer |
|---|---|
| Draft review quoted-space rendering | Rust unit / integration |
| Excessive blank lines after draft execution | tmux capture |
| Empty editor draft enters review | Rust / expect |
| Editor line count should use non-empty lines | Rust |
| Picker cancellation message newline/redraw | tmux capture |
| Narrow terminal long input redraw | tmux capture with small pane |
| Paste trailing newline stripping/review | expect byte-stream |
| Completion whole-command suffix model | Rust completion tests |
| Env picker backend fallback behavior | Rust + integration mock |
| `python3` passthrough hang | manual first, then expect/tmux if stable |
