#!/usr/bin/env bash
# Workspace switches slide vertically (experimental, `[animation]
# workspace_switch`) on dev instance e2e-wss: wsslide.azs checks the
# direction (down the sidebar: old up and out, new in from below; up: the
# reverse), focus and typing landing in the new workspace mid-slide, a
# switch turned round mid-slide, rapid switches, next/previous/last, close
# and new, tab switches untouched, and motion off, Reduce Motion (stood in
# for), "none" and "fade" all instant (no slide). Mid-slide captures: the
# in-app render (<name>-app.png) and a window capture of our own window.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-wss}" E2E_SUITE=wsslide
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
require_frames
reset_instance
write_config </dev/null
# A fixture folder per workspace (the sidebar's Files view and the terminals
# show these, never the real home folder); e2e_cleanup removes it.
fx="$(mktemp -d "${TMPDIR:-/tmp}/azj-wss.XXXXXX")"
for w in one two three four five; do
  mkdir -p "$fx/$w"
  printf '# %s\n' "$w" >"$fx/$w/README.md"
done
sed "s|@FX@|$fx|g" "$e2e_here/wsslide.azs" >"$out/wsslide.azs"

# Window captures (`capture` lines) of our own window, by id.
start_capture_watcher "$out/*"

run_azs "$out/wsslide.azs" wsslide
grep -E "frames 0[1-6]-" "$out/wsslide/automation.log" >>"$checks" || true
! grep -q FAIL "$checks"
