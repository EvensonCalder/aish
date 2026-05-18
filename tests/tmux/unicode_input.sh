#!/bin/sh
set -eu

SESSION="aish-unicode-input-$$"
HOME_DIR="/tmp/aish-tmux-unicode-home-$$"
UNICODE_WORD="café-你好"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" "printf 'unicode:%s\\n' '$UNICODE_WORD'" Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q "^unicode:${UNICODE_WORD}$"
