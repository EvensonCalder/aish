#!/bin/sh
set -eu

SESSION="aish-manual-real-world-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-manual-real-home-$$"
WORK_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-manual-real-work-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$WORK_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR" "$WORK_DIR"
tmux new-session -d -x 120 -y 50 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"

capture_pane() {
    tmux capture-pane -p -S - -t "$SESSION" 2>/dev/null || true
}

wait_for_capture() {
    pattern="$1"
    attempts="${2:-100}"
    attempt=0
    while [ "$attempt" -lt "$attempts" ]; do
        if ! tmux has-session -t "$SESSION" >/dev/null 2>&1; then
            printf 'tmux session exited while waiting for pattern: %s\n' "$pattern" >&2
            return 1
        fi
        CAPTURE="$(capture_pane)"
        if printf '%s\n' "$CAPTURE" | rg -q "$pattern"; then
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 0.2
    done
    printf 'timed out waiting for pattern: %s\n' "$pattern" >&2
    printf '%s\n' "$CAPTURE" >&2
    return 1
}

send_command_and_wait() {
    command="$1"
    expected="$2"
    tmux send-keys -t "$SESSION" C-c
    sleep 0.2
    tmux send-keys -t "$SESSION" "$command" Enter
    wait_for_capture "$expected"
    sleep 0.2
}

send_command_no_wait() {
    command="$1"
    tmux send-keys -t "$SESSION" C-c
    sleep 0.2
    tmux send-keys -t "$SESSION" "$command" Enter
    sleep 0.5
}

wait_for_capture '>[[:space:]]*$' 150
send_command_and_wait "cd $WORK_DIR && echo cwd-ok" '^cwd-ok$'
send_command_and_wait "mkdir -p project/src project/logs && echo mkdir-ok" '^mkdir-ok$'
send_command_and_wait "printf 'alpha\\nbeta\\n' > project/src/input.txt && echo input-ok" '^input-ok$'
send_command_and_wait "grep beta project/src/input.txt" '^beta$'
send_command_and_wait "touch 'file with spaces.txt' && echo touch-ok" '^touch-ok$'
send_command_and_wait "test -f 'file with spaces.txt' && echo spaced-file-ok" '^spaced-file-ok$'
send_command_and_wait 'export AISH_REAL_WORLD=visible && echo export-ok' '^export-ok$'
send_command_and_wait 'printenv AISH_REAL_WORLD' '^visible$'
send_command_and_wait "printf 'stderr-visible\\n' >&2" '^stderr-visible$'
send_command_and_wait 'for x in one two; do echo loop-$x; done' '^loop-two$'
send_command_and_wait "printf 'quoted:%s\\n' 'value with spaces'" '^quoted:value with spaces$'
send_command_no_wait false
send_command_and_wait '#status' '^last_status=1$'
send_command_and_wait 'echo after-real-world' '^after-real-world$'

CAPTURE="$(capture_pane)"
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
