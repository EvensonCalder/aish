#!/bin/sh
set -eu

SESSION="aish-status-command-$$"
HOME_DIR="/tmp/aish-tmux-status-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" '#status' Enter
sleep 2
tmux send-keys -t "$SESSION" 'echo after-status' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^last_status=none$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.enabled=true$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.mode=auto$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.max_results=5$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.coalesce_ms=50$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.display_delay_ms=120$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.inline=true$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.fuzzy=true$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.tab_accept=word$'
printf '%s\n' "$CAPTURE" | rg -q '^after-status$'
