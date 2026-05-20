#!/bin/sh
set -eu

SESSION="aish-rc-inheritance-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-rc-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
: "${AISH_BACKEND_SHELL:?AISH_BACKEND_SHELL must name the backend shell under test}"
: "${AISH_BACKEND_KIND:?AISH_BACKEND_KIND must name the backend shell kind}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR/.aish" "$HOME_DIR/bin"
{
    printf '[shell]\nbackend = "%s"\n' "$AISH_BACKEND_SHELL"
    printf '[completion]\ninline = false\n'
} > "$HOME_DIR/.aish/config.toml"

write_executable() {
    path="$1"
    command="$2"
    {
        printf '%s\n' '#!/bin/sh'
        printf '%s\n' "$command"
    } > "$path"
    chmod +x "$path"
}

case "$AISH_BACKEND_KIND" in
    bash)
        write_executable "$HOME_DIR/bin/from-aish-tmux-bashrc-path" "printf 'path-from-bashrc\n'"
        {
            printf '%s\n' '[ -z "$PS1" ] && return'
            printf '%s\n' "alias aish_tmux_alias_from_rc='printf alias-from-bashrc\\\\n'"
            printf '%s\n' "aish_tmux_function_from_rc() { printf 'function-from-bashrc\\n'; }"
            printf '%s\n' 'export AISH_TMUX_BASH_ENV=env-from-bashrc'
            printf 'export PATH="%s:$PATH"\n' "$HOME_DIR/bin"
            printf '%s\n' "PS1='bashrc-prompt> '"
            printf '%s\n' "PROMPT_COMMAND='export AISH_TMUX_BASH_PROMPT_COMMAND=ran; printf bash-prompt-noise\\\\n'"
        } > "$HOME_DIR/.bashrc"
        FORBIDDEN='bash-prompt-noise|bashrc-prompt|__AISH_STATUS__|__AISH_READY__'
        ;;
    zsh)
        write_executable "$HOME_DIR/bin/from-aish-tmux-zshrc-path" "printf 'path-from-zshrc\n'"
        {
            printf '%s\n' "alias aish_tmux_alias_from_zshrc='printf alias-from-zshrc\\\\n'"
            printf '%s\n' "aish_tmux_function_from_zshrc() { printf 'function-from-zshrc\\n'; }"
            printf '%s\n' 'aish_tmux_user_preexec() { export AISH_TMUX_ZSH_PREEXEC="$1"; }'
            printf '%s\n' 'aish_tmux_user_precmd() { export AISH_TMUX_ZSH_PRECMD=ran; printf zsh-precmd-noise\\n; }'
            printf '%s\n' 'autoload -Uz add-zsh-hook'
            printf '%s\n' 'add-zsh-hook preexec aish_tmux_user_preexec'
            printf '%s\n' 'add-zsh-hook precmd aish_tmux_user_precmd'
            printf '%s\n' 'export AISH_TMUX_ZSH_ENV=env-from-zshrc'
            printf 'export PATH="%s:$PATH"\n' "$HOME_DIR/bin"
            printf '%s\n' "PROMPT='zshrc-prompt> '"
        } > "$HOME_DIR/.zshrc"
        FORBIDDEN='zsh-precmd-noise|zshrc-prompt|__AISH_STATUS__|__AISH_READY__'
        ;;
    fish)
        mkdir -p "$HOME_DIR/.config/fish"
        write_executable "$HOME_DIR/bin/from-aish-tmux-fish-config-path" "printf 'path-from-fish-config\n'"
        {
            printf '%s\n' 'function aish_tmux_function_from_fish_config'
            printf '%s\n' "    printf 'function-from-fish-config\\n'"
            printf '%s\n' 'end'
            printf '%s\n' 'set -gx AISH_TMUX_FISH_ENV env-from-fish-config'
            printf 'set -gx PATH %s $PATH\n' "$HOME_DIR/bin"
            printf '%s\n' 'function aish_tmux_user_fish_preexec --on-event fish_preexec'
            printf '%s\n' '    set -gx AISH_TMUX_FISH_PREEXEC $argv[1]'
            printf '%s\n' 'end'
            printf '%s\n' 'function aish_tmux_user_fish_postexec --on-event fish_postexec'
            printf '%s\n' '    set -gx AISH_TMUX_FISH_POSTEXEC ran'
            printf '%s\n' 'end'
            printf '%s\n' 'function fish_prompt'
            printf '%s\n' "    printf 'fish-config-prompt> '"
            printf '%s\n' 'end'
        } > "$HOME_DIR/.config/fish/config.fish"
        FORBIDDEN='fish-config-prompt|__AISH_STATUS__|__AISH_READY__'
        ;;
    *)
        printf 'unsupported backend kind: %s\n' "$AISH_BACKEND_KIND" >&2
        exit 2
        ;;
