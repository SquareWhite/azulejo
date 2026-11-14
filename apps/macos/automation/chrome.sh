#!/usr/bin/env bash
# Palette, picker, config banner (palette.azs) and attention badges
# (attention.azs), in-process on their own dev instance (AZJ_DEV_NAME=shell).
#   apps/macos/automation/chrome.sh [--keep]   (--keep: leave the dev instance)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-shell}"
# Outside the instance folder: --reset between scripts wipes .dev/<name>/out.
out="$root/.dev/$AZJ_DEV_NAME.out"
keep="${1:-}"
cleanup() { [[ "$keep" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null; }
trap cleanup EXIT

# A hung run must not wait forever: stop the app after 3 minutes per script.
status=0
for script in palette attention; do
  ( sleep 180; "$root/apps/macos/dev.sh" --stop ) &
  watchdog=$!
  "$root/apps/macos/dev.sh" --reset >/dev/null
  "$root/apps/macos/dev.sh" --automation "$here/$script.azs" --out "$out/$script" || status=1
  kill "$watchdog" 2>/dev/null || true
  [[ "$(cat "$out/$script/result" 2>/dev/null)" == ok ]] || status=1
done
exit "$status"
