#!/usr/bin/env bash
# PLAN 0.4, OS-input variant: includes the real HID → WindowServer → app path and
# compares against standalone Ghostty.app (config a).
#
#   scripts/latency/run-os-input.sh [samples]      default 200
#
# !!! Run this ONLY when nobody is using the Mac (no meetings, no typing). !!!
# It launches apps in the foreground and posts real key events ('x' and
# Backspace) to the focused window for several minutes. osbench stops posting
# the moment the target app is no longer frontmost.
#
# Needs Screen Recording + Accessibility for the terminal you run it from.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SAMPLES="${1:-200}"
B="$HERE/build"
OUT="$B/results-os"
SESSION="azj-latency"
SOCK="$HOME/.config/herdr/sessions/$SESSION/herdr.sock"
HERDR="$(command -v herdr)"
GHOSTTY_APP="/Applications/Ghostty.app"

read -r -p "This posts real keystrokes for ~5 minutes. Nobody using the Mac? [y/N] " ok
[[ "$ok" == "y" ]] || exit 1

unset HERDR_PANE_ID HERDR_WORKSPACE_ID HERDR_TAB_ID HERDR_ENV
export HERDR_SOCKET_PATH="$SOCK"
h() { "$HERDR" --session "$SESSION" "$@"; }

"$HERE/harness/build.sh" >/dev/null
swiftc -O -swift-version 5 -target arm64-apple-macos14.0 "$HERE/osbench.swift" -o "$B/osbench"
mkdir -p "$OUT" "$B/xdg/ghostty" "$B/xdg-a/ghostty"
cat > "$B/xdg/ghostty/config" <<'EOF'
cursor-style-blink = false
shell-integration = none
confirm-close-surface = false
quit-after-last-window-closed = true
window-save-state = never
EOF
{ cat "$B/xdg/ghostty/config"; echo "command = /bin/cat"; echo "window-width = 120"; echo "window-height = 36"; } > "$B/xdg-a/ghostty/config"

bench() { "$B/osbench" --pid "$1" --samples "$SAMPLES" --label "$2" --csv "$OUT/$2.csv" --yes-post-real-keys; }

# (a) standalone Ghostty.app running /bin/cat
open -na "$GHOSTTY_APP" --env XDG_CONFIG_HOME="$B/xdg-a"
sleep 3
PID_A="$(pgrep -n -f "$GHOSTTY_APP/Contents/MacOS/ghostty")"
bench "$PID_A" a-os || true
kill "$PID_A" 2>/dev/null || true

# (b) harness surface running /bin/cat, foreground
XDG_CONFIG_HOME="$B/xdg" "$B/LatHarness.app/Contents/MacOS/LatHarness" --foreground --command /bin/cat 2>/dev/null &
PID_B=$!; sleep 3
bench "$PID_B" b-os || true
kill "$PID_B" 2>/dev/null || true

# (c) harness surface via herdr attach, foreground
if ! h workspace list >/dev/null 2>&1; then
  env -i HOME="$HOME" PATH="/usr/bin:/bin:/usr/sbin:/sbin:$(dirname "$HERDR")" \
    TERM=xterm-256color SHELL=/bin/sh LANG=en_GB.UTF-8 \
    nohup "$HERDR" --session "$SESSION" server >"$B/herdr-server.log" 2>&1 &
  for _ in $(seq 50); do h workspace list >/dev/null 2>&1 && break; sleep 0.1; done
fi
h workspace create --label latency-os --cwd /tmp >/dev/null
sleep 0.5
read -r PANE TERM_ID < <(h pane list | python3 -c 'import json,sys; p=json.load(sys.stdin)["result"]["panes"][-1]; print(p["pane_id"], p["terminal_id"])')
h pane send-text "$PANE" 'clear; exec /bin/cat' >/dev/null
h pane send-keys "$PANE" Enter >/dev/null
sleep 0.5
XDG_CONFIG_HOME="$B/xdg" "$B/LatHarness.app/Contents/MacOS/LatHarness" --foreground \
  --command "$HERDR --session $SESSION terminal attach $TERM_ID --takeover" \
  --env "HERDR_SOCKET_PATH=$SOCK" --env "XDG_CONFIG_HOME=$HOME/.config" 2>/dev/null &
PID_C=$!; sleep 3
bench "$PID_C" c-os || true
kill "$PID_C" 2>/dev/null || true
h server stop >/dev/null 2>&1 || true

python3 "$HERE/summarise.py" "$OUT"
