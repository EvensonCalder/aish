#!/bin/sh
set -eu

SESSION="aish-rm-prompt-$$"
HOME_DIR="/tmp/aish-rm-prompt-home-$$"
WORK_DIR="/tmp/aish-rm-prompt-work-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$WORK_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR" "$WORK_DIR"
touch "$WORK_DIR/1.t"
chmod 400 "$WORK_DIR/1.t"

tmux new-session -d -x 80 -y 10 -c "$WORK_DIR" -s "$SESSION" "env SHELL=/bin/bash HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'rm 1.t' Enter
sleep 1

EARLY_CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$EARLY_CAPTURE"

if ! printf '%s\n' "$EARLY_CAPTURE" | rg -q "remove .*1\\.t.*\\?"; then
  printf 'rm prompt was not visible before answering\n' >&2
  exit 1
fi

if printf '%s\n' "$EARLY_CAPTURE" | rg -q '^rm-declined$'; then
  printf 'rm prompt completed before the user answered\n' >&2
  exit 1
fi

tmux send-keys -t "$SESSION" 'n' Enter
sleep 1
tmux send-keys -t "$SESSION" 'test -e 1.t && echo rm-declined' Enter
sleep 1

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^rm-declined$'
