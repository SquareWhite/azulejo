#!/usr/bin/env bash
# Workspace numbers across machine groups (ADR 0013) on dev instance e2e-wsn:
# workspaces made Local, on a remote, then Local again are numbered top to
# bottom across the groups in the full sidebar, the rail, the core's order
# and `azj workspace list`, and ⌘N goes to the row showing N: through a
# drag, a close, adoption, a relaunch, a machine reconnecting and a machine
# leaving herdr's list and coming back (wsnum.azs, wsnum-relaunch.azs).
# The two "remote" machines, bravo and tango, are throwaway local herdr
# sessions reached only through the ssh stand-in (scripts/ssh-standin.sh);
# nothing connects to a real machine.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-wsn}" E2E_SUITE=wsnum
source "$(dirname "$0")/lib.sh"
side="$root/.dev/$AZJ_DEV_NAME.machines"
rb="$session-rb"
rt="$session-rt"
bravo=b0000000000000000000000000000001
tango=c0000000000000000000000000000002

wsnum_cleanup() {
  e2e_cleanup
  [[ "${E2E_KEEP:-}" == 1 ]] && return 0
  local s
  for s in "$rb" "$rt"; do "$e2e_here/hd" "$s" server stop >/dev/null 2>&1 || true; done
  sleep 0.5
  for s in "$rb" "$rt"; do "$e2e_here/hd" "$s" session delete "$s" >/dev/null 2>&1 || true; done
  rm -rf "$side"
}
trap wsnum_cleanup EXIT
reset_instance
rm -rf "$side"
mkdir -p "$side"

# The "remote" herdrs: their own throwaway sessions, running, empty.
for s in "$rb" "$rt"; do
  nohup "$e2e_here/hd" "$s" server >/dev/null 2>&1 &
  for _ in $(seq 100); do "$e2e_here/hd" "$s" workspace list >/dev/null 2>&1 && break; sleep 0.1; done
done

# One ssh for both machines: the stand-in, except that tango's host is
# unreachable while `tango.fail` exists.
cp "$root/scripts/ssh-standin.sh" "$side/standin"
cat >"$side/ssh" <<'EOF'
#!/bin/sh
here="$(dirname "$0")"
for a in "$@"; do
  if [ "$a" = tango-host ] && [ -e "$here/tango.fail" ]; then
    echo "ssh: connect to host tango-host port 22: Connection refused" >&2
    exit 255
  fi
done
exec "$here/standin" "$@"
EOF
chmod +x "$side/ssh" "$side/standin"
profile() { printf '{"id":"%s","label":"%s","target":"%s-host","session":"%s","enabled":true,"selected":false}' "$@"; }
printf '[%s,\n %s]\n' "$(profile "$tango" tango tango "$rt")" "$(profile "$bravo" bravo bravo "$rb")" \
  >"$side/machines-all.json"
printf '[%s]\n' "$(profile "$tango" tango tango "$rt")" >"$side/machines-tango.json"
cp "$side/machines-all.json" "$side/machines.json"
write_config <<EOF
machines = true
machines_file = "$side/machines.json"
ssh = "$side/ssh"
EOF

render() { sed -e "s|@SIDE@|$side|g" -e "s|@RB@|$rb|g" -e "s|@RT@|$rt|g" "$e2e_here/$1.azs.in" >"$out/$1.azs"; }
render wsnum
render wsnum-relaunch
run_azs "$out/wsnum.azs" wsnum

say "relaunch keeps the numbers; machines coming and going regroup them"
run_azs "$out/wsnum-relaunch.azs" relaunch

say "azj workspace list numbers them the same way"
launch_bg >"$out/launch.txt"
wait_ping
want="[('l2', 1), ('l1', 2), ('l4', 3), ('l5', 4), ('l6', 5), ('adopted', 6), ('b1', 7), ('adopted-b', 8), ('t1', 9)]"
listed() {
  azj --json workspace list >"$out/list.json" 2>/dev/null || return 1
  python3 -c "
import json; w = json.load(open('$out/list.json'))['workspaces']
print([(x['name'], x['number']) for x in w])"
}
for _ in $(seq 100); do [[ "$(listed)" == "$want" ]] && break; sleep 0.2; done
listed >"$out/list.txt" || true
check "workspace.list numbers follow the sidebar's groups" test "$(cat "$out/list.txt")" = "$want"
stop_app
! grep -q FAIL "$checks"
