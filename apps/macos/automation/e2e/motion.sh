#!/usr/bin/env bash
# C6: motion polish captures (motion.azs), dev instance e2e-motion.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-motion}" E2E_SUITE=C6-motion
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null
run_azs "$e2e_here/motion.azs" motion
grep "frames " "$out/motion/automation.log" | tee -a "$checks"
! grep -q FAIL "$checks"
