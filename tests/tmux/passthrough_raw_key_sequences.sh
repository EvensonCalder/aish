#!/bin/sh
set -eu

SESSION="aish-raw-key-$$"
HOME_DIR="/tmp/aish-tmux-raw-key-home-$$"
BIN_DIR="/tmp/aish-tmux-raw-key-bin-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$BIN_DIR" || true' EXIT INT TERM

mkdir -p "$HOME_DIR" "$BIN_DIR"
cat > "$BIN_DIR/aish-raw-key-reader" <<'SCRIPT'
#!/bin/sh
saved_tty="$(stty -g)"
cleanup() {
    stty "$saved_tty" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM
stty raw -echo min 0 time 10
printf 'raw-key-ready\r\n'
bytes="$(dd bs=32 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n')"
cleanup
trap - EXIT INT TERM
printf 'raw-key-hex:%s\n' "$bytes"
SCRIPT
chmod +x "$BIN_DIR/aish-raw-key-reader"

tmux new-session -d -x 120 -y 30 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' PATH='$BIN_DIR:$PATH' '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'aish-raw-key-reader' Enter
sleep 1
tmux send-keys -t "$SESSION" F1
sleep 2
tmux send-keys -t "$SESSION" 'echo after-raw-key' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^raw-key-hex:1b'
printf '%s\n' "$CAPTURE" | rg -q '^after-raw-key$'
