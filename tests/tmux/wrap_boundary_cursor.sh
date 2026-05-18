#!/bin/sh
set -eu

SESSION="aish-wrap-boundary-cursor-$$"
HOME_DIR="/tmp/aish-tmux-wrap-boundary-cursor-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR/.aish"
cat >"$HOME_DIR/.aish/config.toml" <<'EOF'
[prompt]
draft = "> "
history = "$ "
ai = "% "

[completion]
enabled = false
EOF

tmux new-session -d -x 4 -y 10 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
tmux resize-window -t "$SESSION" -x 4 -y 10
sleep 5

tmux send-keys -t "$SESSION" 'abc'
sleep 2

CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
CURSOR="$(tmux display-message -p -t "$SESSION" '#{cursor_x} #{cursor_y}')"

printf '%s\n' "$CAPTURE"
printf 'cursor=%s\n' "$CURSOR"

printf '%s\n' "$CAPTURE" | rg -q '^> ab$'
printf '%s\n' "$CAPTURE" | rg -q '^c$'
printf '%s\n' "$CURSOR" | rg -q '^1 1$'
