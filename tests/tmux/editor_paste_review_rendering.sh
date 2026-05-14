#!/bin/sh
set -eu

SESSION="aish-editor-paste-render-$$"
HOME_DIR="/tmp/aish-tmux-editor-paste-home-$$"
EDITOR_SCRIPT="/tmp/aish-tmux-editor-paste-$$.sh"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$EDITOR_SCRIPT"' EXIT INT TERM

mkdir -p "$HOME_DIR"
printf '#!/bin/sh\nprintf '\''echo edited-by-tmux\\n'\'' > "$1"\n' > "$EDITOR_SCRIPT"
chmod +x "$EDITOR_SCRIPT"

tmux new-session -d -x 120 -y 50 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' EDITOR='$EDITOR_SCRIPT' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" C-x C-e
sleep 2
EDITOR_REVIEW="$(tmux capture-pane -p -t "$SESSION")"
tmux send-keys -t "$SESSION" Enter
sleep 2

tmux send-keys -t "$SESSION" Escape
sleep 1
tmux send-keys -l -t "$SESSION" "$(printf '\033[200~echo single-line-paste-tmux\n\033[201~')"
sleep 1
SINGLE_PASTE_REVIEW="$(tmux capture-pane -p -t "$SESSION")"
tmux send-keys -t "$SESSION" Enter
sleep 2

tmux send-keys -l -t "$SESSION" "$(printf '\033[200~echo multi-one\necho multi-two\n\033[201~')"
sleep 1
MULTI_PASTE_REVIEW="$(tmux capture-pane -p -t "$SESSION")"
tmux send-keys -t "$SESSION" Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$EDITOR_REVIEW"
printf '%s\n' "$SINGLE_PASTE_REVIEW"
printf '%s\n' "$MULTI_PASTE_REVIEW"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$EDITOR_REVIEW" | rg -q '\[draft: 1 line, [0-9]+ bytes; Enter run, Ctrl-X Ctrl-E edit\]'
! printf '%s\n' "$EDITOR_REVIEW" | rg -q '2 line'
! printf '%s\n' "$SINGLE_PASTE_REVIEW" | rg -q '\[draft:'
printf '%s\n' "$SINGLE_PASTE_REVIEW" | rg -q 'echo single-line-paste-tmux'
printf '%s\n' "$MULTI_PASTE_REVIEW" | rg -q '\[draft: 2 lines, [0-9]+ bytes; Enter run, Ctrl-X Ctrl-E edit\]'
printf '%s\n' "$CAPTURE" | rg -q '^edited-by-tmux$'
printf '%s\n' "$CAPTURE" | rg -q '^single-line-paste-tmux$'
printf '%s\n' "$CAPTURE" | rg -q '^multi-one$'
printf '%s\n' "$CAPTURE" | rg -q '^multi-two$'
! printf '%s\n' "$CAPTURE" | rg -q "^' 'echo"
