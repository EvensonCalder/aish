#!/bin/sh
set -eu

SESSION="aish-manual-private-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-manual-private-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
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

wait_for_capture '>[[:space:]]*$' 150
send_command_and_wait '#doctor' '^Aish doctor$'
send_command_and_wait '#config' '^Aish config$'
send_command_and_wait '#status' '^Aish status$'
send_command_and_wait '#completion' '^completion.tab_accept=word$'
send_command_and_wait '#editor' '^Aish editor$'
send_command_and_wait '#TODO: tmux-hash-prefix' 'unknown Aish command: #TODO:'
send_command_and_wait '#key' '^usage: #key set \| #key clear$'
send_command_and_wait '#key set' '^encryption key is not configured; run #encrypt on <key-fingerprint>$'
send_command_and_wait '#key clear' '^no stored key to clear$'
send_command_and_wait '#encrypt on' '^encryption key is not configured; run #encrypt on <key-fingerprint>$'
send_command_and_wait '#encrypt off' '^plaintext history and templates will be written from now on$'
send_command_and_wait '#nosuchmanual' 'unknown Aish command: #nosuchmanual'
send_command_and_wait 'echo after-private' '^after-private$'

CAPTURE="$(capture_pane)"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^Aish doctor$'
printf '%s\n' "$CAPTURE" | rg -q '^Aish config$'
printf '%s\n' "$CAPTURE" | rg -q '^Aish status$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.enabled=true$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.mode=auto$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.coalesce_ms=50$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.display_delay_ms=120$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.inline=true$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.fuzzy=true$'
printf '%s\n' "$CAPTURE" | rg -q '^completion.tab_accept=word$'
printf '%s\n' "$CAPTURE" | rg -q 'unknown Aish command: #TODO:'
printf '%s\n' "$CAPTURE" | rg -q '^usage: #key set \| #key clear$'
printf '%s\n' "$CAPTURE" | rg -q '^encryption key is not configured; run #encrypt on <key-fingerprint>$'
printf '%s\n' "$CAPTURE" | rg -q '^no stored key to clear$'
printf '%s\n' "$CAPTURE" | rg -q '^encryption=off$'
printf '%s\n' "$CAPTURE" | rg -q '^plaintext history and templates will be written from now on$'
printf '%s\n' "$CAPTURE" | rg -q 'unknown Aish command: #nosuchmanual'
printf '%s\n' "$CAPTURE" | rg -q '^after-private$'
