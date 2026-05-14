#!/bin/sh
set -eu

SESSION="aish-manual-less-$$"
HOME_DIR="/tmp/aish-tmux-manual-less-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR"' EXIT INT TERM

if ! command -v less >/dev/null 2>&1; then
    printf 'less not installed; skipping passthrough tmux workflow\n'
    exit 0
fi

mkdir -p "$HOME_DIR"
tmux new-session -d -x 120 -y 40 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' LESS= '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'less README.md' Enter
sleep 2
tmux send-keys -t "$SESSION" q
sleep 2
tmux send-keys -t "$SESSION" 'echo after-less' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^after-less$'
