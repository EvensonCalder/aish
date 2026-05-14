#!/bin/sh
set -eu

SESSION="aish-exit-command-$$"
HOME_DIR="/tmp/aish-tmux-exit-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" '#exit' Enter
sleep 2

if tmux has-session -t "$SESSION" >/dev/null 2>&1; then
    printf 'aish tmux session still exists after #exit\n' >&2
    tmux capture-pane -p -t "$SESSION" >&2 || true
    exit 1
fi
