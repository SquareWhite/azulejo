#!/usr/bin/env bash
# Tile headers and workspace branches (2.14, 2.15) check, in-process, on its
# own dev instance (AZJ_DEV_NAME=hdr): terminal header follows cwd and the
# running command, the sidebar follows a branch switch in the tile, and a
# preview's Preview | Source choice survives a relaunch. The fixture repository
# lives in a temp folder that is removed afterwards.
#   apps/macos/automation/headers.sh [--keep]   (--keep: leave the dev instance)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-hdr}"
out="$root/.dev/$AZJ_DEV_NAME/out/headers"
fixtures="$(mktemp -d "${TMPDIR:-/tmp}/azj-headers.XXXXXX")"
cleanup() {
  kill "${watchdog:-}" 2>/dev/null || true
  rm -rf "$fixtures"
  [[ "${1:-}" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null
}
trap 'cleanup "${KEEP:-}"' EXIT
[[ "${1:-}" == "--keep" ]] && KEEP=--keep

repo="$fixtures/shop"
mkdir -p "$repo"
printf '# Plan\n\nShip the *headers*.\n' >"$repo/plan.md"
g() { git -C "$repo" -c user.name=azj -c user.email=azj@example.com -c commit.gpgsign=false "$@"; }
g -c init.defaultBranch=main init -q
g add plan.md
g commit -q -m "plan"
real="$(cd "$repo" && pwd -P)"

"$root/apps/macos/dev.sh" --reset >/dev/null   # also clears $out
mkdir -p "$out"
sed -e "s|@DIR@|$repo|g" -e "s|@REAL@|$real|g" \
  "$here/headers.azs.in" >"$fixtures/headers.azs"

# A hung run must not wait forever: stop the app after 3 minutes.
( sleep 180; "$root/apps/macos/dev.sh" --stop ) &
watchdog=$!

"$root/apps/macos/dev.sh" --automation "$fixtures/headers.azs" --out "$out"
[[ "$(cat "$out/result")" == ok ]] || { echo "headers: $(cat "$out/result")"; exit 1; }
"$root/apps/macos/dev.sh" --automation "$here/headers-relaunch.azs" --out "$out/relaunch"
[[ "$(cat "$out/relaunch/result")" == ok ]] || { echo "headers relaunch: $(cat "$out/relaunch/result")"; exit 1; }
echo "headers: ok ($out)"
