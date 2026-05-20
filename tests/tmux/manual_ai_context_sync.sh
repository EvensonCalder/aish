#!/bin/sh
set -eu

SESSION="aish-manual-ai-context-sync-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-manual-ai-home-$$"
MARKER="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-context-disabled-$$"
DANGEROUS_MARKER="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-context-danger-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$MARKER" "$DANGEROUS_MARKER" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
printf 'must remain\n' > "$DANGEROUS_MARKER"
tmux new-session -d -x 120 -y 60 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' AISH_TMUX_CONTEXT_KEY='test-key' GIT_CONFIG_COUNT=3 GIT_CONFIG_KEY_0=commit.gpgsign GIT_CONFIG_VALUE_0=false GIT_CONFIG_KEY_1=user.name GIT_CONFIG_VALUE_1='Aish Tmux' GIT_CONFIG_KEY_2=user.email GIT_CONFIG_VALUE_2=aish@example.invalid '$AISH_BIN'"

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
    attempts="${3:-100}"
    tmux send-keys -t "$SESSION" C-c
    sleep 0.2
    tmux send-keys -t "$SESSION" "$command" Enter
    wait_for_capture "$expected" "$attempts"
    sleep 0.2
}

send_key_and_wait() {
    key="$1"
    expected="$2"
    attempts="${3:-100}"
    tmux send-keys -t "$SESSION" "$key"
    wait_for_capture "$expected" "$attempts"
    sleep 0.2
}

wait_for_capture '>[[:space:]]*$' 150
send_command_and_wait '#model tmux-model' '^#model=tmux-model$'
send_command_and_wait '#base-url https://127.0.0.1:1/v1' '^#base-url=https://127.0.0.1:1/v1/chat/completions$'
send_command_and_wait '#env-key AISH_TMUX_CONTEXT_KEY' '^#env-key=AISH_TMUX_CONTEXT_KEY$'
tmux send-keys -t "$SESSION" C-l
wait_for_capture '>[[:space:]]*$'
send_command_and_wait '#status' '^keybindings=26$'
STATUS_CAPTURE="$(capture_pane)"

send_command_and_wait '#context' '^context.enabled=true context.confirm=true context.max_bytes=65536$'
send_command_and_wait '#context off' '^context.enabled=false context.confirm=true context.max_bytes=65536$'
send_command_and_wait "# explain disabled < touch $MARKER" "context collection is disabled; context command not executed: touch $MARKER"
send_command_and_wait '#context on' '^context.enabled=true context.confirm=true context.max_bytes=65536$'
tmux send-keys -t "$SESSION" C-c
sleep 0.2
tmux send-keys -t "$SESSION" '# summarize this < echo context-output' Enter
wait_for_capture 'Run context command\? \[Y/n\]'
send_key_and_wait n '^context command skipped: echo context-output$'
send_command_and_wait '#context confirm off' '^context.enabled=true context.confirm=false context.max_bytes=65536$'
send_command_and_wait '#context 4' '^context.enabled=true context.confirm=false context.max_bytes=4$'
send_command_and_wait '# explain truncation < printf 123456789' '^AI request failed:' 150
tmux send-keys -t "$SESSION" C-c
sleep 0.2
tmux send-keys -t "$SESSION" "# explain danger < rm -rf $DANGEROUS_MARKER" Enter
wait_for_capture "dangerous context command requires confirmation: rm -rf $DANGEROUS_MARKER"
send_key_and_wait n "context command skipped: rm -rf $DANGEROUS_MARKER"

send_command_and_wait '#set-remote /tmp/nonexistent-aish-remote.git' '^sync.remote=/tmp/nonexistent-aish-remote.git$'
send_command_and_wait '#sync off' '^sync.enabled=false$'
send_command_and_wait '#sync @hourly' '^sync.schedule=@hourly$'
send_command_and_wait '#sync ai on' '^sync.ai=true$'
send_command_and_wait '#sync history on' '^sync.history=true$'
send_command_and_wait '#sync templates on' '^sync.templates=true$'
send_command_and_wait '#sync drafts on' '^sync.drafts=true$'
send_command_and_wait '#sync ai maybe' '^usage: #sync ai\|history\|templates\|drafts on\|off$'
send_command_and_wait '#sync now' '^sync failed: git push -u origin HEAD$' 200
send_command_and_wait 'echo after-ai-context-sync' '^after-ai-context-sync$'

CAPTURE="$(capture_pane)"
printf '%s\n' "$STATUS_CAPTURE"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$STATUS_CAPTURE" | rg -q '^ai.model=tmux-model$'
printf '%s\n' "$STATUS_CAPTURE" | rg -q '^ai.final_url=https://127.0.0.1:1/v1/chat/completions$'
printf '%s\n' "$STATUS_CAPTURE" | rg -q '^ai.key_source=env$'
! printf '%s\n' "$STATUS_CAPTURE" | rg -q 'AISH_TMUX_CONTEXT_KEY'

printf '%s\n' "$CAPTURE" | rg -q 'context.enabled=false'
printf '%s\n' "$CAPTURE" | rg -q "context collection is disabled; context command not executed: touch $MARKER"
test ! -e "$MARKER"
printf '%s\n' "$CAPTURE" | rg -q 'context.enabled=true'
printf '%s\n' "$CAPTURE" | rg -q 'Run context command\? \[Y/n\]'
printf '%s\n' "$CAPTURE" | rg -q '^context command skipped: echo context-output$'
printf '%s\n' "$CAPTURE" | rg -q 'context.confirm=false'
printf '%s\n' "$CAPTURE" | rg -q 'context.max_bytes=4'
printf '%s\n' "$CAPTURE" | rg -q '^context output truncated to 4 bytes$'
printf '%s\n' "$CAPTURE" | rg -q "dangerous context command requires confirmation: rm -rf $DANGEROUS_MARKER"
printf '%s\n' "$CAPTURE" | rg -q "context command skipped: rm -rf $DANGEROUS_MARKER"
test -e "$DANGEROUS_MARKER"
printf '%s\n' "$CAPTURE" | rg -q '^sync.remote=/tmp/nonexistent-aish-remote.git$'
printf '%s\n' "$CAPTURE" | rg -q '^sync.enabled=false$'
printf '%s\n' "$CAPTURE" | rg -q '^sync.schedule=@hourly$'
printf '%s\n' "$CAPTURE" | rg -q '^sync.ai=true$'
printf '%s\n' "$CAPTURE" | rg -q '^sync.history=true$'
printf '%s\n' "$CAPTURE" | rg -q '^sync.templates=true$'
printf '%s\n' "$CAPTURE" | rg -q '^sync.drafts=true$'
printf '%s\n' "$CAPTURE" | rg -q '^usage: #sync ai\|history\|templates\|drafts on\|off$'
printf '%s\n' "$CAPTURE" | rg -q '^sync failed: git push -u origin HEAD$'
printf '%s\n' "$CAPTURE" | rg -q '^after-ai-context-sync$'
test ! -e "$HOME_DIR/.aish/cache/runtime/scheduler"
