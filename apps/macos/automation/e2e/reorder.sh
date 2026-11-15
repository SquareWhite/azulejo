#!/usr/bin/env bash
# Reordering workspaces by drag (ADR 0013) on dev instance e2e-reorder:
# reorder.azs drags rows in the full sidebar and the rail (drop, Esc, a drop
# out of reach, a click, a refresh mid-drag, ⌘ numbers), then a relaunch
# keeps the order (reorder-relaunch.azs); `azj workspace list` numbers it,
# `azj workspace move` moves one, and herdr's own workspace order never
# changes. Window captures of our own window only.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-reorder}" E2E_SUITE=reorder
source "$(dirname "$0")/lib.sh"
watcher=""
cleanup() { [[ -n "$watcher" ]] && kill "$watcher" 2>/dev/null || true; e2e_cleanup; }
trap cleanup EXIT
reset_instance
write_config </dev/null

# Real window captures (`capture` lines) of our own window, by id.
watch_captures() {
  while true; do
    for dir in "$out"/*/.capture; do
      [[ -d "$dir" ]] || continue
      for req in "$dir"/*; do
        [[ -e "$req" ]] || continue
        name="$(basename "$req")"; rm -f "$req"
        run="$(dirname "$dir")"
        wid="$(cat "$run/window-id" 2>/dev/null || true)"
        [[ -n "$wid" ]] && /usr/sbin/screencapture -l "$wid" -x -o "$run/$name.png" 2>/dev/null || true
      done
    done
    sleep 0.02
  done
}
watch_captures &
watcher=$!

run_azs "$e2e_here/reorder.azs" reorder
# herdr's workspaces in its own order (its numbers).
herdr_ids() { hd workspace list 2>/dev/null | python3 -c '
import json, sys
ws = json.load(sys.stdin)["result"]["workspaces"]
print(" ".join(w["workspace_id"] for w in sorted(ws, key=lambda w: w["number"])))'; }

say "relaunch keeps the order"
run_azs "$e2e_here/reorder-relaunch.azs" relaunch

say "API and CLI see the order; herdr's own order stays"
launch_bg >"$out/launch.txt"
wait_ping
sleep 1
azj --json workspace list >"$out/list.json"
check "workspace.list is in the dragged order, numbered from 1" python3 -c "
import json; w = json.load(open('$out/list.json'))['workspaces']
got = [(x['name'], x['number']) for x in w]; print(got)
assert got == [('alpha', 1), ('bravo', 2), ('delta', 3), ('charlie', 4)]"
# herdr made its workspaces in creation order: alpha, bravo, charlie, delta.
by_name="$(python3 -c "
import json; w = {x['name']: x['herdr_workspace'] for x in json.load(open('$out/list.json'))['workspaces']}
print(' '.join(w[n] for n in ['alpha', 'bravo', 'charlie', 'delta']))")"
herdr_order="$(herdr_ids)"
echo "herdr: $herdr_order; created: $by_name" | tee "$out/herdr-order.txt"
check "herdr's workspace order is untouched" test "$herdr_order" = "$by_name"
charlie="$(jq -r '.workspaces[] | select(.name=="charlie") | .id' "$out/list.json")"
azj workspace move --workspace "$charlie" --index 0 >/dev/null
azj --json workspace list >"$out/list-moved.json"
check "azj workspace move puts charlie first" python3 -c "
import json; w = json.load(open('$out/list-moved.json'))['workspaces']
got = [(x['name'], x['number']) for x in w]; print(got)
assert got == [('charlie', 1), ('alpha', 2), ('bravo', 3), ('delta', 4)]"
check "herdr's order is still untouched" test "$(herdr_ids)" = "$by_name"
stop_app
! grep -q FAIL "$checks"
