#!/usr/bin/env bash
# Right-click menus (workspace rows, tile headers, the browser's chrome)
# check, in-process, on its own dev instance (AZJ_DEV_NAME=mnu): items and
# chords, rename, copy (private pasteboard), reveal (logged, Finder never
# opens), split, zoom, kill (cancelled), close-detach, and cmd+w closing the
# workspace from browser and preview tiles. Then it checks herdr: a closed
# workspace's herdr workspace and panes are all still there, and a relaunch
# doesn't bring the workspace back. Captures menus-workspace.png and
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

# herdr after Close Workspace: the same workspaces and panes as before it,
# and still after a relaunch.
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
for label, before, after in [
    ("workspaces", workspaces("workspaces-before.json"), workspaces("workspaces-after.json")),
    ("panes", panes("panes-before.json"), panes("panes-after.json")),
    ("workspaces (relaunch)", workspaces("workspaces-before.json"), workspaces("relaunch/workspaces-relaunch.json")),
    ("panes (relaunch)", panes("panes-before.json"), panes("relaunch/panes-relaunch.json")),
]:
    missing = sorted(set(before) - set(after))
    print(f"herdr {label}: {len(before)} before, {len(after)} after" + (f", missing {missing}" if missing else ""))
    ok &= not missing
# Close Tile detached its pane too.
before, after = panes("panes-before-close.json"), panes("panes-after-tile-close.json")
print(f"herdr panes around Close Tile: {len(before)} before, {len(after)} after")
ok &= set(before) <= set(after)
sys.exit(0 if ok else 1)
PY
echo "menus: ok ($out)"
