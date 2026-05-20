#!/bin/sh
set -eu

ROOT="${AISH_TMUX_ARTIFACT_DIR:-/tmp}/aish-tmux-manual-startup-$$"
: "${AISH_BIN:?AISH_BIN must point to the aish binary under test}"
trap 'tmux kill-session -t aish-manual-startup-relative-$$ >/dev/null 2>&1 || true; tmux kill-session -t aish-manual-startup-invalid-$$ >/dev/null 2>&1 || true; tmux kill-session -t aish-manual-startup-file-$$ >/dev/null 2>&1 || true; rm -rf "$ROOT" || true' EXIT INT TERM

mkdir -p "$ROOT"

tmux new-session -d -x 120 -y 20 -s "aish-manual-startup-relative-$$" "env HOME='$ROOT/home-relative' AISH_HOME='relative-aish-home' '$AISH_BIN'; printf '\naish-exit:%s\n' \"\$?\"; sleep 10"
sleep 2
RELATIVE_CAPTURE="$(tmux capture-pane -p -S - -t "aish-manual-startup-relative-$$")"

mkdir -p "$ROOT/home-invalid/.aish"
printf 'invalid toml = [\n' > "$ROOT/home-invalid/.aish/config.toml"
tmux new-session -d -x 120 -y 20 -s "aish-manual-startup-invalid-$$" "env HOME='$ROOT/home-invalid' AISH_HOME='$ROOT/home-invalid/.aish' '$AISH_BIN'; printf '\naish-exit:%s\n' \"\$?\"; sleep 10"
sleep 2
INVALID_CAPTURE="$(tmux capture-pane -p -S - -t "aish-manual-startup-invalid-$$")"

mkdir -p "$ROOT/home-file"
printf 'not a directory\n' > "$ROOT/not-a-directory"
tmux new-session -d -x 120 -y 20 -s "aish-manual-startup-file-$$" "env HOME='$ROOT/home-file' AISH_HOME='$ROOT/not-a-directory' '$AISH_BIN'; printf '\naish-exit:%s\n' \"\$?\"; sleep 10"
sleep 2
FILE_CAPTURE="$(tmux capture-pane -p -S - -t "aish-manual-startup-file-$$")"

printf '%s\n' "$RELATIVE_CAPTURE"
printf '%s\n' "$INVALID_CAPTURE"
printf '%s\n' "$FILE_CAPTURE"

printf '%s\n' "$RELATIVE_CAPTURE" | rg -q 'AISH_HOME must be set to an absolute path'
printf '%s\n' "$RELATIVE_CAPTURE" | rg -q '^aish-exit:1$'
printf '%s\n' "$INVALID_CAPTURE" | rg -q 'invalid config'
printf '%s\n' "$INVALID_CAPTURE" | rg -q '^aish-exit:1$'
printf '%s\n' "$FILE_CAPTURE" | rg -q 'failed to create directory'
printf '%s\n' "$FILE_CAPTURE" | rg -q '^aish-exit:1$'
