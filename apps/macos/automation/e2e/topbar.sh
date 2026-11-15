#!/usr/bin/env bash
# Top bar (Nov 5 feedback): the layout picker and the top tabs' right-click
# menu (topbar.azs) on dev instance e2e-top. Checks the picker's switches
# animated as the keys' do.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-top}" E2E_SUITE=topbar
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null
run_azs "$e2e_here/topbar.azs" topbar
# Four picks of a new kind, each a layout motion like its key's.
check picker-animates python3 -c "
import json, sys
d = json.load(open('$out/topbar/picker-stats.json'))
print('animations', d['animation_count'])
sys.exit(0 if d['animation_count'] >= 4 else 1)"
! grep -q FAIL "$checks"
