#!/bin/sh
set -eu

SESSION="aish-completion-right-$$"
HOME_DIR="/tmp/aish-tmux-completion-right-home-$$"
WORK_DIR="/tmp/aish-tmux-completion-right-work-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$WORK_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR" "$WORK_DIR"
printf 'accepted-right\n' > "$WORK_DIR/right-target.txt"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" "cd $WORK_DIR" Enter
sleep 2
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'cat right-t'
tmux send-keys -t "$SESSION" Right
sleep 1
tmux send-keys -t "$SESSION" Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q 'cat right-target.txt'
printf '%s\n' "$CAPTURE" | rg -q '^accepted-right$'
