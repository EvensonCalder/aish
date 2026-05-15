#!/bin/sh
set -eu

SESSION="aish-manual-completion-$$"
HOME_DIR="/tmp/aish-tmux-manual-completion-home-$$"
WORK_DIR="/tmp/aish-tmux-manual-completion-work-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$WORK_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR" "$WORK_DIR/src"
printf 'unique-content\n' > "$WORK_DIR/unique-target.txt"
printf 'main-content\n' > "$WORK_DIR/src/main.rs"
touch "$WORK_DIR/alpha-one.txt" "$WORK_DIR/alpha-two.txt" "$WORK_DIR/alpha-three.txt"
touch "$WORK_DIR/very-long-aish-completion-candidate-name-that-needs-elision.txt"

tmux new-session -d -x 120 -y 50 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" "cd $WORK_DIR" Enter
sleep 1

tmux send-keys -t "$SESSION" 'cat unique-tar'
sleep 1
PANEL_ONE="$(tmux capture-pane -p -t "$SESSION")"
tmux send-keys -t "$SESSION" Tab
sleep 1
tmux send-keys -t "$SESSION" Enter
sleep 2

tmux send-keys -t "$SESSION" '#completion max 1' Enter
sleep 1
tmux send-keys -t "$SESSION" 'cat alpha-'
sleep 1
PANEL_MAX_ONE="$(tmux capture-pane -p -t "$SESSION")"
tmux send-keys -t "$SESSION" Escape
sleep 1

tmux send-keys -t "$SESSION" '#completion max 0' Enter
sleep 1
tmux send-keys -t "$SESSION" '#completion inline maybe' Enter
sleep 1
tmux send-keys -t "$SESSION" '#completion tab-accept line' Enter
sleep 1

tmux send-keys -t "$SESSION" '#completion inline off' Enter
sleep 1
tmux send-keys -t "$SESSION" 'cat unique-tar'
tmux send-keys -t "$SESSION" Tab
sleep 1
tmux send-keys -t "$SESSION" Enter
sleep 2

tmux send-keys -t "$SESSION" '#completion inline on' Enter
sleep 1
tmux send-keys -t "$SESSION" '#completion tab-accept word' Enter
sleep 1
tmux send-keys -t "$SESSION" 'echo tmuxword-one tmuxword-two' Enter
sleep 1
tmux send-keys -t "$SESSION" 'echo tmux'
sleep 1
PANEL_WORD="$(tmux capture-pane -p -t "$SESSION")"
tmux send-keys -t "$SESSION" Tab
sleep 1
tmux send-keys -t "$SESSION" Enter
sleep 2

tmux send-keys -t "$SESSION" '#completion tab-accept full' Enter
sleep 1
tmux send-keys -t "$SESSION" 'cat src/m'
tmux send-keys -t "$SESSION" Tab
sleep 1
tmux send-keys -t "$SESSION" Enter
sleep 2

WIDE_CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"

tmux resize-window -t "$SESSION" -x 44 -y 30
sleep 1
tmux send-keys -t "$SESSION" 'cat very-long-aish'
sleep 1
NARROW_PANEL="$(tmux capture-pane -p -t "$SESSION")"
tmux send-keys -t "$SESSION" Escape
sleep 1

printf '%s\n' "$PANEL_ONE"
printf '%s\n' "$PANEL_MAX_ONE"
printf '%s\n' "$PANEL_WORD"
printf '%s\n' "$WIDE_CAPTURE"
printf '%s\n' "$NARROW_PANEL"

printf '%s\n' "$PANEL_ONE" | rg -q 'unique-target.txt'
printf '%s\n' "$WIDE_CAPTURE" | rg -q '^unique-content$'

MAX_COUNT="$(printf '%s\n' "$PANEL_MAX_ONE" | rg '^(file|history|template|exec)[[:space:]]+' | wc -l | tr -d ' ')"
test "$MAX_COUNT" = "1"

printf '%s\n' "$WIDE_CAPTURE" | rg -q '^completion max results must be greater than 0$'
printf '%s\n' "$WIDE_CAPTURE" | rg -q '^usage: #completion inline on\|off$'
printf '%s\n' "$WIDE_CAPTURE" | rg -q '^usage: #completion tab-accept full\|word$'
printf '%s\n' "$WIDE_CAPTURE" | rg -q '^completion.inline=false$'
printf '%s\n' "$WIDE_CAPTURE" | rg -q '^completion.tab_accept=word$'
printf '%s\n' "$PANEL_WORD" | rg -q 'tmuxword-one tmuxword-two'
printf '%s\n' "$WIDE_CAPTURE" | rg -q '^tmuxword-one$'
printf '%s\n' "$WIDE_CAPTURE" | rg -q '^main-content$'
printf '%s\n' "$NARROW_PANEL" | rg -q '\.\.\.'
printf '%s\n' "$NARROW_PANEL" | awk 'length($0) > 44 { exit 1 }'
