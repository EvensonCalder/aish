#!/bin/sh
set -eu

SESSION="aish-completion-no-matches-$$"
HOME_DIR="/tmp/aish-tmux-completion-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'zzzzzz-no-match' Tab
sleep 1
PANEL_CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
tmux send-keys -t "$SESSION" Escape
sleep 1
tmux send-keys -t "$SESSION" 'echo after-completion' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
printf '%s\n' "$PANEL_CAPTURE"
printf '%s\n' "$CAPTURE"

if printf '%s\n' "$PANEL_CAPTURE" | rg -q 'no completions'; then
    printf '%s\n' "unexpected no-completions panel" >&2
    exit 1
fi
printf '%s\n' "$CAPTURE" | rg -q '^after-completion$'
