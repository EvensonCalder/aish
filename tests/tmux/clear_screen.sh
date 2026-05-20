#!/bin/sh
set -eu

SESSION="aish-clear-screen-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-clear-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'echo before-clear' Enter
sleep 2
tmux send-keys -t "$SESSION" C-l
sleep 1
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo after-clear' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^after-clear$'
if printf '%s\n' "$CAPTURE" | rg -q 'before-clear'; then
    printf 'screen still contains pre-clear output\n' >&2
    exit 1
fi
