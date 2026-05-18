#!/bin/sh
set -eu

SESSION="aish-continuation-cancel-$$"
HOME_DIR="/tmp/aish-tmux-continuation-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'echo "' Enter
sleep 1
CONTINUATION_CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo after-cancel' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
printf '%s\n' "$CONTINUATION_CAPTURE"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CONTINUATION_CAPTURE" | rg -q 'dquote>'
printf '%s\n' "$CAPTURE" | rg -q '^after-cancel$'
if printf '%s\n' "$CAPTURE" | rg -q 'dquote> .*after-cancel'; then
    printf 'prompt remained in continuation mode after Ctrl-C\n' >&2
    exit 1
fi
