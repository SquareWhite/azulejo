#!/usr/bin/env bash
# B5: agent supervision (agents.azs) on dev instance e2e-agents.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-agents}" E2E_SUITE=B5-agents
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null
run_azs "$e2e_here/agents.azs" agents
# Ring spin: compare the sidebar row's icon between two captures 310 ms apart.
uv run -q --with pillow python - "$out/agents" <<'PY' | tee -a "$checks"
import sys
from PIL import Image, ImageChops
d = sys.argv[1]
def icon(name):
    im = Image.open(f"{d}/{name}-app.png").convert("RGB")
    s = im.width / 1100  # points → pixels
    return im.crop((int(18*s), int(110*s), int(46*s), int(134*s)))  # first workspace row's icon
for mode in ("on", "off"):
    a, b = icon(f"ring-{mode}-a"), icon(f"ring-{mode}-b")
    diff = ImageChops.difference(a, b).getbbox()
    a.resize((a.width*4, a.height*4)).save(f"{d}/ring-{mode}-icon.png")
    print(f"ring motion {mode}: icon {'changes' if diff else 'identical'} between captures")
PY
! grep -q FAIL "$checks"
