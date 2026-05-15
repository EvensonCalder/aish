#!/bin/sh
set -eu

SESSION_ONE="aish-manual-template-one-$$"
SESSION_TWO="aish-manual-template-two-$$"
HOME_DIR="/tmp/aish-tmux-manual-template-home-$$"
EDITOR_SCRIPT="/tmp/aish-tmux-manual-editor-$$.sh"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION_ONE" >/dev/null 2>&1 || true; tmux kill-session -t "$SESSION_TWO" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$EDITOR_SCRIPT"' EXIT INT TERM

mkdir -p "$HOME_DIR"
printf '#!/bin/sh\nprintf '\''echo editor-tmux-ok\\n'\'' > "$1"\n' > "$EDITOR_SCRIPT"
chmod +x "$EDITOR_SCRIPT"

tmux new-session -d -x 120 -y 50 -s "$SESSION_ONE" "env HOME='$HOME_DIR' EDITOR='$EDITOR_SCRIPT' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION_ONE" '#config' Enter
sleep 1
tmux send-keys -t "$SESSION_ONE" '#mt echo template-tmux-ok' Enter
sleep 1
TEMPLATE_ID="$(tmux capture-pane -p -S - -t "$SESSION_ONE" | awk '/^template stored: tpl-/ { id=$3 } END { print id }')"
test -n "$TEMPLATE_ID"
tmux send-keys -t "$SESSION_ONE" '#template list' Enter
sleep 1
tmux send-keys -t "$SESSION_ONE" "#template find template-tmux-ok" Enter
sleep 1
tmux send-keys -t "$SESSION_ONE" "#template show $TEMPLATE_ID" Enter
sleep 1
tmux send-keys -t "$SESSION_ONE" "#template use $TEMPLATE_ID" Enter
sleep 1
tmux send-keys -t "$SESSION_ONE" Enter
sleep 2
tmux send-keys -t "$SESSION_ONE" C-c
sleep 1
tmux send-keys -t "$SESSION_ONE" '#mt echo {message}' Enter
sleep 1
UNRESOLVED_ID="$(tmux capture-pane -p -S - -t "$SESSION_ONE" | awk '/^template stored: tpl-/ { id=$3 } END { print id }')"
test -n "$UNRESOLVED_ID"
tmux send-keys -t "$SESSION_ONE" "#template use $UNRESOLVED_ID" Enter
sleep 1
tmux send-keys -t "$SESSION_ONE" Enter
sleep 1
tmux send-keys -t "$SESSION_ONE" C-c
sleep 1
tmux send-keys -t "$SESSION_ONE" "#template replace $TEMPLATE_ID echo replaced-template-tmux" Enter
sleep 1
REPLACED_ID="$(tmux capture-pane -p -S - -t "$SESSION_ONE" | awk '/^template replaced: tpl-/ { id=$5 } END { print id }')"
test -n "$REPLACED_ID"
tmux send-keys -t "$SESSION_ONE" "#template show $REPLACED_ID" Enter
sleep 1
tmux send-keys -t "$SESSION_ONE" 'echo default-history-tmux' Enter
sleep 1
tmux send-keys -t "$SESSION_ONE" C-c
sleep 1
tmux send-keys -t "$SESSION_ONE" C-x C-e
sleep 2
tmux send-keys -t "$SESSION_ONE" Enter
sleep 2

CAPTURE_ONE="$(tmux capture-pane -p -S - -t "$SESSION_ONE")"
printf '%s\n' "$CAPTURE_ONE"

printf '%s\n' "$CAPTURE_ONE" | rg -q "$HOME_DIR/.aish"
printf '%s\n' "$CAPTURE_ONE" | rg -q '^template stored: tpl-[0-9a-f]+$'
printf '%s\n' "$CAPTURE_ONE" | rg -q '^template listing is intentionally not supported'
printf '%s\n' "$CAPTURE_ONE" | rg -q '^template tpl-[0-9a-f]+.*template-tmux-ok$'
printf '%s\n' "$CAPTURE_ONE" | rg -q '^template copied to draft: tpl-[0-9a-f]+$'
printf '%s\n' "$CAPTURE_ONE" | rg -q '^template-tmux-ok$'
printf '%s\n' "$CAPTURE_ONE" | rg -q '^cannot execute unresolved template placeholders: message$'
printf '%s\n' "$CAPTURE_ONE" | rg -q '^template replaced: tpl-[0-9a-f]+ -> tpl-[0-9a-f]+'
printf '%s\n' "$CAPTURE_ONE" | rg -q '^echo replaced-template-tmux$'
printf '%s\n' "$CAPTURE_ONE" | rg -q '^default-history-tmux$'
printf '%s\n' "$CAPTURE_ONE" | rg -q '^editor-tmux-ok$'

test -d "$HOME_DIR/.aish"
rg -q 'default-history-tmux' "$HOME_DIR/.aish/history/regular.jsonl"
rg -q 'replaced-template-tmux' "$HOME_DIR/.aish/templates/templates.jsonl"

tmux send-keys -t "$SESSION_ONE" '#exit' Enter
sleep 2

tmux new-session -d -x 120 -y 35 -s "$SESSION_TWO" "env HOME='$HOME_DIR' EDITOR='$EDITOR_SCRIPT' '$AISH_BIN'"
sleep 5
tmux send-keys -t "$SESSION_TWO" C-c
sleep 1
tmux send-keys -t "$SESSION_TWO" '#template find replaced-template' Enter
sleep 1
tmux send-keys -t "$SESSION_TWO" Tab
sleep 1
tmux send-keys -t "$SESSION_TWO" Up
sleep 1

CAPTURE_TWO="$(tmux capture-pane -p -S - -t "$SESSION_TWO")"
printf '%s\n' "$CAPTURE_TWO"

printf '%s\n' "$CAPTURE_TWO" | rg -q 'replaced-template-tmux'
printf '%s\n' "$CAPTURE_TWO" | rg -q 'default-history-tmux'
