#!/usr/bin/env bash
# Runs inside the spike container (see run.sh). Writes everything to /spike/out.
set -uo pipefail
OUT=/spike/out
mkdir -p "$OUT"
H="herdr --session azj-spike"
log() { printf '\n== %s\n' "$*"; }
shot() { sleep "${2:-1.5}"; import -display :99 -window root "$OUT/$1.png" && echo "screenshot $OUT/$1.png"; }

log "build VTE app"
(cd /spike/vte && cargo build --release -q) || exit 1
APP=/spike/vte/target/release/azj-vte-spike

log "build vtdump against libghostty-vt"
gcc -O2 -o /tmp/vtdump /spike/vtdump/vtdump.c \
  -I/opt/ghostty-vt/include -L/opt/ghostty-vt/lib -lghostty-vt \
  -Wl,-rpath,/opt/ghostty-vt/lib || exit 1

log "herdr server + workspace"
$H server >"$OUT/herdr-server.log" 2>&1 &
sleep 2
$H workspace create --label spike --env AZULEJO_TILE_ID=tile_spike >"$OUT/workspace.json"
PANE=$(python3 -c 'import json;print(json.load(open("/spike/out/workspace.json"))["result"]["root_pane"]["pane_id"])')
TERM_ID=$(python3 -c 'import json;print(json.load(open("/spike/out/workspace.json"))["result"]["root_pane"]["terminal_id"])')
echo "pane=$PANE terminal=$TERM_ID"

log "Xvfb + GTK app running: herdr terminal attach $TERM_ID"
Xvfb :99 -screen 0 1100x700x24 >/dev/null 2>&1 &
sleep 1
export DISPLAY=:99 GDK_BACKEND=x11 GSK_RENDERER=cairo NO_AT_BRIDGE=1
dbus-run-session -- "$APP" -- herdr --session azj-spike terminal attach "$TERM_ID" >"$OUT/app.log" 2>&1 &
APP_PID=$!
sleep 3

log "observer stream (second client, read-only)"
$H terminal session observe "$PANE" --cols 120 --rows 40 >"$OUT/observe.ndjson" 2>"$OUT/observe.err" &
OBS_PID=$!
sleep 1

run() { $H pane run "$PANE" "$1" >/dev/null; }
keys() { $H pane send-keys "$PANE" "$@" >/dev/null; }
text() { $H pane send-text "$PANE" "$1" >/dev/null; }

log "shell: env, colours, unicode, OSC 8 link"
run 'clear; echo "AZULEJO_TILE_ID=$AZULEJO_TILE_ID TERM=$TERM"; printf "\e[1;31mbold red\e[0m \e[38;2;255;128;0mtruecolor\e[0m \e[4munderline\e[0m  → ✓ λ 漢字 🦀\n"; printf "\e]8;;https://azulejo.dev\e\\\\OSC8 link\e]8;;\e\\\\\n"; seq 1 300 | tail -n 300 >/dev/null; seq 1 120; ls --color /'
shot 01-shell 2

log "vim"
run 'vim /etc/os-release'
shot 02-vim 2.5
text ':q'; keys Enter
sleep 1

log "htop"
run 'htop'
shot 03-htop 3
text 'q'
sleep 1

log "back to shell + scrollback probe"
run 'clear; seq 1 300'
shot 04-after-seq 2

log "scrollback: mouse wheel up over the VTE window (herdr enables mouse reporting)"
WIN=$(xdotool search --name 'azulejo vte spike' | head -1)
xdotool mousemove --window "$WIN" 400 300
for i in $(seq 1 15); do xdotool click 4; done
shot 06-wheel-up 1.5
$H pane get "$PANE" | python3 -c 'import json,sys;print("herdr scroll state:",json.load(sys.stdin)["result"]["pane"]["scroll"])'
for i in $(seq 1 15); do xdotool click 5; done
sleep 1

log "herdr pane.read (visible / recent)"
$H pane read "$PANE" --source visible >"$OUT/pane-read-visible.txt" 2>&1
$H pane read "$PANE" --source recent >"$OUT/pane-read-recent.txt" 2>&1
wc -l "$OUT"/pane-read-*.txt

log "takeover: a second writable client takes the terminal"
(sleep 6 | timeout 4 script -qf -c "stty rows 20 cols 80; herdr --session azj-spike terminal attach $TERM_ID --takeover" /dev/null >"$OUT/takeover-client.log" 2>&1) &
TK_PID=$!
shot 05-during-takeover 2
$H pane get "$PANE" | python3 -c 'import json,sys;print("pane during takeover:",json.load(sys.stdin)["result"]["pane"]["scroll"])'
wait $TK_PID
shot 05b-after-takeover-exit 1.5
echo "app still running? $(kill -0 $APP_PID 2>/dev/null && echo yes || echo no)"
tail -5 "$OUT/app.log"

log "libghostty-vt: replay the observer stream"
kill $OBS_PID 2>/dev/null; sleep 0.5
python3 /spike/vtdump/decode_frames.py <"$OUT/observe.ndjson" >"$OUT/observe.bin" 2>"$OUT/observe-shape.txt"
cat "$OUT/observe-shape.txt"
head -c 600 "$OUT/observe.ndjson" > "$OUT/observe-head.txt"
/tmp/vtdump 120 40 <"$OUT/observe.bin" >"$OUT/vtdump.txt"; echo "vtdump exit=$?"
head -3 "$OUT/vtdump.txt"

log "done"
kill $APP_PID 2>/dev/null
$H server stop >/dev/null 2>&1
