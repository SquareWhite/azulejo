#!/usr/bin/env bash
# Screen-on check for V1 (stale terminals after crop motion). Run it with the
# screen unlocked and the display awake; the window stays in the background.
#
#   apps/macos/automation/e2e/stale-check.sh
#
# Crop motions over vim and fish (split, resize, swap, close, a layout
# cycle), then checks, from the app log and the captures' freshness reports:
#   - every reveal came from the display-link clock (`fresh` or `deadline`),
#     none from the fallback timer, and none uncovered a STALE live frame;
#   - libghostty drew at every new size (no "NO frame at" lines);
#   - no capture flagged a STALE terminal, and the display link ticked.
# It also leaves the system capture (screencapture) next to the app's own
# render and herdr's `pane read` text, to eyeball: stale-check/settled*.png,
# stale-check/pane*.txt. Uses dev instance e2e-stale; resets it after.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-stale}" E2E_SUITE=stale-check
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT

# STALE_CHECK_FORCE=1 runs it anyway (to test the script; the screen-on checks then fail).
if [[ "${STALE_CHECK_FORCE:-}" != 1 ]] && ioreg -n Root -d1 -a 2>/dev/null | grep -A1 CGSSessionScreenIsLocked | grep -q '<true/>'; then
  echo "stale-check: the screen is locked; unlock it (and wake the display), then run again" >&2
  exit 2
fi

reset_instance
write_config </dev/null
azs="$out/stale-check.azs"
cat >"$azs" <<'EOF'
log V1 stale-check: crop motions over vim and fish
delivery direct
until terminal.phase attached 10000
key ctrl+cmd+t
until tiles 2 3000
wait 1500
core {"method":"terminal.send_text","params":{"text":"vim -c \"set nu\" /etc/hosts\n"}}
wait 1200
key ctrl+cmd+t
until tiles 3 3000
wait 1200
key ctrl+alt+shift+h
wait 600
capture resize-end 0
key alt+shift+h
wait 600
capture swap-end 0
key alt+backslash
wait 600
key alt+backslash
wait 600
key alt+t
wait 800
key alt+w
capture close-300ms 300
wait 800
shot settled-shot
capture settled 0
wait 500
sh {herdr} --session {session} pane read {pane:1} --source visible --format text > {out}/pane1.txt; {herdr} --session {session} pane read {pane:2} --source visible --format text > {out}/pane2.txt
EOF

# System captures (needs the screen on): the app drops requests in .capture.
mkdir -p "$out/stale-check/.capture"
(
  while [[ ! -f "$out/stale-check/result" ]]; do
    for req in "$out/stale-check/.capture"/*; do
      [[ -e "$req" ]] || continue
      name="$(basename "$req")"; rm -f "$req"
      wid="$(cat "$out/stale-check/window-id" 2>/dev/null || true)"
      [[ -n "$wid" ]] && /usr/sbin/screencapture -l "$wid" -x -o "$out/stale-check/$name.png" 2>/dev/null || true
    done
    sleep 0.01
  done
) &
watcher=$!
run_azs "$azs" stale-check
kill "$watcher" 2>/dev/null || true

d="$out/stale-check"
log="$d/app.log"
reveals="$(grep -o 'revealed ([^)]*)' "$log" | sort | uniq -c || true)"
echo "reveals:"; echo "${reveals:-  none}"
check "every reveal came from the display-link clock" bash -c "! grep -q 'revealed (timer' '$log'"
check "no reveal uncovered a STALE live frame" bash -c "! grep 'revealed (' '$log' | grep -q STALE"
check "libghostty drew at every new size" bash -c "! grep -q 'NO frame at' '$log'"
check "no capture saw a STALE terminal" bash -c "! grep -q '  STALE ' '$d/automation.log'"
check "the display link ticked" python3 -c "
import json,sys; r=json.load(open('$d/settled-app.fresh.json'))
sys.exit(0 if not r['screen_locked'] and r['display_link_ticks_since_last_motion'] > 0 else 1)"
check "system captures taken" test -f "$d/settled.png"
python3 - "$log" <<'PY' | tee -a "$checks"
import re, sys
ms = [int(m) for m in re.findall(r"drew at \S+ px (\d+) ms after the resize", open(sys.argv[1]).read())]
if ms:
    ms.sort()
    print(f"NOTE libghostty first frame at the new size: {len(ms)} resizes, median {ms[len(ms)//2]} ms, max {ms[-1]} ms")
PY
echo "compare: $d/settled.png (system) vs $d/settled-app.png and settled-shot.png (in-app) vs $d/pane1.txt, pane2.txt"
! grep -q FAIL "$checks"
