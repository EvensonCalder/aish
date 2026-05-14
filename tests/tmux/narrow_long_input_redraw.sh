#!/bin/sh
set -eu

SESSION="aish-narrow-long-input-$$"
HOME_DIR="/tmp/aish-tmux-narrow-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR/.aish"
cat >"$HOME_DIR/.aish/config.toml" <<'EOF'
[prompt]
draft = "> "
history = "$ "
ai = "% "
EOF

tmux new-session -d -x 52 -y 20 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'echo very-long-completion-candidate-alpha-beta-gamma-delta-epsilon-zeta'
sleep 2
LONG_CAPTURE="$(tmux capture-pane -p -t "$SESSION")"
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo after-narrow-redraw' Enter
sleep 2
CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"

printf '%s\n' "$LONG_CAPTURE"
printf '%s\n' "$CAPTURE"

! printf '%s\n' "$LONG_CAPTURE" | rg -q '> .* > '
printf '%s\n' "$CAPTURE" | rg -q '^after-narrow-redraw$'
