#!/usr/bin/env bash
# Remote files panel (ADR 0015, rfiles.azs.in), dev instance e2e-rfile. The
# "remote" machine is a second throwaway herdr session reached only through
# the ssh stand-in (scripts/ssh-standin.sh): the panel lists its files over
# that "SSH", follows the remote shell's cd, opens folders and a preview,
# and shows the error while the link is cut. Nothing connects to a real
# machine.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-rfile}" E2E_SUITE=rfiles
source "$(dirname "$0")/lib.sh"
side="$root/.dev/$AZJ_DEV_NAME.machines"
remote="$session-r"
id=0123456789abcdef0123456789abcdef

rfiles_cleanup() {
  e2e_cleanup
  [[ "${E2E_KEEP:-}" == 1 ]] && return 0
  "$e2e_here/hd" "$remote" server stop >/dev/null 2>&1 || true
  sleep 0.5
  "$e2e_here/hd" "$remote" session delete "$remote" >/dev/null 2>&1 || true
  rm -rf "$side"
}
trap rfiles_cleanup EXIT
reset_instance
rm -rf "$side"
mkdir -p "$side"

# Fixtures, resolved (herdr reports a shell's cwd with symlinks followed).
fx="$(mktemp -d "${TMPDIR:-/tmp}/azj-e2e.XXXXXX")"
fx="$(cd "$fx" && pwd -P)"
mkdir -p "$fx/remote-proj/src" "$fx/remote-proj/docs" "$fx/other"
printf 'a\n' >"$fx/remote-proj/alpha.txt"
printf '# Notes\n\nOn the remote machine.\n' >"$fx/remote-proj/notes.md"
printf 'pub fn f() {}\n' >"$fx/remote-proj/src/lib.rs"
printf '# Beta\n' >"$fx/other/beta.md"

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

sed -e "s|@FX@|$fx|g" -e "s|@SIDE@|$side|g" -e "s|@ID@|$id|g" "$e2e_here/rfiles.azs.in" >"$out/rfiles.azs"
run_azs "$out/rfiles.azs" rfiles
! grep -q FAIL "$checks"
