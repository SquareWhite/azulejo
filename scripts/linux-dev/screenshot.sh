#!/usr/bin/env bash
# Runs inside the container: starts the shell under Xvfb, takes a screenshot,
# presses Ctrl+Shift+D, takes another. Output: target/linux-dev/*.png
set -euo pipefail
cargo build -p azulejo-linux
out=/src/target/linux-dev
mkdir -p "$out"

Xvfb :99 -screen 0 1280x800x24 >/dev/null 2>&1 &
export DISPLAY=:99 GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none
sleep 1
dbus-run-session -- /target/debug/azulejo >/tmp/azulejo.log 2>&1 &
win=""
for _ in $(seq 50); do
  win="$(xdotool search --onlyvisible --name '^Azulejo$' 2>/dev/null | head -1 || true)"
  [ -n "$win" ] && break
  sleep 0.2
done
[ -n "$win" ] || { cat /tmp/azulejo.log; echo "window never appeared" >&2; exit 1; }
sleep 1
import -window "$win" "$out/linux-shell.png"
xdotool windowfocus --sync "$win"
xdotool key ctrl+shift+d
sleep 1
import -window "$win" "$out/linux-shell-key.png"
cat /tmp/azulejo.log
echo "$out/linux-shell.png $out/linux-shell-key.png"
