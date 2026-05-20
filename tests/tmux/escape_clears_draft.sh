#!/bin/sh
set -eu

SESSION="aish-escape-clears-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-escape-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'echo should-not-run'
sleep 1
tmux send-keys -t "$SESSION" Escape
sleep 1
tmux send-keys -t "$SESSION" 'echo after-escape' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
printf '%s\n' "$CAPTURE"

if printf '%s\n' "$CAPTURE" | rg -q '^should-not-run$'; then
    printf 'cleared draft executed unexpectedly\n' >&2
    exit 1
fi
printf '%s\n' "$CAPTURE" | rg -q '^after-escape$'
