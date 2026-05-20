#!/bin/sh
set -eu

SESSION="aish-manual-private-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-manual-private-home-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR"
tmux new-session -d -x 120 -y 50 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" '#doctor' Enter
sleep 1
tmux send-keys -t "$SESSION" '#config' Enter
sleep 1
tmux send-keys -t "$SESSION" '#status' Enter
sleep 1
tmux send-keys -t "$SESSION" '#completion' Enter
sleep 1
tmux send-keys -t "$SESSION" '#editor' Enter
sleep 1
tmux send-keys -t "$SESSION" '# NOTE: tmux-note' Enter
sleep 1
tmux send-keys -t "$SESSION" '# TODO: tmux-todo' Enter
sleep 1
tmux send-keys -t "$SESSION" '#key' Enter
sleep 1
tmux send-keys -t "$SESSION" '#key set' Enter
sleep 1
tmux send-keys -t "$SESSION" '#key clear' Enter
sleep 1
tmux send-keys -t "$SESSION" '#encrypt on' Enter
sleep 1
tmux send-keys -t "$SESSION" '#encrypt off' Enter
sleep 1
tmux send-keys -t "$SESSION" '#nosuchmanual' Enter
sleep 1
tmux send-keys -t "$SESSION" 'echo after-private' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
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
printf '%s\n' "$CAPTURE" | rg -q '^usage: #key set \| #key clear$'
printf '%s\n' "$CAPTURE" | rg -q '^encryption key is not configured; run #encrypt on <key-fingerprint>$'
printf '%s\n' "$CAPTURE" | rg -q '^no stored key to clear$'
printf '%s\n' "$CAPTURE" | rg -q '^encryption=off$'
printf '%s\n' "$CAPTURE" | rg -q '^plaintext history and templates will be written from now on$'
printf '%s\n' "$CAPTURE" | rg -q 'not implemented yet: #nosuchmanual'
printf '%s\n' "$CAPTURE" | rg -q '^after-private$'

test -f "$HOME_DIR/.aish/history/notes.jsonl"
rg -q 'tmux-note' "$HOME_DIR/.aish/history/notes.jsonl"
rg -q 'tmux-todo' "$HOME_DIR/.aish/history/notes.jsonl"
