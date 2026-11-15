#!/usr/bin/env bash
# Right-click menus (workspace rows, tile headers, the browser's chrome)
# check, in-process, on its own dev instance (AZJ_DEV_NAME=mnu): items and
# chords, rename, copy (private pasteboard), reveal (logged, Finder never
# opens), split, zoom, kill (cancelled), Close Tile (detaches), Close
# Workspace, cmd+w (close_pane) closing browser and preview tiles, and
# alt+cmd+w closing the workspace. Then it checks herdr: Close Workspace
# closes that herdr workspace with its panes and nothing else (ADR 0014),
# and after a relaunch herdr holds only the fresh workspace that replaced
# the app's last one. Captures menus-workspace.png and
# menus-tile.png (the menu drawn open in an in-app render).
#   apps/macos/automation/menus.sh [--keep]   (--keep: leave the dev instance)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-mnu}"
out="$root/.dev/$AZJ_DEV_NAME/out/menus"
fixtures="$(mktemp -d "${TMPDIR:-/tmp}/azj-menus.XXXXXX")"
fixtures="$(cd "$fixtures" && pwd -P)"
cleanup() {
  kill "${watchdog:-}" 2>/dev/null || true
  rm -rf "$fixtures"
  [[ "${1:-}" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null
}
trap 'cleanup "${KEEP:-}"' EXIT
[[ "${1:-}" == "--keep" ]] && KEEP=--keep

mkdir -p "$fixtures/alpha" "$fixtures/beta"
printf '# Notes\n\nRight-click menus.\n' >"$fixtures/notes.md"

"$root/apps/macos/dev.sh" --reset >/dev/null   # also clears $out
mkdir -p "$out"
sed -e "s|@DIR@|$fixtures|g" "$here/menus.azs.in" >"$fixtures/menus.azs"

# A hung run must not wait forever: stop the app after 3 minutes.
( sleep 180; "$root/apps/macos/dev.sh" --stop ) &
watchdog=$!

"$root/apps/macos/dev.sh" --automation "$fixtures/menus.azs" --out "$out"
[[ "$(cat "$out/result")" == ok ]] || { echo "menus: $(cat "$out/result")"; exit 1; }
"$root/apps/macos/dev.sh" --automation "$here/menus-relaunch.azs" --out "$out/relaunch"
[[ "$(cat "$out/relaunch/result")" == ok ]] || { echo "menus relaunch: $(cat "$out/relaunch/result")"; exit 1; }

# herdr after Close Workspace: exactly that workspace and its panes are gone
# (ADR 0014). After alt+cmd+w closed the rest and a relaunch: none of them
# is left, only the fresh workspace that took the last one's place.
python3 - "$out" <<'PY'
import json, sys
out = sys.argv[1]
def load(name):
    with open(f"{out}/{name}") as f:
        return json.load(f)["result"]
def panes(name):
    return sorted(p["pane_id"] for p in load(name)["panes"])
def workspaces(name):
    return sorted(w["workspace_id"] for w in load(name)["workspaces"])
ok = True
ws_before, ws_after = workspaces("workspaces-before.json"), workspaces("workspaces-after.json")
p_before, p_after = panes("panes-before.json"), panes("panes-after.json")
closed = sorted(set(ws_before) - set(ws_after))
gone = sorted(set(p_before) - set(p_after))
print(f"herdr after Close Workspace: workspaces {ws_before} -> {ws_after}, panes gone {gone}")
ok &= len(closed) == 1 and set(ws_after) <= set(ws_before)
ok &= bool(gone) and {p.split(":")[0] for p in gone} == set(closed)
ok &= all(p.split(":")[0] in ws_after for p in p_after)
ws_relaunch = workspaces("relaunch/workspaces-relaunch.json")
p_relaunch = panes("relaunch/panes-relaunch.json")
print(f"herdr after relaunch: workspaces {ws_relaunch}, {len(p_relaunch)} pane(s)")
ok &= len(ws_relaunch) == 1 and not set(ws_relaunch) & set(ws_before)
ok &= not set(p_relaunch) & set(p_before)
# Close Tile detached its pane too.
before, after = panes("panes-before-close.json"), panes("panes-after-tile-close.json")
print(f"herdr panes around Close Tile: {len(before)} before, {len(after)} after")
ok &= set(before) <= set(after)
sys.exit(0 if ok else 1)
PY
echo "menus: ok ($out)"
