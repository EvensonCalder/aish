#!/bin/sh
set -eu

SESSION="aish-history-mode-$$"
HOME_DIR="/tmp/aish-tmux-history-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'echo history-tmux-ok' Enter
sleep 2
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" Tab
sleep 1
HISTORY_CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
tmux send-keys -t "$SESSION" Up
sleep 1
tmux send-keys -t "$SESSION" Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
printf '%s\n' "$HISTORY_CAPTURE"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$HISTORY_CAPTURE" | rg -q '\$ '
count="$(printf '%s\n' "$CAPTURE" | rg -c '^history-tmux-ok$')"
if [ "$count" -lt 2 ]; then
    printf 'expected history command output at least twice, got %s\n' "$count" >&2
    exit 1
fi
LAST_NON_EMPTY="$(printf '%s\n' "$CAPTURE" | awk 'NF { line=$0 } END { print line }')"
if ! printf '%s\n' "$LAST_NON_EMPTY" | rg -q '> *$'; then
    printf 'expected blank draft prompt after history execution, got: %s\n' "$LAST_NON_EMPTY" >&2
    exit 1
fi
