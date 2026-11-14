#!/usr/bin/env bash
# B15: stability (stab.azs.in), dev instance e2e-stab.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-stab}" E2E_SUITE=B15-stab
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
make_fixtures
serve_site 47842
write_config </dev/null
sed -e "s|@FX@|$fx|g" -e "s|@HERE@|$e2e_here|g" -e "s|@SITE@|http://127.0.0.1:47842/a.html|g" \
  -e "s|@OUT@|$out/stab|g" "$e2e_here/stab.azs.in" >"$out/stab.azs"
run_azs "$out/stab.azs" stab
cat "$out/stab/killed-webcontent.txt" "$out/stab/killed-quicklook.txt" 2>/dev/null || true
grep "remember" "$out/stab/automation.log" || true
! grep -q FAIL "$checks"
