#!/bin/sh
set -eu

SESSION="aish-manual-less-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-manual-less-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

if ! command -v less >/dev/null 2>&1; then
    printf 'less not installed; skipping passthrough tmux workflow\n'
    exit 0
fi

mkdir -p "$HOME_DIR"
tmux new-session -d -x 120 -y 40 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' LESS= '$AISH_BIN'"

capture_pane() {
    tmux capture-pane -p -S - -t "$SESSION" 2>/dev/null || true
}

wait_for_capture() {
    pattern="$1"
    attempts="${2:-50}"
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
tmux send-keys -t "$SESSION" 'less README.md' Enter
wait_for_capture '^# Aish$'
tmux send-keys -t "$SESSION" q
wait_for_capture '>[[:space:]]*$'
tmux send-keys -t "$SESSION" C-c
wait_for_capture '>[[:space:]]*$'
tmux send-keys -t "$SESSION" 'echo after-less' Enter
wait_for_capture '^after-less$'

CAPTURE="$(capture_pane)"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^after-less$'
