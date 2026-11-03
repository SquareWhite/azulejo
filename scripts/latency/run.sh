#!/usr/bin/env bash
# PLAN 0.4 latency bench, in-process mode (safe while you're using the Mac).
#
#   scripts/latency/run.sh [samples] [rounds]     default 250 samples, 2 rounds
#
# Configs, same libghostty surface and Ghostty config each time:
#   b  surface runs /bin/cat directly (no herdr)
#   c  surface runs `herdr terminal attach` to a herdr pane running /bin/cat
#
# Keys are injected in-process (ghostty_surface_key); the harness window sits
# behind everything, never activates and is captured window-only. Nothing is
# posted to the OS input system. Results: scripts/latency/build/results/*.csv
#
# The standalone Ghostty.app comparison (a) needs real OS input and focus:
# see run-os-input.sh, to run only when the Mac is free.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SAMPLES="${1:-250}"
ROUNDS="${2:-2}"
B="$HERE/build"
APP="$B/LatHarness.app/Contents/MacOS/LatHarness"
OUT="$B/results"
SESSION="azj-latency"
SOCK="$HOME/.config/herdr/sessions/$SESSION/herdr.sock"
HERDR="$(command -v herdr)"

# Never talk to the user's live herdr session.
unset HERDR_PANE_ID HERDR_WORKSPACE_ID HERDR_TAB_ID HERDR_ENV
export HERDR_SOCKET_PATH="$SOCK"
h() { "$HERDR" --session "$SESSION" "$@"; }

"$HERE/harness/build.sh" >/dev/null
mkdir -p "$OUT" "$B/xdg/ghostty"
# Identical Ghostty config for every config: no cursor blink, no shell integration.
cat > "$B/xdg/ghostty/config" <<'EOF'
cursor-style-blink = false
shell-integration = none
confirm-close-surface = false
quit-after-last-window-closed = true
window-save-state = never
EOF

# herdr server with a clean env (panes inherit the server's env) and /bin/sh.
if ! h workspace list >/dev/null 2>&1; then
  env -i HOME="$HOME" PATH="/usr/bin:/bin:/usr/sbin:/sbin:$(dirname "$HERDR")" \
    TERM=xterm-256color SHELL=/bin/sh LANG=en_GB.UTF-8 \
    nohup "$HERDR" --session "$SESSION" server >"$B/herdr-server.log" 2>&1 &
  for _ in $(seq 50); do h workspace list >/dev/null 2>&1 && break; sleep 0.1; done
fi
h workspace create --label latency --cwd /tmp >/dev/null
sleep 0.5
read -r PANE TERM_ID < <(h pane list | python3 -c 'import json,sys; p=json.load(sys.stdin)["result"]["panes"][-1]; print(p["pane_id"], p["terminal_id"])')
h pane send-text "$PANE" 'clear; exec /bin/cat' >/dev/null
h pane send-keys "$PANE" Enter >/dev/null
sleep 0.5
echo "==> herdr pane $PANE ($TERM_ID) running: $(h pane process-info --pane "$PANE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["process_info"]["foreground_processes"][0]["cmdline"])')"

run_b() {
  XDG_CONFIG_HOME="$B/xdg" "$APP" --command /bin/cat \
    --bench "$SAMPLES" --label "b-r$1" --csv "$OUT/b-r$1.csv" 2>>"$B/harness.log"
}
run_c() {
  XDG_CONFIG_HOME="$B/xdg" "$APP" \
    --command "$HERDR --session $SESSION terminal attach $TERM_ID --takeover" \
    --env "HERDR_SOCKET_PATH=$SOCK" --env "XDG_CONFIG_HOME=$HOME/.config" \
    --bench "$SAMPLES" --label "c-r$1" --csv "$OUT/c-r$1.csv" 2>>"$B/harness.log"
}

for r in $(seq "$ROUNDS"); do
  run_b "$r"
  run_c "$r"
done

# Byte-level echo round trip, no rendering: isolates the herdr attach hop.
python3 "$HERE/herdr_rtt.py" direct 500 > "$OUT/rtt-direct.txt"
python3 "$HERE/herdr_rtt.py" attach 500 "$SESSION" "$TERM_ID" > "$OUT/rtt-attach.txt"

h server stop >/dev/null 2>&1 || true
python3 "$HERE/summarise.py" "$OUT"
