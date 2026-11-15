#!/usr/bin/env bash
# Monocle tabs (mtabs.azs): a Monocle tile's header strip shows the stack as
# tabs; clicks focus, drags reorder, overflow collapses into "+N", the
# switch in and out cross-fades with motion on and is instant with it off
# or under Reduce Motion. Dev instance e2e-mtabs. Takes in-app shots in a
# dark and a light theme (local only: .dev/e2e.out/mtabs/mtabs/) and the
# mid-fade window capture of our own window.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-mtabs}" E2E_SUITE=mtabs
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null

# The app asks for window captures (`capture`) by dropping requests in
# .capture: `screencapture -l` of its own window only.
d="$out/mtabs"
rm -rf "$d"; mkdir -p "$d/.capture"
(
  while [[ ! -f "$d/result" ]]; do
    for req in "$d/.capture"/*; do
      [[ -e "$req" ]] || continue
      name="$(basename "$req")"; rm -f "$req"
      wid="$(cat "$d/window-id" 2>/dev/null || true)"
      [[ -n "$wid" ]] && /usr/sbin/screencapture -l "$wid" -x -o "$d/$name.png" 2>/dev/null || true
    done
    sleep 0.01
  done
) &
watcher=$!
run_azs "$e2e_here/mtabs.azs" mtabs
kill "$watcher" 2>/dev/null || true

for shot in mtabs-dark mtabs-light mtabs-overflow; do
  check "shot $shot" test -s "$d/$shot.png"
done
check "mid-fade capture" test -s "$d/mtabs-mid-in-app.png"
! grep -q FAIL "$checks"
