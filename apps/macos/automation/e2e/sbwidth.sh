#!/usr/bin/env bash
# The sidebar's width on dev instance e2e-sbw: sbwidth.azs drags the card's
# inner edge (in-app mouse events), checks the clamps, the tile area and its
# terminals, collapse and open, and the double-click reset; a relaunch
# (sbwidth-relaunch.azs) opens at the saved width, then resets it so the dev
# profile's saved width is the default again.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-sbw}" E2E_SUITE=sbwidth
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null
run_azs "$e2e_here/sbwidth.azs" sbwidth
say "relaunch keeps the width"
run_azs "$e2e_here/sbwidth-relaunch.azs" relaunch
! grep -q FAIL "$checks"
