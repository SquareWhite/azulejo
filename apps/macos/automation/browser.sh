#!/usr/bin/env bash
# Browser tile (1.8) check, in-process, on its own dev instance (AZJ_DEV_NAME=browser):
# serves automation/browser-site on 127.0.0.1:47831, runs browser.azs, relaunches
# the app and runs browser-relaunch.azs, then cleans up.
#   apps/macos/automation/browser.sh [--keep]   (--keep: leave the dev instance)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-browser}"
out="$root/.dev/$AZJ_DEV_NAME/out/browser"

python3 -m http.server 47831 --bind 127.0.0.1 --directory "$here/browser-site" >/dev/null 2>&1 &
server=$!
cleanup() {
  kill "$server" "${watchdog:-}" 2>/dev/null || true
  [[ "${1:-}" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null
}
trap 'cleanup "${KEEP:-}"' EXIT
[[ "${1:-}" == "--keep" ]] && KEEP=--keep
for _ in $(seq 50); do curl -fs http://127.0.0.1:47831/a.html >/dev/null && break; sleep 0.1; done

# A hung run must not wait forever: stop the app after 3 minutes.
( sleep 180; "$root/apps/macos/dev.sh" --stop ) &
watchdog=$!

"$root/apps/macos/dev.sh" --reset >/dev/null
"$root/apps/macos/dev.sh" --automation "$here/browser.azs" --out "$out"
[[ -f "$out/downloads/azulejo-download-test.bin" ]] && echo "download landed in $out/downloads"
"$root/apps/macos/dev.sh" --automation "$here/browser-relaunch.azs" --out "$out/relaunch"
