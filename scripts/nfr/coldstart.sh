#!/usr/bin/env bash
# PLAN 1.13: cold start, process start → first window with the restored layout
# drawn (target < 500 ms). In-app marks (Startup.swift) are relative to the
# kernel's process start time; `open` → process start (LaunchServices) is
# reported separately.
#
#   scripts/nfr/coldstart.sh [runs]            debug build (just mac first)
#   NFR_APP=target/release-mac/Azulejo.app scripts/nfr/coldstart.sh [runs]
#
# Two states: empty (no saved session) and restored (2 workspaces, 8 tiles:
# 6 terminals, a browser, a preview). Runs on the dev instance `nfr`.
set -euo pipefail
source "$(dirname "$0")/lib.sh"
runs="${1:-10}"
build="$(basename "$(dirname "$app")")"
timing="$out/coldstart-$build.jsonl"
cargo build -q -p azulejo-cli
reset_all
prepare
: >"$timing"

measure() { # label
  local label=$1 i before t_open
  for i in $(seq "$runs"); do
    if [[ $label == empty ]]; then rm -f "$state"; herdr_fresh; fi
    before=$(wc -l <"$timing")
    t_open=$(python3 -c 'import time; print("%.4f" % time.time())')
    launch "AZULEJO_TIMING_OUT=$timing"
    for _ in $(seq 100); do [[ $(wc -l <"$timing") -gt $before ]] && break; sleep 0.05; done
    [[ $(wc -l <"$timing") -gt $before ]] || fail "$label run $i: no timing line (see $log)"
    # Annotate the line with the label and the LaunchServices hop.
    python3 - "$timing" "$label" "$t_open" <<'EOF'
import json, sys
path, label, t_open = sys.argv[1], sys.argv[2], float(sys.argv[3])
lines = open(path).read().splitlines()
d = json.loads(lines[-1]); d["label"] = label; d["open_to_process_ms"] = round((d["process_start"] - t_open) * 1000, 1)
lines[-1] = json.dumps(d, sort_keys=True)
open(path, "w").write("\n".join(lines) + "\n")
EOF
    # Restored runs: also time until every visible terminal is attached.
    if [[ $label == restored ]]; then
      sleep 1.5
      python3 - "$timing" "$log" <<'EOF'
import json, re, sys
path, logf = sys.argv[1], sys.argv[2]
lines = open(path).read().splitlines()
d = json.loads(lines[-1])
ats = [float(m.group(1)) for m in re.finditer(r"attached \d+ ms after launch at ([0-9.]+)", open(logf).read())]
d["terminals_attached"] = len(ats)
if ats: d["all_attached_ms"] = round((max(ats) - d["process_start"]) * 1000, 1)
lines[-1] = json.dumps(d, sort_keys=True)
open(path, "w").write("\n".join(lines) + "\n")
EOF
    fi
    stop
    sleep 0.5
  done
}

say "empty state × $runs ($build)"
measure empty

say "build the restored state (2 workspaces, 8 tiles)"
rm -f "$state"
herdr_fresh
launch
wait_ping
wait_tiles 1
for _ in 1 2 3; do azj layout split-auto >/dev/null; done
plan="$dev/plan.md"
printf '# Plan\n\n- [x] cold start\n- [ ] memory\n' >"$plan"
azj workspace create --name second >/dev/null
azj tab create >/dev/null
azj layout split-auto >/dev/null
azj layout split-auto --kind browser --url "http://127.0.0.1:9/nfr" >/dev/null
azj layout split-auto --kind preview --path "$plan" >/dev/null
sleep 2
first_ws="$(azj --json session get | jq -r '.session.windows[0].workspaces[0].id')"
azj workspace focus --workspace "$first_ws" >/dev/null
sleep 1
azj --json tile list | jq -r '[.tiles[].kind] | group_by(.) | map("\(.[0]) \(length)") | join(", ")'
stop
sleep 0.5

say "restored state × $runs ($build)"
measure restored

stop
python3 - "$timing" <<'EOF'
import json, statistics, sys
rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
def pct(v, q):
    v = sorted(v); return v[min(len(v) - 1, round(q * (len(v) - 1)))]
for label in ("empty", "restored"):
    rs = [r for r in rows if r.get("label") == label]
    if not rs: continue
    print(f"\n{label}: {len(rs)} runs, tiles {rs[0]['tiles']}")
    for key in ("main_ms", "core_ms", "layout_ms", "committed_ms", "first_frame_ms", "all_attached_ms", "open_to_process_ms"):
        v = [r[key] for r in rs if key in r]
        if v: print(f"  {key:20} median {statistics.median(v):7.1f}  p95 {pct(v, .95):7.1f}  min {min(v):7.1f}  max {max(v):7.1f}")
EOF
