#!/usr/bin/env bash
# B8 / B13: reload.azs on dev instance e2e-reload.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-reload}" E2E_SUITE=B13-reload
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null
sed -e "s|\"\$AZJ\"|${clean_env[*]} AZULEJO_SOCKET=$sock $azj_bin|" "$e2e_here/reload.azs" >"$out/reload.azs"
run_azs "$out/reload.azs" reload
pc="$(grep -o 'remember palette.count = [0-9]*' "$out/reload/automation.log" | grep -o '[0-9]*$' || echo ?)"
cc="$(jq '.commands | length' "$out/reload/commands.json" 2>/dev/null || echo ?)"
echo "palette > rows $pc, commands.list $cc" | tee -a "$checks"
python3 -c "import json;d=json.load(open('$out/reload/anim-after-reload.json'));print('animations after reload with enabled=false:',d['animation_count'])" | tee -a "$checks"
! grep -q FAIL "$checks"
