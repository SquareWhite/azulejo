#!/usr/bin/env bash
# B12: git branch per workspace (git.azs.in), dev instance e2e-git.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-git}" E2E_SUITE=B12-git
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
make_fixtures
write_config </dev/null
sha="$(git -C "$fx/azulejo" rev-parse --short HEAD)"
sed -e "s|@FX@|$fx|g" -e "s|@SHA@|$sha|g" "$e2e_here/git.azs.in" >"$out/git.azs"
run_azs "$out/git.azs" git
! grep -q FAIL "$checks"
