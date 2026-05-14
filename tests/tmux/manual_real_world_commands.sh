#!/bin/sh
set -eu

SESSION="aish-manual-real-world-$$"
HOME_DIR="/tmp/aish-tmux-manual-real-home-$$"
WORK_DIR="/tmp/aish-tmux-manual-real-work-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$WORK_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR" "$WORK_DIR"
tmux new-session -d -x 120 -y 50 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" "cd $WORK_DIR" Enter
sleep 1
tmux send-keys -t "$SESSION" "mkdir -p project/src project/logs" Enter
sleep 1
tmux send-keys -t "$SESSION" "printf 'alpha\\nbeta\\n' > project/src/input.txt" Enter
sleep 1
tmux send-keys -t "$SESSION" "grep beta project/src/input.txt" Enter
sleep 1
tmux send-keys -t "$SESSION" "touch 'file with spaces.txt'" Enter
sleep 1
tmux send-keys -t "$SESSION" "test -f 'file with spaces.txt' && echo spaced-file-ok" Enter
sleep 1
tmux send-keys -t "$SESSION" 'export AISH_REAL_WORLD=visible' Enter
sleep 1
tmux send-keys -t "$SESSION" 'printenv AISH_REAL_WORLD' Enter
sleep 1
tmux send-keys -t "$SESSION" "printf 'stderr-visible\\n' >&2" Enter
sleep 1
tmux send-keys -t "$SESSION" 'for x in one two; do echo loop-$x; done' Enter
sleep 1
tmux send-keys -t "$SESSION" "printf 'quoted:%s\\n' 'value with spaces'" Enter
sleep 1
tmux send-keys -t "$SESSION" false Enter
sleep 1
tmux send-keys -t "$SESSION" '#status' Enter
sleep 1
tmux send-keys -t "$SESSION" 'echo after-real-world' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^beta$'
printf '%s\n' "$CAPTURE" | rg -q '^spaced-file-ok$'
printf '%s\n' "$CAPTURE" | rg -q '^visible$'
printf '%s\n' "$CAPTURE" | rg -q '^stderr-visible$'
printf '%s\n' "$CAPTURE" | rg -q '^loop-one$'
printf '%s\n' "$CAPTURE" | rg -q '^loop-two$'
printf '%s\n' "$CAPTURE" | rg -q '^quoted:value with spaces$'
printf '%s\n' "$CAPTURE" | rg -q '^last_status=1$'
printf '%s\n' "$CAPTURE" | rg -q '^after-real-world$'
