#!/bin/sh
set -eu

SESSION="aish-cr-progress-$$"
HOME_DIR="/tmp/aish-tmux-cr-progress-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" "printf 'progress 1/3\\rprogress 2/3\\rprogress 3/3\\n'" Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^progress 3/3$'
if printf '%s\n' "$CAPTURE" | rg -q '^progress [12]/3$'; then
    printf 'carriage-return progress updates were expanded into separate visible lines\n' >&2
    exit 1
fi
