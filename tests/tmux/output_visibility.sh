#!/bin/sh
set -eu

SESSION="aish-output-visibility-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-home-$$"
EXPECTED_USER="$(id -un)"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'whoami' Enter
sleep 2
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'whoami' Enter
sleep 2
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo 123' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
printf '%s\n' "$CAPTURE"

count="$(printf '%s\n' "$CAPTURE" | rg -c "^${EXPECTED_USER}$")"
if [ "$count" -lt 2 ]; then
    printf 'expected repeated whoami output at least twice, got %s\n' "$count" >&2
    exit 1
fi
printf '%s\n' "$CAPTURE" | rg -q '^123$'
