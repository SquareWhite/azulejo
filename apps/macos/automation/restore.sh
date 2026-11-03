#!/usr/bin/env bash
# PLAN 1.12: session restore and wake, measured on the dev instance `restore`
# (herdr session azj-dev-restore). No OS input, no window activation: the app
# runs with --bg, wake is a distributed notification to the app's debug hook.
#
#   apps/macos/automation/restore.sh        (build first: just mac)
#
# Steps: build a session (6 terminals in a tall layout, a second tab with a
# browser and a zoomed preview, sidebar hidden) → quit + relaunch → kill -9 +
# relaunch → kill every attach process + wake → stop + restart herdr → launch
# with herdr stopped. Each step checks the session came back and times how
# long until every terminal tile is attached again (target ≤ 2 s).
set -euo pipefail

root="$(cd "$(dirname "$0")/../../.." && pwd)"
export AZJ_DEV_NAME=restore
dev="$root/.dev/restore"
sock="$dev/run/azulejo.sock"
log="$dev/logs/app.log"
out="$dev/out/restore"
session=azj-dev-restore
dev_sh="$root/apps/macos/dev.sh"
target=6

clean_env=(env -u HERDR_SOCKET_PATH -u HERDR_PANE_ID -u HERDR_TAB_ID -u HERDR_WORKSPACE_ID -u HERDR_ENV -u AZULEJO_TILE_ID -u AZULEJO_WORKSPACE_ID)
azj() { "${clean_env[@]}" AZULEJO_SOCKET="$sock" "$root/target/debug/azj" --no-caller "$@"; }
hd() { "${clean_env[@]}" herdr --session "$session" "$@"; }
say() { printf '\n== %s\n' "$*"; }
fail() { echo "FAIL: $*" >&2; exit 1; }

cargo build -q -p azulejo-cli
"$dev_sh" --reset >/dev/null
mkdir -p "$out"
notify="$out/notify"
swiftc -O -o "$notify" "$root/apps/macos/automation/notify.swift" 2>/dev/null

launch() {
  "$dev_sh" --bg >/dev/null
  for _ in $(seq 100); do azj core ping >/dev/null 2>&1 && return 0; sleep 0.05; done
  fail "app didn't answer on $sock"
}

# Waits for `n` "attached N ms after <reason> at T" lines after line `from` of the
# log; prints the slowest. Fails after `timeout` seconds.
wait_attached() {
  local from=$1 n=$2 reason=$3 timeout=${4:-15} lines
  for _ in $(seq $((timeout * 20))); do
    lines="$(tail -n +"$((from + 1))" "$log" | grep -E "attached [0-9]+ ms after $reason at" || true)"
    if [[ $(printf '%s' "$lines" | grep -c . || true) -ge $n ]]; then
      printf '%s\n' "$lines" | sed -E 's/.*attached ([0-9]+) ms.*/\1/' | sort -n | tail -1
      return 0
    fi
    sleep 0.05
  done
  echo "--- log tail" >&2; tail -40 "$log" >&2
  fail "only $(printf '%s' "$lines" | grep -c . || true)/$n terminals attached after $reason"
}
mark() { wc -l <"$log" | tr -d ' '; }

# "attached" is logged when the attach process starts; one that fails at once
# (herdr down) logs "attach ended" right after. Settled = every tile's last
# attach is still running after a quiet period. Prints the latest such
# attach time, as epoch seconds.
settled() {
  local from=$1 quiet=${2:-2} t lines tiles last=0 tile at
  sleep "$quiet"
  lines="$(tail -n +"$((from + 1))" "$log")"
  tiles="$(printf '%s\n' "$lines" | grep -oE 'terminal [A-Z0-9]{4}: attached' | cut -d' ' -f2 | tr -d : | sort -u)"
  for tile in $tiles; do
    at="$(printf '%s\n' "$lines" | grep -E "terminal $tile: attached" | tail -1 | sed -E 's/.* at ([0-9.]+)$/\1/')"
    # Anything ending after the last attach means it didn't hold.
    if printf '%s\n' "$lines" | awk -v a="terminal $tile: attached " -v e="terminal $tile: attach ended" '
        index($0, a) { seen = NR }
        index($0, e) { ended = NR }
        END { exit !(ended > seen) }'; then
      fail "terminal $tile didn't stay attached"
    fi
    last="$(python3 -c "print(max($last, $at))")"
  done
  echo "$last"
}

# The session as saved, minus what may legitimately move between runs.
# Live fields (pane title/cwd/command, headers, branches) aren't layout.
snapshot() { azj --json session get | jq -S 'del(.. | .frame?, .pane?, .header?, .git?)'; }
terminal_ids() { hd pane list 2>/dev/null | grep -oE 'term_[A-Za-z0-9]+' | sort -u; }
tick() { azj --json terminal read --tile "$1" --source recent --lines 5 | jq -r '.text' | { grep -oE 'tick [0-9]+' || true; } | tail -1 | cut -d' ' -f2; }

now() { python3 -c 'import time; print("%.3f" % time.time())'; }
ms_since() { python3 -c "print(int(($2 - $1) * 1000))"; }
results=()
record() { results+=("$1: $2 ms"); echo "   $1: $2 ms"; }

