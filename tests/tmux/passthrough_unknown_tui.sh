#!/bin/sh
set -eu

SESSION="aish-unknown-tui-$$"
HOME_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-unknown-tui-home-$$"
BIN_DIR="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-unknown-tui-bin-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$BIN_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR" "$BIN_DIR"
cat > "$BIN_DIR/aish-unknown-tui" <<'SCRIPT'
#!/bin/sh
saved_tty="$(stty -g)"
cleanup() {
    printf '\033[?1049l'
    stty "$saved_tty" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM
stty raw -echo
printf '\033[?1049hunknown-tui-ready\r\n'
key="$(dd bs=1 count=1 2>/dev/null)"
cleanup
trap - EXIT INT TERM
printf 'unknown-tui-key:%s\n' "$key"
SCRIPT
chmod +x "$BIN_DIR/aish-unknown-tui"

tmux new-session -d -x 120 -y 40 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' PATH='$BIN_DIR:$PATH' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'aish-unknown-tui' Enter
for _ in 1 2 3 4 5 6 7 8 9 10; do
    if tmux capture-pane -p -t "$SESSION" | rg -q 'unknown-tui-ready'; then
        break
    fi
    sleep 0.2
done
tmux send-keys -t "$SESSION" x
sleep 2
tmux send-keys -t "$SESSION" C-c
sleep 1
tmux send-keys -t "$SESSION" 'echo after-unknown-tui' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q 'unknown-tui-key:x'
printf '%s\n' "$CAPTURE" | rg -q '^after-unknown-tui$'
