#!/usr/bin/env bash
# herdr reconnect NFR (≤ 2 s), the two slow steps of restore.sh repeated:
# herdr server restart → every terminal reattached, and a launch that has to
# start herdr → every terminal attached. Same session as restore.sh: 6
# terminals in a tall layout, a browser and a zoomed preview in a second tab.
# Each run records the load average, since other work on the Mac moves these.
#
#   scripts/nfr/reconnect.sh [runs]            debug build (just mac first)
#
# Times are wall clock until every terminal's last attach is running and
# still running a few seconds later (restore.sh's "attached and holding").
# Dev instance `nfr` (AZJ_DEV_NAME overrides it); reset at the end.
set -euo pipefail
source "$(dirname "$0")/lib.sh"
runs="${1:-5}"
results="$out/reconnect-$(basename "$(dirname "$app")").txt"
target=6
cargo build -q -p azulejo-cli
reset_all
prepare
: >"$results"
trap reset_all EXIT

mark() { wc -l <"$log" | tr -d ' '; }
now() { python3 -c 'import time; print("%.3f" % time.time())'; }
load() { sysctl -n vm.loadavg | awk '{print $2}'; }

# Waits for `n` "attached … after <reason>" lines after line `from`.
wait_attached() {
  local from=$1 n=$2 reason=$3 timeout=${4:-15}
  for _ in $(seq $((timeout * 20))); do
    [[ $(tail -n +"$((from + 1))" "$log" | grep -cE "attached [0-9]+ ms after $reason at" || true) -ge $n ]] && return 0
    sleep 0.05
  done
  fail "terminals didn't attach after $reason (see $log)"
}

# Ms from `start` until every terminal's last attach after line `from`,
# once none has ended in a quiet period.
settled_ms() {
  local from=$1 start=$2 quiet=${3:-3}
  sleep "$quiet"
  tail -n +"$((from + 1))" "$log" | python3 -c '
import re, sys
start, last, ended = float(sys.argv[1]), {}, {}
for n, line in enumerate(sys.stdin):
    if m := re.search(r"terminal (\w{4}): attached \d+ ms after .* at ([0-9.]+)$", line):
        last[m.group(1)] = (n, float(m.group(2)))
    elif m := re.search(r"terminal (\w{4}): attach ended", line):
        ended[m.group(1)] = n
for tile, (n, _) in last.items():
    if ended.get(tile, -1) > n:
        sys.exit(f"terminal {tile} did not stay attached")
if len(last) < int(sys.argv[2]):
    sys.exit(f"only {len(last)} terminals attached")
print(int((max(t for _, t in last.values()) - start) * 1000))
' "$start" "$target"
}

say "build the session"
launch
wait_ping
wait_tiles 1
for _ in 1 2 3 4 5; do azj layout split-auto >/dev/null; done
wait_attached 0 $target "(launch|new tile)"
plan="$dev/plan.md"
printf '# Plan\n\n- [x] reconnect\n' >"$plan"
first_tab="$(azj --json session get | jq -r '.session.windows[0].workspaces[0].active_tab')"
azj tab create --kind browser --url "http://127.0.0.1:9/reconnect" >/dev/null
azj layout split-auto --kind preview --path "$plan" >/dev/null
preview="$(azj --json tile list | jq -r '.tiles[] | select(.kind=="preview") | .id')"
azj layout zoom --tile "$preview" --zoomed true >/dev/null
azj tab focus --tab "$first_tab" >/dev/null
sleep 1.5

for i in $(seq "$runs"); do
  hd server stop >/dev/null 2>&1 || true
  sleep 1
  m=$(mark); l1=$(load); start=$(now)
  "${clean_env[@]}" nohup herdr --session "$session" server </dev/null >/dev/null 2>&1 &
  restart=$(settled_ms "$m" "$start" 5)

  stop
  hd server stop >/dev/null 2>&1 || true
  sleep 1
  l2=$(load); start=$(now)
  launch
  wait_attached 0 $target launch 20
  stopped=$(settled_ms 0 "$start")
  printf 'run %d: herdr restart %d ms (load %s) | launch with herdr stopped %d ms (load %s)\n' \
    "$i" "$restart" "$l1" "$stopped" "$l2" | tee -a "$results"
done
python3 - "$results" <<'EOF'
import re, statistics, sys
rows = open(sys.argv[1]).read()
for label, pat in (("herdr restart", r"restart (\d+) ms"), ("launch, herdr stopped", r"stopped (\d+) ms")):
    v = [int(x) for x in re.findall(pat, rows)]
    print(f"{label:24} median {statistics.median(v):5.0f}  min {min(v):5d}  max {max(v):5d}  (n={len(v)})")
EOF
