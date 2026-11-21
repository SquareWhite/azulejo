#!/usr/bin/env bash
# Stability and security checks on the dev instance `nfr`.
#
# Stability: two terminals running (one a ticker) keep their attach
# processes and the ticker keeps counting.
# Security: API socket 0600, its directory 0700, owned by the user.
#
#   scripts/nfr/stability.sh
set -euo pipefail
source "$(dirname "$0")/lib.sh"
cargo build -q -p azulejo-cli
reset_all
herdr_fresh

descendants() { local p c; for p in "$@"; do for c in $(pgrep -P "$p" || true); do echo "$c"; descendants "$c"; done; done; }
tick() { azj --json terminal read --tile "$1" --source recent --lines 5 | jq -r '.text' | { grep -oE 'tick [0-9]+' || true; } | tail -1 | cut -d' ' -f2; }

say "two terminals, one a ticker"
launch
wait_ping
wait_tiles 1
azj layout split-auto >/dev/null
for _ in $(seq 100); do [[ $(grep -c "phase attached" "$log") -ge 2 ]] && break; sleep 0.1; done
t1="$(azj --json tile list | jq -r '[.tiles[] | select(.kind=="terminal")][0].id')"
azj terminal send-text --tile "$t1" --text $'sh -c \'i=0; while :; do i=$((i+1)); echo tick $i; sleep 0.2; done\'\n' >/dev/null
pid="$(running_pid)"

say "security: socket and directory permissions"
sock_mode="$(stat -f '%Lp %Su' "$sock")"
dir_mode="$(stat -f '%Lp %Su' "$(dirname "$sock")")"
echo "   socket $sock_mode, dir $dir_mode"
[[ ${sock_mode%% *} == 600 ]] || fail "socket mode $sock_mode"
[[ ${dir_mode%% *} == 700 ]] || fail "socket dir mode $dir_mode"

attach_before="$(descendants "$pid" | sort | tr '
' ' ')"
tick_before="$(tick "$t1")"
echo "   attach processes: $attach_before"
echo "   ticker at $tick_before"
sleep 4

say "terminals unaffected?"
kill -0 "$pid" 2>/dev/null || fail "the app died"
attach_after="$(descendants "$pid" | sort | tr '\n' ' ')"
[[ $attach_before == "$attach_after" ]] || fail "attach processes changed: $attach_before → $attach_after"
tick_after="$(tick "$t1")"
[[ -n $tick_after && $tick_after -gt $tick_before ]] || fail "ticker stopped ($tick_before → ${tick_after:-none})"
echo "   app alive, same attach processes, ticker $tick_before → $tick_after"
azj --json tile list | jq -r '.tiles[] | "   \(.kind) \(.id[-4:])"'
stop
echo
echo "PASS"
