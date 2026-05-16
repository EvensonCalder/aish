#!/bin/sh
set -eu

SESSION="aish-template-completion-$$"
HOME_DIR="/tmp/aish-tmux-template-completion-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR"

tmux new-session -d -x 100 -y 30 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" '#mt echo {something}' Enter
sleep 1

tmux send-keys -t "$SESSION" 'echo something'
sleep 1
AUTO_CAPTURE="$(tmux capture-pane -p -t "$SESSION")"

tmux send-keys -t "$SESSION" Tab
sleep 1
ACCEPT_CAPTURE="$(tmux capture-pane -p -t "$SESSION")"

tmux send-keys -t "$SESSION" Enter
sleep 1
BLOCK_CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"

tmux send-keys -t "$SESSION" C-c
sleep 1

tmux send-keys -t "$SESSION" '#mt echo {a} {older}' Enter
sleep 1
tmux send-keys -t "$SESSION" '#mt echo {a} {b} {c}' Enter
sleep 1

tmux send-keys -t "$SESSION" 'echo {a} {something}'
sleep 1
STRUCTURAL_AUTO_CAPTURE="$(tmux capture-pane -p -t "$SESSION")"

tmux send-keys -t "$SESSION" Tab
sleep 1
STRUCTURAL_ACCEPT_CAPTURE="$(tmux capture-pane -p -t "$SESSION")"

tmux send-keys -t "$SESSION" Enter
sleep 1
STRUCTURAL_BLOCK_CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"

printf '%s\n' "$AUTO_CAPTURE"
printf '%s\n' "$ACCEPT_CAPTURE"
printf '%s\n' "$BLOCK_CAPTURE"
printf '%s\n' "$STRUCTURAL_AUTO_CAPTURE"
printf '%s\n' "$STRUCTURAL_ACCEPT_CAPTURE"
printf '%s\n' "$STRUCTURAL_BLOCK_CAPTURE"

printf '%s\n' "$AUTO_CAPTURE" | rg -q '\{something\}'
printf '%s\n' "$ACCEPT_CAPTURE" | rg -q 'echo \{something\}'
printf '%s\n' "$BLOCK_CAPTURE" | rg -q '^cannot execute unresolved template placeholders: something$'
printf '%s\n' "$STRUCTURAL_AUTO_CAPTURE" | rg -q 'template[[:space:]]+echo \{a\} \{b\} \{c\}'
printf '%s\n' "$STRUCTURAL_ACCEPT_CAPTURE" | rg -q 'echo \{a\} \{b\} \{c\}'
printf '%s\n' "$STRUCTURAL_BLOCK_CAPTURE" | rg -q '^cannot execute unresolved template placeholders: a, b, c$'
