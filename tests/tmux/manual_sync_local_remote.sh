#!/bin/sh
set -eu

SESSION="aish-manual-sync-local-$$"
ROOT="/tmp/aish-tmux-manual-sync-$$"
REMOTE="$ROOT/remote.git"
SEED="$ROOT/seed"
HOME_DIR="$ROOT/aish-home"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true; rm -rf "$ROOT" || true' EXIT INT TERM

if ! command -v git >/dev/null 2>&1; then
    printf 'git not installed; skipping local sync tmux workflow\n'
    exit 0
fi

mkdir -p "$ROOT" "$SEED"
git init --bare "$REMOTE" >/dev/null
git -C "$SEED" init >/dev/null
git -C "$SEED" config user.name "Aish Tmux"
git -C "$SEED" config user.email "aish@example.invalid"
git -C "$SEED" config commit.gpgsign false
printf 'seed\n' > "$SEED/README.md"
git -C "$SEED" add README.md
git -C "$SEED" commit -m seed >/dev/null
git -C "$SEED" remote add origin "$REMOTE"
git -C "$SEED" push -u origin HEAD >/dev/null
git clone "$REMOTE" "$HOME_DIR/.aish" >/dev/null
git -C "$HOME_DIR/.aish" config user.name "Aish Tmux"
git -C "$HOME_DIR/.aish" config user.email "aish@example.invalid"
git -C "$HOME_DIR/.aish" config commit.gpgsign false

tmux new-session -d -x 120 -y 50 -s "$SESSION" "env HOME='$HOME_DIR' AISH_HOME='$HOME_DIR/.aish' GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=commit.gpgsign GIT_CONFIG_VALUE_0=false '$AISH_BIN'"
sleep 5

tmux send-keys -t "$SESSION" "#set-remote $REMOTE" Enter
sleep 1
tmux send-keys -t "$SESSION" '#push' Enter
sleep 5
tmux send-keys -t "$SESSION" 'echo after-local-sync' Enter
sleep 2

CAPTURE="$(tmux capture-pane -p -S - -t "$SESSION")"
printf '%s\n' "$CAPTURE"

printf '%s\n' "$CAPTURE" | rg -q "^sync.remote=$REMOTE$"
printf '%s\n' "$CAPTURE" | rg -q '^sync step ok: git add -- \.gitattributes \.gitignore SYNC\.md$'
printf '%s\n' "$CAPTURE" | rg -q '^sync step ok: git commit'
printf '%s\n' "$CAPTURE" | rg -q '^sync step ok: git pull --no-rebase --no-edit( origin [^[:space:]]+)?$'
printf '%s\n' "$CAPTURE" | rg -q '^sync step ok: git push'
printf '%s\n' "$CAPTURE" | rg -q '^sync push completed$'
printf '%s\n' "$CAPTURE" | rg -q '^after-local-sync$'

git --git-dir "$REMOTE" show HEAD:.gitignore | rg -q '# BEGIN AISH MANAGED'
git --git-dir "$REMOTE" show HEAD:.gitattributes | rg -q 'merge=union'
git --git-dir "$REMOTE" show HEAD:SYNC.md | rg -q 'Aish Sync Repository'
test ! -e "$HOME_DIR/.aish/cache/runtime/scheduler"
