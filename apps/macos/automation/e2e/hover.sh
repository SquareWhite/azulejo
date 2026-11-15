#!/usr/bin/env bash
# D2: divider hover (hover.azs) on dev instance e2e-hover: the hovered
# divider, its hint and the app's cursor follow an in-app pointer across
# vertical and horizontal dividers, T and cross junctions, into terminals
# (libghostty's shape comes back) and through a drag.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-hover}" E2E_SUITE=D2-hover
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config <<'EOF'
[layout]
default = "manual"
EOF
run_azs "$e2e_here/hover.azs" hover
! grep -q FAIL "$checks"
