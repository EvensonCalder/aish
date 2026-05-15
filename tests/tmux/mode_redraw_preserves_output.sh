#!/bin/sh
set -eu

SESSION="aish-mode-redraw-$$"
HOME_DIR="/tmp/aish-tmux-mode-redraw-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'echo before-mode-redraw' Enter
sleep 2
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" Tab
sleep 1
tmux send-keys -t "$SESSION" Tab
sleep 1
tmux send-keys -t "$SESSION" Tab
sleep 1
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo after-mode-redraw' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^before-mode-redraw$'
printf '%s\n' "$CAPTURE" | rg -q '^after-mode-redraw$'
