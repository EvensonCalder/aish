#!/bin/sh
set -eu

SESSION="aish-manual-real-world-$$"
HOME_DIR="/tmp/aish-tmux-manual-real-home-$$"
WORK_DIR="/tmp/aish-tmux-manual-real-work-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$WORK_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR" "$WORK_DIR"
tmux new-session -d -x 120 -y 50 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

send_command() {
    tmux send-keys -t "$SESSION" C-c
    sleep 1
    tmux send-keys -t "$SESSION" "$1" Enter
    sleep 1
}

send_command "cd $WORK_DIR"
send_command "mkdir -p project/src project/logs"
send_command "printf 'alpha\\nbeta\\n' > project/src/input.txt"
send_command "grep beta project/src/input.txt"
send_command "touch 'file with spaces.txt'"
send_command "test -f 'file with spaces.txt' && echo spaced-file-ok"
send_command 'export AISH_REAL_WORLD=visible'
send_command 'printenv AISH_REAL_WORLD'
send_command "printf 'stderr-visible\\n' >&2"
send_command 'for x in one two; do echo loop-$x; done'
send_command "printf 'quoted:%s\\n' 'value with spaces'"
send_command false
send_command '#status'
send_command 'echo after-real-world'
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
