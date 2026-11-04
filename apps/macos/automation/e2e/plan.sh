#!/usr/bin/env bash
# B6: plan review from inside a terminal tile (plan.azs.in), dev instance e2e-plan.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-plan}" E2E_SUITE=B6-plan
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
make_fixtures
write_config </dev/null
# The app can't screencapture itself: serve its capture requests (our window
# only, by id; nothing is raised). Needs an unlocked screen.
watch() {
  local o="$out/plan" req name wid
  while :; do
    for req in "$o/.capture"/*; do
      [[ -e "$req" ]] || continue
      name="$(basename "$req")"; rm -f "$req"
      wid="$(cat "$o/window-id" 2>/dev/null || true)"
      [[ -n "$wid" ]] && /usr/sbin/screencapture -l "$wid" -x -o "$o/$name.png" 2>/dev/null || true
    done
    sleep 0.05
  done
}
sed -e "s|@FX@|$fx|g" -e "s|@AZJ@|$azj_bin|g" -e "s|@OUT@|$out/plan|g" "$e2e_here/plan.azs.in" >"$out/plan.azs"
( sleep 2; mkdir -p "$out/plan/.capture"; watch ) &
watcher=$!
run_azs "$out/plan.azs" plan
kill "$watcher" 2>/dev/null || true
echo "--- azj preview open from the tile:"; cat "$out/plan/open.out" 2>/dev/null || true
echo "--- azj tile split from the tile:"; cat "$out/plan/split.out" 2>/dev/null || true
echo "--- tile env:"; cat "$out/plan/tile-env.txt" 2>/dev/null || true
# Save → preview swap latency: each edit's write time vs the app's next swap.
python3 - "$out/plan/edits.txt" "$out/plan/app.log" <<'PY' | tee -a "$checks"
import re, sys
edits = [float(l) for l in open(sys.argv[1]) if l.strip()]
swaps = [float(m.group(1)) for m in re.finditer(r"preview\.applied .*mode=swap .*t=([0-9.]+)", open(sys.argv[2]).read())]
lat = []
for e in edits:
    nxt = [s for s in swaps if s >= e]
    if nxt: lat.append((min(nxt) - e) * 1000)
print("preview save->swap latency ms:", ", ".join("%.1f" % x for x in lat), ("| max %.1f" % max(lat)) if lat else "none")
print("PASS preview within 200 ms" if lat and len(lat) == len(edits) and max(lat) < 200 else "FAIL preview within 200 ms")
PY
grep -o "preview.applied.*mode=native-[a-z]*" "$out/plan/app.log" | head -5 || true
ls "$out/plan"/*.png
