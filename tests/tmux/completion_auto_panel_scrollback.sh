#!/bin/sh
set -eu

SESSION="aish-completion-auto-scrollback-$$"
HOME_DIR="/tmp/aish-cas-home-$$"
WORK_DIR="/tmp/aish-cas-work-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$WORK_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR" "$WORK_DIR"
touch "$WORK_DIR/alpha-one.txt" "$WORK_DIR/alpha-two.txt" "$WORK_DIR/alpha-three.txt"

tmux new-session -d -x 60 -y 5 -c "$WORK_DIR" -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'cat alpha-'
sleep 1

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^(file|history|template|exec)[[:space:]]+cat alpha-(one|two|three)\.txt$'

PROMPT_REDRAWS="$(printf '%s\n' "$CAPTURE" | rg -c '> cat( |$| alpha)')"
if [ "$PROMPT_REDRAWS" -gt 2 ]; then
  printf 'ordinary typing leaked %s prompt redraws into scrollback\n' "$PROMPT_REDRAWS" >&2
  exit 1
fi

tmux send-keys -t "$SESSION" C-c
sleep 1

CLEARED_CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CLEARED_CAPTURE"

LAST_NON_EMPTY="$(printf '%s\n' "$CLEARED_CAPTURE" | awk 'NF { line=$0 } END { print line }')"
printf '%s\n' "$LAST_NON_EMPTY" | rg -q '> *$'
