#!/usr/bin/env bash
# The ⌘K menu's grow and shrink (palette-motion.azs): timings at the real
# speed, typing and Esc / ⌘K / Enter mid-motion, rapid toggles, motion off,
# and mid-motion captures in all four built-in themes. Dev instance e2e-palm.
# System captures (screencapture of this window only) need an unlocked
# screen; the app's own renders (<name>-app.png) land either way.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-palm}" E2E_SUITE=palette-motion
source "$(dirname "$0")/lib.sh"
trap 'kill "${watcher:-}" 2>/dev/null || true; e2e_cleanup' EXIT
reset_instance
write_config </dev/null
name=palette-motion
rm -rf "$out/$name"
mkdir -p "$out/$name"

# The app asks for window captures by dropping files in <out>/.capture (it
# has no Screen Recording permission of its own); take them from here.
watch_captures() {
  local dir="$out/$name"
  while [[ ! -f "$dir/result" ]]; do
    for req in "$dir/.capture"/*; do
      [[ -e "$req" ]] || continue
      local shot
      shot="$(basename "$req")"
      rm -f "$req"
      local wid
      wid="$(cat "$dir/window-id" 2>/dev/null || true)"
      [[ -n "$wid" ]] && /usr/sbin/screencapture -l "$wid" -x -o "$dir/$shot.png" 2>/dev/null || true
    done
    sleep 0.01
  done
}
watch_captures &
watcher=$!

run_azs "$e2e_here/$name.azs" "$name"
kill "$watcher" 2>/dev/null || true
rm -rf "$out/$name/.capture"

# Grow ~160 ms, shrink ~110 ms (Design.Motion.paletteOpen / paletteClose),
# give or take a frame or two in a background window.
check "open and close timings" python3 - "$out/$name/automation.log" <<'PY'
import re, sys
log = open(sys.argv[1]).read()
opens = [int(v) for v in re.findall(r"remember palette\.open\.ms = (\d+)", log)]
closes = [int(v) for v in re.findall(r"remember palette\.close\.ms = (\d+)", log)]
print("open ms", opens, "close ms", closes)
assert len(opens) == 3 and len(closes) == 3, "three of each"
assert all(140 <= v <= 260 for v in opens), opens
assert all(90 <= v <= 210 for v in closes), closes
PY
grep -E "remember palette\.(open|close)\.ms" "$out/$name/automation.log" | sed 's/^/  /' | tee -a "$checks"
grep -E "chrome [0-9]+-[a-z0-9-]+: " "$out/$name/automation.log" | sed -E 's/.*chrome ([^:]+):.*palette=/  \1 palette=/' | tee -a "$checks"
renders="$(find "$out/$name" -maxdepth 1 -name '*-app.png' | wc -l | tr -d ' ')"
system="$(find "$out/$name" -maxdepth 1 -name '*.png' ! -name '*-app.png' | wc -l | tr -d ' ')"
note "captures: $renders in-app renders, $system system captures in $out/$name"
! grep -q FAIL "$checks"
