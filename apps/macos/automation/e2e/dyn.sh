#!/usr/bin/env bash
# B3: dynamic layouts (dyn.azs) on dev instance e2e-dyn.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-dyn}" E2E_SUITE=B3-dyn
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null
run_azs "$e2e_here/dyn.azs" dyn
python3 -c "
import json;d=json.load(open('$out/dyn/dyn-stats.json'));print('animations',d['animation_count'],'pty',d['pty_resizes_actual'],'/',d['pty_resizes_expected'],'mismatches',d.get('resize_mismatches'))" | tee -a "$checks"
! grep -q FAIL "$checks"
