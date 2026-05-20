#!/bin/sh
set -eu

SESSION="aish-manual-ai-context-sync-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-manual-ai-home-$$"
MARKER="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-context-disabled-$$"
DANGEROUS_MARKER="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-context-danger-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$MARKER" "$DANGEROUS_MARKER" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
printf 'must remain\n' > "$DANGEROUS_MARKER"
tmux new-session -d -x 120 -y 60 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' AISH_TMUX_CONTEXT_KEY='test-key' GIT_CONFIG_COUNT=3 GIT_CONFIG_KEY_0=commit.gpgsign GIT_CONFIG_VALUE_0=false GIT_CONFIG_KEY_1=user.name GIT_CONFIG_VALUE_1='Aish Tmux' GIT_CONFIG_KEY_2=user.email GIT_CONFIG_VALUE_2=aish@example.invalid '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" '#model tmux-model' Enter
sleep 1
tmux send-keys -t "$SESSION" '#base-url https://127.0.0.1:1/v1' Enter
sleep 1
tmux send-keys -t "$SESSION" '#env-key AISH_TMUX_CONTEXT_KEY' Enter
sleep 1
tmux send-keys -t "$SESSION" C-l
sleep 1
tmux send-keys -t "$SESSION" '#status' Enter
sleep 1
STATUS_CAPTURE="$(tmux capture-pane -p -t "$SESSION")"

tmux send-keys -t "$SESSION" '#context' Enter
sleep 1
tmux send-keys -t "$SESSION" '#context off' Enter
sleep 1
tmux send-keys -t "$SESSION" "# explain disabled < touch $MARKER" Enter
sleep 1
tmux send-keys -t "$SESSION" '#context on' Enter
sleep 1
tmux send-keys -t "$SESSION" '# summarize this < echo context-output' Enter
sleep 1
tmux send-keys -t "$SESSION" n
sleep 1
tmux send-keys -t "$SESSION" '#context confirm off' Enter
sleep 1
tmux send-keys -t "$SESSION" '#context 4' Enter
sleep 1
tmux send-keys -t "$SESSION" '# explain truncation < printf 123456789' Enter
sleep 2
tmux send-keys -t "$SESSION" "# explain danger < rm -rf $DANGEROUS_MARKER" Enter
sleep 1
tmux send-keys -t "$SESSION" n
sleep 1

tmux send-keys -t "$SESSION" '#set-remote /tmp/nonexistent-aish-remote.git' Enter
sleep 1
tmux send-keys -t "$SESSION" '#sync off' Enter
sleep 1
tmux send-keys -t "$SESSION" '#sync @hourly' Enter
sleep 1
tmux send-keys -t "$SESSION" '#sync ai on' Enter
sleep 1
tmux send-keys -t "$SESSION" '#sync history on' Enter
sleep 1
tmux send-keys -t "$SESSION" '#sync templates on' Enter
sleep 1
tmux send-keys -t "$SESSION" '#sync drafts on' Enter
sleep 1
tmux send-keys -t "$SESSION" '#sync ai maybe' Enter
sleep 1
tmux send-keys -t "$SESSION" '#sync now' Enter
sleep 2
tmux send-keys -t "$SESSION" 'echo after-ai-context-sync' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$STATUS_CAPTURE"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$STATUS_CAPTURE" | rg -q '^ai.model=tmux-model$'
printf '%s\n' "$STATUS_CAPTURE" | rg -q '^ai.final_url=https://127.0.0.1:1/v1/chat/completions$'
printf '%s\n' "$STATUS_CAPTURE" | rg -q '^ai.key_source=env$'
! printf '%s\n' "$STATUS_CAPTURE" | rg -q 'AISH_TMUX_CONTEXT_KEY'

printf '%s\n' "$CAPTURE" | rg -q 'context.enabled=false'
printf '%s\n' "$CAPTURE" | rg -q "context collection is disabled; context command not executed: touch $MARKER"
test ! -e "$MARKER"
printf '%s\n' "$CAPTURE" | rg -q 'context.enabled=true'
printf '%s\n' "$CAPTURE" | rg -q 'Run context command\? \[Y/n\]'
printf '%s\n' "$CAPTURE" | rg -q '^context command skipped: echo context-output$'
printf '%s\n' "$CAPTURE" | rg -q 'context.confirm=false'
printf '%s\n' "$CAPTURE" | rg -q 'context.max_bytes=4'
printf '%s\n' "$CAPTURE" | rg -q '^context output truncated to 4 bytes$'
printf '%s\n' "$CAPTURE" | rg -q "dangerous context command requires confirmation: rm -rf $DANGEROUS_MARKER"
printf '%s\n' "$CAPTURE" | rg -q "context command skipped: rm -rf $DANGEROUS_MARKER"
test -e "$DANGEROUS_MARKER"
printf '%s\n' "$CAPTURE" | rg -q '^sync.remote=/tmp/nonexistent-aish-remote.git$'
printf '%s\n' "$CAPTURE" | rg -q '^sync.enabled=false$'
printf '%s\n' "$CAPTURE" | rg -q '^sync.schedule=@hourly$'
printf '%s\n' "$CAPTURE" | rg -q '^sync.ai=true$'
printf '%s\n' "$CAPTURE" | rg -q '^sync.history=true$'
printf '%s\n' "$CAPTURE" | rg -q '^sync.templates=true$'
printf '%s\n' "$CAPTURE" | rg -q '^sync.drafts=true$'
printf '%s\n' "$CAPTURE" | rg -q '^usage: #sync ai\|history\|templates\|drafts on\|off$'
printf '%s\n' "$CAPTURE" | rg -q '^sync failed: git push -u origin HEAD$'
printf '%s\n' "$CAPTURE" | rg -q '^after-ai-context-sync$'
test ! -e "$HOME_DIR/.aish/cache/runtime/scheduler"
