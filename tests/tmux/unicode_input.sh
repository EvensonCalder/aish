#!/bin/sh
set -eu

SESSION="aish-unicode-input-$$"
HOME_DIR="/tmp/aish-tmux-unicode-home-$$"
UNICODE_WORD="café-你好"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' /Users/evenson/aish/target/debug/aish"
sleep 5

tmux send-keys -t "$SESSION" "printf 'unicode:%s\\n' '$UNICODE_WORD'" Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q "^unicode:${UNICODE_WORD}$"
