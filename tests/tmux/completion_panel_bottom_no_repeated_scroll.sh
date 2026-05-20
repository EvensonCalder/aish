#!/bin/sh
set -eu

SESSION="aish-completion-bottom-scroll-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-cbs-home-$$"
WORK_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-cbs-work-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$WORK_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR" "$WORK_DIR"
touch \
  "$WORK_DIR/alpha-five.txt" \
  "$WORK_DIR/alpha-four.txt" \
  "$WORK_DIR/alpha-one.txt" \
  "$WORK_DIR/alpha-three.txt" \
  "$WORK_DIR/alpha-two.txt"

tmux new-session -d -x 60 -y 5 -c "$WORK_DIR" -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" "printf 'bottom-1\\nbottom-2\\nbottom-3\\nbottom-4\\n'" Enter
sleep 1
tmux send-keys -t "$SESSION" 'cat alpha-'
sleep 1

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^file[[:space:]]+.*\.txt$'

PROMPT_REDRAWS="$(printf '%s\n' "$CAPTURE" | rg -c '> cat alpha')"
if [ "$PROMPT_REDRAWS" -gt 2 ]; then
  printf 'bottom completion leaked %s prompt redraws into scrollback\n' "$PROMPT_REDRAWS" >&2
  exit 1
fi

for name in five four one three two; do
  COUNT="$(printf '%s\n' "$CAPTURE" | rg -c "file[[:space:]]+cat alpha-$name\\.txt$" || true)"
  COUNT="${COUNT:-0}"
  if [ "$COUNT" -gt 1 ]; then
    printf 'bottom completion leaked duplicate candidate alpha-%s.txt (%s times)\n' "$name" "$COUNT" >&2
    exit 1
  fi
done
