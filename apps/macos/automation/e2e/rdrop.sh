#!/usr/bin/env bash
# Files dropped on a remote terminal (rdrop.azs.in), dev instance e2e-rdrop.
# The "remote" machine is a second throwaway herdr session reached only
# through the ssh stand-in (scripts/ssh-standin.sh), which also answers scp:
# the move zone names the pane's folder on the machine, a drop uploads the
# files there with scp (the originals go only after the upload), ⌥ copies
# with Finder's number on a name clash, and a cut link fails with the
# originals left alone. Nothing connects to a real machine.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-rdrop}" E2E_SUITE=rdrop
source "$(dirname "$0")/lib.sh"
side="$root/.dev/$AZJ_DEV_NAME.machines"
remote="$session-r"
id=0123456789abcdef0123456789abcdef

rdrop_cleanup() {
  e2e_cleanup
  [[ "${E2E_KEEP:-}" == 1 ]] && return 0
  "$e2e_here/hd" "$remote" server stop >/dev/null 2>&1 || true
  sleep 0.5
  "$e2e_here/hd" "$remote" session delete "$remote" >/dev/null 2>&1 || true
  rm -rf "$side"
}
trap rdrop_cleanup EXIT
reset_instance
rm -rf "$side"
mkdir -p "$side"

# Fixtures, resolved (herdr reports a shell's cwd with symlinks followed):
# local files to drop, and the remote pane's folder.
fx="$(mktemp -d "${TMPDIR:-/tmp}/azj-rdrop.XXXXXX")"
fx="$(cd "$fx" && pwd -P)"
mkdir -p "$fx/local" "$fx/remote-dest"
echo one >"$fx/local/one.txt"
echo two >"$fx/local/two.txt"
echo new >"$fx/local/clash.txt"
echo old >"$fx/remote-dest/clash.txt"

# The "remote" herdr: its own throwaway session, running.
nohup "$e2e_here/hd" "$remote" server >/dev/null 2>&1 &
for _ in $(seq 100); do "$e2e_here/hd" "$remote" workspace list >/dev/null 2>&1 && break; sleep 0.1; done

cp "$root/scripts/ssh-standin.sh" "$side/ssh"
cat >"$side/machines.json" <<EOF
[{"id":"$id","label":"standin","target":"standin-host","session":"$remote","enabled":true,"selected":false}]
EOF
write_config <<EOF
machines = true
machines_file = "$side/machines.json"
ssh = "$side/ssh"
EOF

sed -e "s|@FX@|$fx|g" -e "s|@SIDE@|$side|g" "$e2e_here/rdrop.azs.in" >"$out/rdrop.azs"
run_azs "$out/rdrop.azs" rdrop
grep -E '^ *[0-9]+ # ===|FAIL|line [0-9]+:' "$out/rdrop/automation.log" | tee "$out/scenarios.txt" || true
! grep -q FAIL "$checks"
