#!/bin/sh
set -eu

SESSION="aish-bash-clear-cursor-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-bash-clear-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -x 120 -y 20 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'bash' Enter
sleep 2
tmux send-keys -t "$SESSION" 'exit' Enter
sleep 2
tmux send-keys -t "$SESSION" 'clear' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
CURSOR="$(tmux display-message -p -t "$SESSION" '#{cursor_x} #{cursor_y}')"
printf '%s\n' "$CAPTURE"
printf 'cursor=%s\n' "$CURSOR"

if printf '%s\n' "$CAPTURE" | rg -q '^bash$|^exit$'; then
    printf 'screen still contains nested bash command text\n' >&2
    exit 1
fi

FIRST_NON_EMPTY="$(printf '%s\n' "$CAPTURE" | awk 'NF { print NR; exit }')"
if [ "$FIRST_NON_EMPTY" != "1" ]; then
    printf 'first non-empty screen line after bash then clear was %s, expected 1\n' "${FIRST_NON_EMPTY:-none}" >&2
    exit 1
fi

case "$CURSOR" in
    *" 0") ;;
    *)
        printf 'cursor after bash then clear was %s, expected row 0\n' "$CURSOR" >&2
        exit 1
        ;;
esac
