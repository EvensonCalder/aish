#!/bin/sh
set -eu

SESSION="aish-completion-enter-clears-$$"
HOME_DIR="/tmp/aish-tmux-completion-enter-home-$$"
WORK_DIR="/tmp/aish-tmux-completion-enter-work-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"

cleanup() {
  tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true
  for path in "$HOME_DIR" "$WORK_DIR"; do
    for _ in 1 2 3 4 5; do
      rm -rf "$path" 2>/dev/null && break
      sleep 0.1
    done
    rm -rf "$path" 2>/dev/null || true
  done
}

trap cleanup EXIT INT TERM

mkdir -p "$HOME_DIR" "$WORK_DIR"
touch "$WORK_DIR/alpha-one.txt" "$WORK_DIR/alpha-two.txt"

tmux new-session -d -x 100 -y 30 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" "cd $WORK_DIR" Enter
sleep 1
tmux send-keys -t "$SESSION" C-c
sleep 1

tmux send-keys -t "$SESSION" 'cat alpha-'
sleep 1
tmux send-keys -t "$SESSION" Enter
sleep 2
tmux send-keys -t "$SESSION" 'echo after-enter' Enter
sleep 1

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q 'cat alpha-'
printf '%s\n' "$CAPTURE" | rg -q '^after-enter$'
if printf '%s\n' "$CAPTURE" | rg -q '^(file|history|template|exec)[[:space:]]+cat alpha-(one|two)\.txt$'; then
  echo "completion panel leaked into pane history after Enter" >&2
  exit 1
fi
