#!/bin/sh
set -eu

SESSION="aish-common-shell-$$"
HOME_DIR="/tmp/ah-$$"
WORK_DIR="/tmp/aw-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$WORK_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR/.aish" "$WORK_DIR"
if [ "${AISH_BACKEND_SHELL:-}" ]; then
    printf '[shell]\nbackend = "%s"\n' "$AISH_BACKEND_SHELL" > "$HOME_DIR/.aish/config.toml"
fi

START_DELAY=5
STEP_DELAY=1

case "${AISH_BACKEND_KIND:-posix}" in
    bash)
        CREATE_COMMAND="mkdir -p c; printf 'alpha\\nbeta\\n' > c/i"
        ENV_COMMAND='export AISH_COMMON_VALUE=visible'
        TEST_COMMAND='test -f c/i && echo file-exists'
        BACKEND_COMMAND='printf '\''backend:bash:%s\n'\'' "$BASH_VERSION"'
        ;;
    zsh)
        CREATE_COMMAND="mkdir -p c; printf 'alpha\\nbeta\\n' > c/i"
        ENV_COMMAND='export AISH_COMMON_VALUE=visible'
        TEST_COMMAND='test -f c/i && echo file-exists'
        BACKEND_COMMAND='printf '\''backend:zsh:%s\n'\'' "$ZSH_VERSION"'
        ;;
    fish)
        START_DELAY=7
        STEP_DELAY=2
        CREATE_COMMAND="mkdir -p c; printf 'alpha\\nbeta\\n' > c/i"
        ENV_COMMAND='set -gx AISH_COMMON_VALUE visible'
        TEST_COMMAND='test -f c/i; and echo file-exists'
        BACKEND_COMMAND='printf '\''backend:fish:%s\n'\'' "$version"'
        ;;
    *)
        CREATE_COMMAND="mkdir -p c; printf 'alpha\\nbeta\\n' > c/i"
        ENV_COMMAND='export AISH_COMMON_VALUE=visible'
        TEST_COMMAND='test -f c/i && echo file-exists'
        BACKEND_COMMAND='printf '\''backend:posix\n'\'''
        ;;
esac

tmux new-session -d -x 120 -y 40 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep "$START_DELAY"

send_command() {
    tmux send-keys -t "$SESSION" C-c
    sleep "$STEP_DELAY"
    tmux send-keys -t "$SESSION" "$1" Enter
    sleep "$STEP_DELAY"
}

send_command "cd $WORK_DIR"
send_command "$CREATE_COMMAND"
send_command 'cat c/i | grep beta'
send_command "printf 'quoted:%s\\n' 'value with spaces'"
send_command "$ENV_COMMAND"
send_command 'printenv AISH_COMMON_VALUE'
send_command "$TEST_COMMAND"
send_command "$BACKEND_COMMAND"
send_command 'false'
send_command 'echo after-failure'
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^beta$'
printf '%s\n' "$CAPTURE" | rg -q '^quoted:value with spaces$'
printf '%s\n' "$CAPTURE" | rg -q '^visible$'
printf '%s\n' "$CAPTURE" | rg -q '^file-exists$'
printf '%s\n' "$CAPTURE" | rg -q '^backend:'
printf '%s\n' "$CAPTURE" | rg -q '^after-failure$'
