#!/usr/bin/env bash
# Top tabs: drag to reorder (Nov 5 feedback), on dev instance e2e-tabdrag
# with AZJ_DEBUG_MIRROR=1. tabdrag.azs drives the drags with mouse events
# made in the app; this checks that a reorder made herdr do nothing (ADR
# 0012: tab order isn't part of any mirror target, and herdr has no tab
# order to set), and takes the mid-drag capture of our own window.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-tabdrag}" E2E_SUITE=tabdrag AZJ_DEBUG_MIRROR=1
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null

# The app asks for window captures (`capture`) by dropping requests in
# .capture: `screencapture -l` of its own window only.
rm -rf "$out/tabdrag"; mkdir -p "$out/tabdrag/.capture"
(
  while [[ ! -f "$out/tabdrag/result" ]]; do
    for req in "$out/tabdrag/.capture"/*; do
      [[ -e "$req" ]] || continue
      name="$(basename "$req")"; rm -f "$req"
      wid="$(cat "$out/tabdrag/window-id" 2>/dev/null || true)"
      [[ -n "$wid" ]] && /usr/sbin/screencapture -l "$wid" -x -o "$out/tabdrag/$name.png" 2>/dev/null || true
    done
    sleep 0.01
  done
) &
watcher=$!
run_azs "$e2e_here/tabdrag.azs" tabdrag
kill "$watcher" 2>/dev/null || true

d="$out/tabdrag"
log="$d/app.log"
# Mirror lines between a drop's marks: none (no push, no pull).
quiet() {
  python3 - "$log" "$1" <<'PY'
import sys
lines = open(sys.argv[1], errors="replace").read().splitlines()
start = next(i for i, l in enumerate(lines) if f"tabdrag mark {sys.argv[2]}" in l and "settled" not in l)
end = next(i for i, l in enumerate(lines) if f"tabdrag mark {sys.argv[2]}-settled" in l)
mirror = [l for l in lines[start:end] if "mirror:" in l]
moves = [l for l in lines[start:end] if "tab.move" in l and "failed" in l]
print("\n".join(mirror + moves) or "quiet")
sys.exit(1 if mirror or moves else 0)
PY
}
check mirror-logging-on grep -q 'mirror:' "$log"
check drop-1-mirror-quiet quiet drop-1
check drop-2-mirror-quiet quiet drop-2
check herdr-tabs-unchanged python3 - "$d/herdr-tabs-before.json" "$d/herdr-tabs-after.json" <<'PY'
import json, sys
def tabs(path):
    data = json.load(open(path))
    data = data.get("result", data)
    tabs = data.get("tabs", data) if isinstance(data, dict) else data
    return sorted((t.get("tab_id"), t.get("workspace_id"), t.get("number")) for t in tabs)
before, after = tabs(sys.argv[1]), tabs(sys.argv[2])
print(before, after)
sys.exit(0 if before == after and len(before) == 3 else 1)
PY
check mid-drag-capture test -s "$d/mid-drag-app.png"
[[ -s "$d/mid-drag.png" ]] || note "no system capture of the mid-drag frame (screen locked?); mid-drag-app.png has the app's own render"
! grep -q FAIL "$checks"
