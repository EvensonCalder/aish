#!/bin/sh
set -eu

SESSION="aish-python-repl-$$"
HOME_DIR="/tmp/aish-tmux-python-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

if ! command -v python3 >/dev/null 2>&1; then
    printf 'python3 not installed; skipping python passthrough tmux workflow\n'
    exit 0
fi

mkdir -p "$HOME_DIR"
tmux new-session -d -x 120 -y 40 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'python3' Enter
sleep 2
tmux send-keys -t "$SESSION" "print('python-tmux-ok')" Enter
sleep 1
tmux send-keys -t "$SESSION" C-d
sleep 2
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo after-python-repl' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^python-tmux-ok$'
printf '%s\n' "$CAPTURE" | rg -q '^after-python-repl$'
