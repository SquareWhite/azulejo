#!/usr/bin/env bash
# The ⌘K menu's list stays between the field row and the footer when it
# scrolls (palette-clip.azs): no row drawn over either, the last row whole
# above the footer, at the menu's most height and in short windows. The
# app's own renders land in the output folder. Dev instance e2e-palc.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-palc}" E2E_SUITE=palette-clip
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null
name=palette-clip

run_azs "$e2e_here/$name.azs" "$name"
renders="$(find "$out/$name" -maxdepth 1 -name '*.png' | wc -l | tr -d ' ')"
note "renders: $renders in $out/$name"
! grep -q FAIL "$checks"
