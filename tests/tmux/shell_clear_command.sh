#!/bin/sh
set -eu

SESSION="aish-shell-clear-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-shell-clear-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -x 120 -y 20 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'echo before-shell-clear' Enter
sleep 1
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'clear' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
printf '%s\n' "$CAPTURE"

if printf '%s\n' "$CAPTURE" | rg -q 'before-shell-clear'; then
    printf 'screen still contains pre-clear output\n' >&2
    exit 1
fi

FIRST_NON_EMPTY="$(printf '%s\n' "$CAPTURE" | awk 'NF { print NR; exit }')"
if [ "$FIRST_NON_EMPTY" != "1" ]; then
    printf 'first non-empty screen line after shell clear was %s, expected 1\n' "${FIRST_NON_EMPTY:-none}" >&2
    exit 1
fi
