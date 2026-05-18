#!/bin/sh
set -eu

SESSION="aish-picker-cancel-render-$$"
HOME_DIR="/tmp/aish-tmux-picker-cancel-home-$$"
BIN_DIR="/tmp/aish-tmux-picker-cancel-bin-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$BIN_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR" "$BIN_DIR"
printf '#!/bin/sh\ncat >/dev/null\nexit 130\n' > "$BIN_DIR/fzf"
chmod +x "$BIN_DIR/fzf"

tmux new-session -d -x 120 -y 40 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' PATH='$BIN_DIR':\$PATH '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'echo picker-history-source' Enter
sleep 1
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo keep-after-picker-cancel'
tmux send-keys -t "$SESSION" C-r
sleep 2
tmux send-keys -t "$SESSION" Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^history search cancelled$'
! printf '%s\n' "$CAPTURE" | rg -q 'keep-after-picker-cancelhistory search cancelled'
printf '%s\n' "$CAPTURE" | rg -q '^keep-after-picker-cancel$'
