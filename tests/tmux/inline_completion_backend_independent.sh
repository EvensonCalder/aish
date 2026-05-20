#!/bin/sh
set -eu

SESSION="aish-inline-backend-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-inline-home-$$"
WORK_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-inline-work-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
cleanup() {
    tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true
    sleep 0.2
    rm -rf "$HOME_DIR" "$WORK_DIR" || true
}
trap cleanup EXIT INT TERM

mkdir -p "$HOME_DIR/.aish" "$WORK_DIR"
if [ "${AISH_BACKEND_SHELL:-}" ]; then
    printf '[shell]\nbackend = "%s"\n' "$AISH_BACKEND_SHELL" > "$HOME_DIR/.aish/config.toml"
fi

tmux new-session -d -x 90 -y 30 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" "cd $WORK_DIR" Enter
sleep 1
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo inline-history seeded' Enter
sleep 1
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" '#completion tab-accept word' Enter
sleep 1
tmux send-keys -t "$SESSION" 'echo in'
sleep 1
PANEL_CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
tmux send-keys -t "$SESSION" Tab
sleep 1
tmux send-keys -t "$SESSION" Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$PANEL_CAPTURE"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$PANEL_CAPTURE" | rg -q 'inline-history'
printf '%s\n' "$CAPTURE" | rg -q 'echo inline-history'
printf '%s\n' "$CAPTURE" | rg -q '^inline-history$'
