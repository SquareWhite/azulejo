#!/usr/bin/env bash
# B4: animations off in config → no motion (animoff.azs), dev instance e2e-aoff.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-aoff}" E2E_SUITE=B4-animoff
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config <<'TOML'
[animation]
enabled = false
TOML
run_azs "$e2e_here/animoff.azs" animoff
grep "frames off-" "$out/animoff/automation.log" | tee -a "$checks"
python3 -c "import json;d=json.load(open('$out/animoff/off-stats.json'));print('animation_count',d['animation_count'])" | tee -a "$checks"
