#!/usr/bin/env bash
# B2: every default chord (chords.azs.in), on dev instance e2e-chords.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-chords}" E2E_SUITE=B2-chords
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null
cp "$e2e_here/chords.azs.in" "$out/chords.azs"
run_azs "$out/chords.azs" chords
! grep -q FAIL "$checks"
