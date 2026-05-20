#!/bin/sh
set -eu

SESSION="aish-stdin-recovery-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-stdin-home-$$"
BIN_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-stdin-bin-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$BIN_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR" "$BIN_DIR"
cat > "$BIN_DIR/aish-stdin-blocker" <<'SCRIPT'
#!/bin/sh
printf 'stdin-blocker-ready\n'
cat >/dev/null
printf 'stdin-blocker-exit\n'
SCRIPT
chmod +x "$BIN_DIR/aish-stdin-blocker"

tmux new-session -d -x 120 -y 40 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' PATH='$BIN_DIR:$PATH' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'aish-stdin-blocker' Enter
sleep 1
tmux send-keys -t "$SESSION" C-d
sleep 2
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo after-stdin-blocker' Enter
sleep 2

if command -v gpg >/dev/null 2>&1; then
    tmux send-keys -t "$SESSION" C-c
    sleep 1
    tmux send-keys -t "$SESSION" 'gpg' Enter
    sleep 1
    tmux send-keys -t "$SESSION" C-c
    sleep 2
    tmux send-keys -t "$SESSION" C-c
    sleep 1
    tmux send-keys -t "$SESSION" 'echo after-gpg' Enter
    sleep 2
else
    tmux send-keys -t "$SESSION" C-c
    sleep 1
    tmux send-keys -t "$SESSION" 'echo after-gpg' Enter
    sleep 2
fi

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^stdin-blocker-ready$'
printf '%s\n' "$CAPTURE" | rg -q '^after-stdin-blocker$'
printf '%s\n' "$CAPTURE" | rg -q '^after-gpg$'
