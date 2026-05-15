#!/bin/sh
set -eu

SESSION="aish-manual-editing-$$"
HOME_DIR="/tmp/aish-tmux-manual-editing-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -x 120 -y 50 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" "printf 'edit:%s\\n' ac"
tmux send-keys -t "$SESSION" Left
tmux send-keys -t "$SESSION" b
tmux send-keys -t "$SESSION" Enter
sleep 1

tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo end'
tmux send-keys -t "$SESSION" C-a
tmux send-keys -t "$SESSION" "printf 'ctrl-a:' && "
tmux send-keys -t "$SESSION" C-e
tmux send-keys -t "$SESSION" -- '-tail'
tmux send-keys -t "$SESSION" Enter
sleep 1

tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo alpha gamma'
tmux send-keys -t "$SESSION" M-b
tmux send-keys -t "$SESSION" 'beta-'
tmux send-keys -t "$SESSION" Enter
sleep 1

tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo one three'
tmux send-keys -t "$SESSION" C-a
tmux send-keys -t "$SESSION" M-f
tmux send-keys -t "$SESSION" M-f
tmux send-keys -t "$SESSION" 'two '
tmux send-keys -t "$SESSION" Enter
sleep 1

tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo delete bad'
tmux send-keys -t "$SESSION" C-w
tmux send-keys -t "$SESSION" good
tmux send-keys -t "$SESSION" Enter
sleep 1

tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" garbage
tmux send-keys -t "$SESSION" C-u
tmux send-keys -t "$SESSION" 'echo ctrl-u-ok'
tmux send-keys -t "$SESSION" Enter
sleep 1

tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo ctrl-k-ok bad'
tmux send-keys -t "$SESSION" Left Left Left
tmux send-keys -t "$SESSION" C-k
tmux send-keys -t "$SESSION" Enter
sleep 1

tmux send-keys -t "$SESSION" 'echo should-not-run-edit'
tmux send-keys -t "$SESSION" Escape
sleep 1
tmux send-keys -t "$SESSION" 'echo after-escape-edit' Enter
sleep 1

tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" C-x C-g
sleep 1
tmux send-keys -t "$SESSION" 'echo ctrlx-ok' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^edit:abc$'
printf '%s\n' "$CAPTURE" | rg -q '^ctrl-a:end-tail$'
printf '%s\n' "$CAPTURE" | rg -q '^alpha beta-gamma$'
printf '%s\n' "$CAPTURE" | rg -q '^one two three$'
printf '%s\n' "$CAPTURE" | rg -q '^delete good$'
printf '%s\n' "$CAPTURE" | rg -q '^ctrl-u-ok$'
printf '%s\n' "$CAPTURE" | rg -q '^ctrl-k-ok$'
! printf '%s\n' "$CAPTURE" | rg -q '^should-not-run-edit$'
printf '%s\n' "$CAPTURE" | rg -q '^after-escape-edit$'
printf '%s\n' "$CAPTURE" | rg -q '^ctrlx-ok$'
