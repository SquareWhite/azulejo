#!/usr/bin/env bash
# Rail captures: the collapsed sidebar (five workspaces, agents working and
# blocked, the second selected) in all four built-in themes (rail-shots.azs),
# on its own dev instance (AZJ_DEV_NAME=railcap). Window captures of our own
# window only; the left 64 pt of each, with 2x zooms of the rows, land in
# docs/testing/rail/<label>/ for before/after comparisons.
#   apps/macos/automation/rail.sh LABEL [--keep]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
label="${1:?usage: rail.sh LABEL [--keep]}"
KEEP="${2:-}"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-railcap}"
out="$root/.dev/$AZJ_DEV_NAME.out/rail"
dest="$root/docs/testing/rail/$label"
cleanup() {
  kill "${watcher:-}" "${watchdog:-}" 2>/dev/null || true
  rm -rf "$out/.capture"
  [[ "$KEEP" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null 2>&1 || true
}
trap cleanup EXIT

# Never the user's herdr: the dev instance has its own session.
unset HERDR_SOCKET_PATH HERDR_PANE_ID HERDR_WORKSPACE_ID HERDR_TAB_ID HERDR_ENV
rm -f "$root/.dev/$AZJ_DEV_NAME/config/azulejo/config.toml"
"$root/apps/macos/dev.sh" --reset >/dev/null
rm -rf "$out"
mkdir -p "$out/.capture"

# The app asks for window captures by dropping files in $out/.capture; take
# them of its own window (by id) from here.
watch_captures() {
  while [[ ! -f "$out/result" ]]; do
    for req in "$out/.capture"/*; do
      [[ -e "$req" ]] || continue
      name="$(basename "$req")"
      rm -f "$req"
      wid="$(cat "$out/window-id" 2>/dev/null || true)"
      [[ -n "$wid" ]] && /usr/sbin/screencapture -l "$wid" -x -o "$out/$name.png" 2>/dev/null || true
    done
    sleep 0.02
  done
}
watch_captures &
watcher=$!
( sleep 180; "$root/apps/macos/dev.sh" --stop ) &
watchdog=$!

"$root/apps/macos/dev.sh" --automation "$here/rail-shots.azs" --out "$out"
[[ "$(cat "$out/result")" == ok ]] || { echo "rail: $(cat "$out/result")"; exit 1; }

mkdir -p "$dest"
python3 - "$out" "$dest" <<'PY'
import sys
from pathlib import Path
from PIL import Image

out, dest = Path(sys.argv[1]), Path(sys.argv[2])
shots = sorted(out.glob("*-rail.png"))
for p in shots:
    img = Image.open(p)
    scale = round(img.width / 1000) or 1
    rail = img.crop((0, 0, 64 * scale, img.height))
    rail.save(dest / p.name, optimize=True)
    # The rows, zoomed: the group header and the five workspaces.
    rows = img.crop((0, 52 * scale, 64 * scale, 200 * scale))
    rows.resize((rows.width * 2, rows.height * 2), Image.NEAREST).save(
        dest / p.name.replace("-rail", "-rows"), optimize=True)
print(f"rail: {len(shots)} captures → {dest}")
PY
echo "rail: ok ($out)"
