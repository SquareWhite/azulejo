#!/usr/bin/env bash
# Window chrome, tab strip and toolbar search field (D.2, D.4, D.8) check,
# in-process, on its own dev instance (AZJ_DEV_NAME=toolbar). Window captures
# land in .dev/<name>/out/toolbar for comparing with docs/design/mockup-v1.png.
#   apps/macos/automation/toolbar.sh [--keep]   (--keep: leave the dev instance)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-toolbar}"
out="$root/.dev/$AZJ_DEV_NAME/out/toolbar"
KEEP="${1:-}"
cleanup() {
  kill "${watcher:-}" "${watchdog:-}" 2>/dev/null || true
  rm -rf "$out/.capture"
  [[ "$KEEP" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null 2>&1 || true
}
trap cleanup EXIT

"$root/apps/macos/dev.sh" --reset >/dev/null   # also clears $out
mkdir -p "$out/.capture"

# The app asks for window captures by dropping files in $out/.capture (it has
# no Screen Recording permission of its own); take them from here. Window
# captures include the title bar's traffic lights, which `shot` can't see.
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

# A hung run must not wait forever: stop the app after 2 minutes.
( sleep 120; "$root/apps/macos/dev.sh" --stop ) &
watchdog=$!

"$root/apps/macos/dev.sh" --automation "$here/toolbar.azs" --out "$out"
[[ "$(cat "$out/result")" == ok ]] || { echo "toolbar: $(cat "$out/result")"; exit 1; }
echo "toolbar: ok ($out)"
