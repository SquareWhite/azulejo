#!/usr/bin/env bash
# D1: divider drags (drag.azs) on dev instance e2e-drag, with the mirror's
# debug log on. Besides the script's own checks (ratios, frames, hit areas,
# PTY resizes), the app log must show no mirror push while a drag is under
# way and exactly one once it has settled.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-drag}" E2E_SUITE=D1-drag AZJ_DEBUG_MIRROR=1
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config <<'EOF'
mirror_layout = true
[layout]
default = "manual"
EOF
run_azs "$e2e_here/drag.azs" drag

# Mirror pushes per drag, between the script's `drag-N start` and `drag-N
# settled` markers. The app logs the release ("divider drag …: N
# command(s)"). A drag that keeps moving gets its one push after the
# release; one that rests on the way (HOLD longer than the mirror's 150 ms
# quiet period) gets it during the rest, and none after.
python3 - "$out/drag/app.log" <<'PY' | tee -a "$checks"
import re, sys
lines = open(sys.argv[1], errors="replace").read().splitlines()
marks = {}
for i, line in enumerate(lines):
    m = re.search(r"# (drag-\d+) (start|settled)$", line)
    if m:
        marks.setdefault(m.group(1), {})[m.group(2)] = i
pushes = [i for i, l in enumerate(lines) if "mirror: push" in l]
ok = bool(marks)
for name, m in sorted(marks.items(), key=lambda kv: int(kv[0].split("-")[1])):
    if not {"start", "settled"} <= m.keys():
        print(f"FAIL mirror {name}: markers missing ({sorted(m)})"); ok = False; continue
    span = range(m["start"], m["settled"])
    release = next((i for i in span if "azulejo: divider drag " in lines[i]), None)
    command = next((lines[i] for i in span if re.search(r"^\s*\d+ divider drag ", lines[i])), "")
    held = len(command.split()) > 6
    inside = [i for i in pushes if i in span]
    early = [i for i in inside if release is None or i < release]
    restructure = [lines[i] for i in inside if "may restructure" in lines[i]]
    good = release is not None and len(inside) == 1 and not restructure and (held or not early)
    ok &= good
    print(f"{'PASS' if good else 'FAIL'} mirror {name}: {len(inside)} push(es), "
          f"{len(early)} before the release{' (held)' if held else ''}"
          f"{', restructured' if restructure else ''}")
print(f"{'PASS' if ok else 'FAIL'} mirror: one push per drag, none while the pointer moves")
PY

python3 - "$out/drag" <<'PY' | tee -a "$checks"
import json, pathlib, sys
d = pathlib.Path(sys.argv[1])
a = json.load(open(d / "drag-anim.json"))
print(f"NOTE frames: {a['ticks']} ticks at {a['refresh_hz']:.0f} Hz, dropped {a['dropped_frames']} "
      f"(by kind {a['dropped_by_kind']}), interval p99 {a['interval_p99_ms']} ms, max {a['interval_max_ms']} ms")
for f in sorted(d.glob("drag-[0-9]*.json")):
    s = json.load(open(f))
    if "commands" not in s:
        continue  # a capture's freshness report
    terms = s.get("terminals", [])
    mid = [t["mid_drag"] for t in terms]
    tot = [t["total"] for t in terms]
    print(f"NOTE {f.stem}: {s['commands']} command(s) in {s['duration_ms']} ms, "
          f"PTY resizes mid-drag {mid}, total {tot}")
PY
! grep -q FAIL "$checks"
