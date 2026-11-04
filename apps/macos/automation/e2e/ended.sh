#!/usr/bin/env bash
# B9: a pane killed while the app is closed shows "Session ended"; Respawn and
# Close both work. Dev instance e2e-ended.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-ended}" E2E_SUITE=B9-ended
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null
run_azs "$e2e_here/ended-1.azs" part1
p2="$(cat "$out/part1/kill1")"; p3="$(cat "$out/part1/kill2")"
echo "closing $p2 and $p3 in herdr while the app is down" | tee -a "$checks"
hd pane close "$p2" >/dev/null
hd pane close "$p3" >/dev/null
run_azs "$e2e_here/ended-2.azs" part2
! grep -q FAIL "$checks"
