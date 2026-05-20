#!/bin/sh
set -eu

SESSION="aish-mode-redraw-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-mode-redraw-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"

capture_pane() {
    tmux capture-pane -p -t "$SESSION" 2>/dev/null || true
}

wait_for_capture() {
    pattern="$1"
    attempts="${2:-100}"
    attempt=0
    while [ "$attempt" -lt "$attempts" ]; do
        if ! tmux has-session -t "$SESSION" >/dev/null 2>&1; then
            printf 'tmux session exited while waiting for pattern: %s\n' "$pattern" >&2
            return 1
        fi
        CAPTURE="$(capture_pane)"
        if printf '%s\n' "$CAPTURE" | rg -q "$pattern"; then
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 0.2
    done
    printf 'timed out waiting for pattern: %s\n' "$pattern" >&2
    printf '%s\n' "$CAPTURE" >&2
    return 1
}

wait_for_capture '>[[:space:]]*$' 150
tmux send-keys -t "$SESSION" 'echo before-mode-redraw' Enter
wait_for_capture '^before-mode-redraw$'
tmux send-keys -t "$SESSION" C-c
sleep 0.2
tmux send-keys -t "$SESSION" Tab
sleep 0.2
tmux send-keys -t "$SESSION" Tab
sleep 0.2
tmux send-keys -t "$SESSION" Tab
sleep 0.2
tmux send-keys -t "$SESSION" C-c
sleep 0.2
tmux send-keys -t "$SESSION" 'echo after-mode-redraw' Enter
wait_for_capture '^after-mode-redraw$'

CAPTURE="$(capture_pane)"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^before-mode-redraw$'
printf '%s\n' "$CAPTURE" | rg -q '^after-mode-redraw$'
