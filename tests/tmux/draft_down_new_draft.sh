#!/bin/sh
set -eu

SESSION="aish-draft-down-$$"
HOME_DIR="/tmp/aish-tmux-draft-down-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR"

tmux new-session -d -x 100 -y 30 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'echo saved-draft-by-down'
tmux send-keys -t "$SESSION" Down
sleep 1
tmux send-keys -t "$SESSION" Up
sleep 1
RESTORE_CAPTURE="$(tmux capture-pane -p -t "$SESSION")"

tmux send-keys -t "$SESSION" C-c
sleep 1

tmux send-keys -t "$SESSION" 'echo after-down-new-draft' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$RESTORE_CAPTURE"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$RESTORE_CAPTURE" | rg -q 'echo saved-draft-by-down'
printf '%s\n' "$CAPTURE" | rg -q '^after-down-new-draft$'
if printf '%s\n' "$CAPTURE" | rg -q 'saved-draft-by-downecho after-down-new-draft'; then
    printf '%s\n' "new command was appended to stale draft" >&2
    exit 1
fi

rg -q 'echo saved-draft-by-down' "$HOME_DIR/.aish/history/draft.jsonl"