say "build the session"
launch
m=$(mark)
for _ in 1 2 3 4 5; do azj layout split-auto >/dev/null; done
wait_attached 0 $target "(launch|new tile)" >/dev/null
tiles=($(azj --json tile list | jq -r '.tiles[] | select(.kind=="terminal") | .id'))
[[ ${#tiles[@]} -eq $target ]] || fail "expected $target terminals, have ${#tiles[@]}"
first_tab="$(azj --json session get | jq -r '.session.windows[0].workspaces[0].active_tab')"
plan="$dev/plan.md"
printf '# Plan\n\n- [x] restore\n- [ ] wake\n' >"$plan"
azj tab create --kind browser --url "http://127.0.0.1:9/restore-check" >/dev/null
azj layout split-auto --kind preview --path "$plan" >/dev/null
preview="$(azj --json tile list | jq -r '.tiles[] | select(.kind=="preview") | .id')"
azj layout zoom --tile "$preview" --zoomed true >/dev/null
azj tab focus --tab "$first_tab" >/dev/null
window="$(azj --json window list | jq -r '.windows[0].id')"
azj window set-sidebar --window "$window" --visible false >/dev/null
# A process that must survive everything: a ticker in the first terminal.
azj terminal send-text --tile "${tiles[0]}" --text $'sh -c \'i=0; while :; do i=$((i+1)); echo tick $i; sleep 0.2; done\'\n' >/dev/null
sleep 1.5
snapshot >"$out/before.json"
terminal_ids >"$out/terms-before.txt"
t0="$(tick "${tiles[0]}")"
echo "   $target terminals, browser + zoomed preview in tab 2, sidebar hidden, ticker at $t0"

check_same() {
  local label=$1
  snapshot >"$out/after-$label.json"
  diff -u "$out/before.json" "$out/after-$label.json" >"$out/diff-$label.txt" \
    || fail "$label: session differs (see $out/diff-$label.txt)"
  terminal_ids >"$out/terms-$label.txt"
  diff -q "$out/terms-before.txt" "$out/terms-$label.txt" >/dev/null \
    || fail "$label: herdr terminals changed"
  local t
  t="$(tick "${tiles[0]}")"
  [[ -n "$t" && "$t" -gt "$t0" ]] || fail "$label: ticker didn't keep running ($t0 → ${t:-none})"
  t0=$t
  echo "   session identical, same $(wc -l <"$out/terms-$label.txt" | tr -d ' ') herdr terminals, ticker at $t"
}

say "quit (SIGTERM) and relaunch"
"$dev_sh" --stop
sleep 0.5
start=$(now); launch
wait_attached 0 $target launch >/dev/null
record "quit/relaunch (launch → all attached and holding)" "$(ms_since "$start" "$(settled 0)")"
check_same relaunch

say "kill -9 and relaunch"
kill -9 "$(cat "$dev/run/app.pid")"
sleep 0.5
rm -f "$dev/run/app.pid"
start=$(now); launch
wait_attached 0 $target launch >/dev/null
record "kill -9/relaunch (launch → all attached and holding)" "$(ms_since "$start" "$(settled 0)")"
check_same kill9

say "attach processes die during 'sleep'; wake"
m=$(mark)
start=$(now)
pkill -9 -f "herdr --session $session terminal attach" || fail "no attach processes"
"$notify" dev.azulejo.debug.terminal "wake all"
wait_attached "$m" $target "(wake \(debug\)|attach ended)" >/dev/null
record "wake (kill → all attached and holding)" "$(ms_since "$start" "$(settled "$m")")"
check_same wake

say "attach processes die while awake (child-exit path alone), then a stray wake"
m=$(mark)
start=$(now)
pkill -9 -f "herdr --session $session terminal attach"
wait_attached "$m" $target "(wake \(debug\)|attach ended)" >/dev/null
end="$(settled "$m" 3)"
"$notify" dev.azulejo.debug.terminal "wake all"
record "attach killed, no wake (kill → all attached and holding)" "$(ms_since "$start" "$end")"
check_same late-wake

say "herdr blip: stop the server, start it again"
m=$(mark)
hd server stop >/dev/null 2>&1 || true
sleep 1
m=$(mark)
start=$(now)
"${clean_env[@]}" nohup herdr --session "$session" server </dev/null >/dev/null 2>&1 &
end="$(settled "$m" 5)"
[[ "$end" != 0 ]] || fail "no terminal reattached after herdr came back"
record "herdr restart (server start → all reattached and holding)" "$(ms_since "$start" "$end")"
n="$(tail -n +"$((m + 1))" "$log" | grep -oE 'terminal [A-Z0-9]{4}: attached' | sort -u | wc -l | tr -d ' ')"
[[ "$n" == "$target" ]] || fail "only $n/$target terminals reattached after herdr came back"
grep -E "herdr (connected|disconnected)" "$log" | tail -3 | sed 's/^/   log: /'
echo "   (herdr restores layout but not processes: panes respawn with new terminals, ADR 0008)"

say "launch with herdr stopped"
"$dev_sh" --stop
hd server stop >/dev/null 2>&1 || true
sleep 1
start=$(now); launch
wait_attached 0 $target launch 20 >/dev/null
end="$(settled 0 3)"
n="$(grep -oE 'terminal [A-Z0-9]{4}: attached' "$log" | sort -u | wc -l | tr -d ' ')"
[[ "$n" == "$target" ]] || fail "only $n/$target terminals attached"
record "launch with herdr stopped (launch → all attached and holding)" "$(ms_since "$start" "$end")"
[[ "$(azj --json session get | jq '[.. | objects | select(.kind?=="terminal" and .ended==true)] | length')" == 0 ]] \
  || fail "tiles ended after herdr came back"

say "results"
printf '   %s\n' "${results[@]}" | tee "$out/results.txt"
echo "   evidence: $out"
