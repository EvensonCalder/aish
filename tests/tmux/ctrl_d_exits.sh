#!/bin/sh
set -eu

SESSION="aish-ctrl-d-exits-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-ctrl-d-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'; printf '\n__AISH_AFTER_CTRL_D__\n'; sleep 30"
sleep 5

tmux send-keys -t "$SESSION" C-d
sleep 2

if tmux has-session -t "$SESSION" >/dev/null 2>&1; then
    tmux capture-pane -p -t "$SESSION" >"$HOME_DIR/captured.txt"
else
    printf 'tmux session ended before Ctrl-D output could be captured\n' >&2
    exit 1
fi

cat "$HOME_DIR/captured.txt"
