#!/bin/sh
set -eu

SESSION="aish-sudo-password-$$"
HOME_DIR="/tmp/aish-sudo-home-$$"
WORK_DIR="/tmp/aish-sudo-work-$$"
BIN_DIR="/tmp/aish-sudo-bin-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$HOME_DIR" "$WORK_DIR" "$BIN_DIR"' EXIT INT TERM

mkdir -p "$HOME_DIR" "$WORK_DIR" "$BIN_DIR"
cat > "$BIN_DIR/sudo" <<'SCRIPT'
#!/bin/sh
printf '[sudo] password for test: ' > /dev/tty
IFS= read -r password < /dev/tty
printf '\n' > /dev/tty
printf 'fake-sudo-password=%s\n' "$password"
printf 'fake-sudo-command=%s\n' "$*"
SCRIPT
chmod +x "$BIN_DIR/sudo"

tmux new-session -d -x 80 -y 10 -c "$WORK_DIR" -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' PATH='$BIN_DIR':\$PATH '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" 'sudo whoami' Enter
sleep 1

EARLY_CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$EARLY_CAPTURE"

if printf '%s\n' "$EARLY_CAPTURE" | rg -q '^fake-sudo-password='; then
  printf 'sudo consumed queued input before the user typed a password\n' >&2
  exit 1
fi

tmux send-keys -t "$SESSION" 'pw-ok' Enter
sleep 1
tmux send-keys -t "$SESSION" 'echo after-sudo' Enter
sleep 1

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q '^fake-sudo-password=pw-ok$'
printf '%s\n' "$CAPTURE" | rg -q '^fake-sudo-command=whoami$'
printf '%s\n' "$CAPTURE" | rg -q '^after-sudo$'