esac

tmux new-session -d -x 120 -y 40 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"

capture_pane() {
    tmux capture-pane -p -S - -t "$SESSION"
}

wait_for_capture() {
    pattern="$1"
    attempts="${2:-50}"
    attempt=0
    while [ "$attempt" -lt "$attempts" ]; do
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
}

wait_for_capture '>[[:space:]]*$'

case "$AISH_BACKEND_KIND" in
    bash)
        send_command_and_wait 'aish_tmux_alias_from_rc' '^alias-from-bashrc$'
        send_command_and_wait 'aish_tmux_function_from_rc' '^function-from-bashrc$'
        send_command_and_wait 'printf "env:%s\n" "$AISH_TMUX_BASH_ENV"' '^env:env-from-bashrc$'
        send_command_and_wait 'from-aish-tmux-bashrc-path' '^path-from-bashrc$'
        send_command_and_wait 'printf "prompt-command:%s\n" "$AISH_TMUX_BASH_PROMPT_COMMAND"' '^prompt-command:ran$'
        ;;
    zsh)
        send_command_and_wait 'aish_tmux_alias_from_zshrc' '^alias-from-zshrc$'
        send_command_and_wait 'aish_tmux_function_from_zshrc' '^function-from-zshrc$'
        send_command_and_wait 'printf "env:%s\n" "$AISH_TMUX_ZSH_ENV"' '^env:env-from-zshrc$'
        send_command_and_wait 'from-aish-tmux-zshrc-path' '^path-from-zshrc$'
        send_command_and_wait 'printf "hooks:%s|%s\n" "$AISH_TMUX_ZSH_PRECMD" "$AISH_TMUX_ZSH_PREEXEC"' '^hooks:ran\|printf'
        ;;
    fish)
        send_command_and_wait 'aish_tmux_function_from_fish_config' '^function-from-fish-config$'
        send_command_and_wait 'printf "env:%s\n" $AISH_TMUX_FISH_ENV' '^env:env-from-fish-config$'
        send_command_and_wait 'from-aish-tmux-fish-config-path' '^path-from-fish-config$'
        send_command_and_wait 'printf "events:%s|%s\n" $AISH_TMUX_FISH_POSTEXEC $AISH_TMUX_FISH_PREEXEC' '^events:ran\|printf'
        ;;
esac

CAPTURE="$(capture_pane)"
printf '%s\n' "$CAPTURE"

case "$AISH_BACKEND_KIND" in
    bash)
        printf '%s\n' "$CAPTURE" | rg -q '^alias-from-bashrc$'
        printf '%s\n' "$CAPTURE" | rg -q '^function-from-bashrc$'
        printf '%s\n' "$CAPTURE" | rg -q '^env:env-from-bashrc$'
        printf '%s\n' "$CAPTURE" | rg -q '^path-from-bashrc$'
        printf '%s\n' "$CAPTURE" | rg -q '^prompt-command:ran$'
        ;;
    zsh)
        printf '%s\n' "$CAPTURE" | rg -q '^alias-from-zshrc$'
        printf '%s\n' "$CAPTURE" | rg -q '^function-from-zshrc$'
        printf '%s\n' "$CAPTURE" | rg -q '^env:env-from-zshrc$'
        printf '%s\n' "$CAPTURE" | rg -q '^path-from-zshrc$'
        printf '%s\n' "$CAPTURE" | rg -q '^hooks:ran\|printf'
        ;;
    fish)
        printf '%s\n' "$CAPTURE" | rg -q '^function-from-fish-config$'
        printf '%s\n' "$CAPTURE" | rg -q '^env:env-from-fish-config$'
        printf '%s\n' "$CAPTURE" | rg -q '^path-from-fish-config$'
        printf '%s\n' "$CAPTURE" | rg -q '^events:ran\|printf'
        ;;
esac
! printf '%s\n' "$CAPTURE" | rg -q "$FORBIDDEN"
