#!/usr/bin/env bash
# Top bar captures: the toolbar row (traffic lights, sidebar toggle, tabs,
# layout picker, ⌘K field) in all four built-in themes, with the sidebar
# full, as a rail and hidden (topbar-shots.azs), on its own dev instance
# (AZJ_DEV_NAME=topbar). Window captures of our own window only; the top
# 72 pt of each lands in docs/testing/topbar/<label>/ for before/after
# comparisons. Full screen isn't captured: it would take over the screen.
#   apps/macos/automation/topbar.sh LABEL [--keep]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
label="${1:?usage: topbar.sh LABEL [--keep]}"
KEEP="${2:-}"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-topbar}"
out="$root/.dev/$AZJ_DEV_NAME.out/topbar"
dest="$root/docs/testing/topbar/$label"
cleanup() {
  kill "${watcher:-}" "${watchdog:-}" 2>/dev/null || true
  rm -rf "$out/.capture"
  [[ "$KEEP" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null 2>&1 || true
}
trap cleanup EXIT

# A fresh profile config (dev.sh writes it again): the run edits it.
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

"$root/apps/macos/dev.sh" --automation "$here/topbar-shots.azs" --out "$out"
[[ "$(cat "$out/result")" == ok ]] || { echo "topbar: $(cat "$out/result")"; exit 1; }

mkdir -p "$dest"
python3 - "$out" "$dest" <<'PY'
import sys
from pathlib import Path
from PIL import Image

out, dest = Path(sys.argv[1]), Path(sys.argv[2])
shots = sorted(p for p in out.glob("*.png") if p.stem.endswith(("-full", "-rail", "-hidden")))
for p in shots:
    img = Image.open(p)
    scale = round(img.width / 1200) or 1
    img.crop((0, 0, img.width, 72 * scale)).save(dest / p.name, optimize=True)
print(f"topbar: {len(shots)} captures → {dest}")
PY
echo "topbar: ok ($out)"
