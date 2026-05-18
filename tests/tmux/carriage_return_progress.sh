#!/bin/sh
set -eu

SESSION="aish-cr-progress-$$"
HOME_DIR="/tmp/aish-tmux-cr-progress-home-$$"
BIN_DIR="$HOME_DIR/bin"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR/.aish" "$BIN_DIR"
printf '[completion]\ninline = false\n' > "$HOME_DIR/.aish/config.toml"
{
    printf '%s\n' '#!/bin/sh'
    printf '%s\n' "printf 'progress 1/3\\rprogress 2/3\\rprogress 3/3\\n'"
} > "$BIN_DIR/aish-cr-progress"
chmod +x "$BIN_DIR/aish-cr-progress"
tmux new-session -d -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' PATH='$BIN_DIR':\"\$PATH\" '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" "aish-cr-progress" Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^progress 3/3$'
if printf '%s\n' "$CAPTURE" | rg -q '^progress [12]/3$'; then
    printf 'carriage-return progress updates were expanded into separate visible lines\n' >&2
    exit 1
fi
