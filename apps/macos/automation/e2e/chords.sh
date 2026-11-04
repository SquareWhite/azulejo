#!/usr/bin/env bash
# B2: every default chord (chords.azs.in), on dev instance e2e-chords.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-chords}" E2E_SUITE=B2-chords
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
make_fixtures
serve_site 47841
write_config </dev/null
sed -e "s|@PLAN@|$fx/plan.md|g" -e "s|@SITE@|http://127.0.0.1:47841/|g" "$e2e_here/chords.azs.in" >"$out/chords.azs"
run_azs "$out/chords.azs" chords
! grep -q FAIL "$checks"
