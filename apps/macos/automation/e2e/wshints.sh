#!/usr/bin/env bash
# ⌘ held alone shows each workspace row's chord in the full sidebar
# (WorkspaceHints) on dev instance e2e-wsh: wshints.azs checks the numbers
# against the rows, the rail and the chords across two machine groups and
# past nine workspaces, the wait before they show, what hides them, the
# rail left alone, the sidebar opened by ⌘A with ⌘ kept down (badges once
# its motion ends; nothing with ⌘ up or ⌘⇧), and motion off and Reduce
# Motion fading nothing. Shots of
# the sidebar with the badges in a dark and a light theme (01-hints-dark,
# 02-hints-light, 03-hints-ten). The "remote" machine, bravo, is a throwaway
# local herdr session reached only through the ssh stand-in
# (scripts/ssh-standin.sh); the Local workspaces' repositories are fixtures.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-wsh}" E2E_SUITE=wshints
source "$(dirname "$0")/lib.sh"
side="$root/.dev/$AZJ_DEV_NAME.machines"
rb="$session-rb"
bravo=b0000000000000000000000000000001

wshints_cleanup() {
  e2e_cleanup
  [[ "${E2E_KEEP:-}" == 1 ]] && return 0
  "$e2e_here/hd" "$rb" server stop >/dev/null 2>&1 || true
  sleep 0.5
  "$e2e_here/hd" "$rb" session delete "$rb" >/dev/null 2>&1 || true
  rm -rf "$side"
}
trap wshints_cleanup EXIT
reset_instance
rm -rf "$side"
mkdir -p "$side"

# Fixture repositories, one per Local workspace, each on its own branch.
# Under /tmp, not $TMPDIR: the terminal and its header show the folder.
fx="$(mktemp -d /tmp/azj-wsh.XXXXXX)"
branches=(main feature/sidebar-hints fix/rail-numbers release/2.0 main main main main main)
for i in 1 2 3 4 5 6 7 8 9; do
  r="$fx/l$i"
  mkdir -p "$r"
  git -C "$r" init -q -b "${branches[$((i - 1))]}"
  printf '# l%s\n' "$i" >"$r/README.md"
  git -C "$r" add . && git -C "$r" -c user.name=e2e -c user.email=e2e@example.invalid commit -qm init
done

# bravo's herdr: its own throwaway session, running, empty.
nohup "$e2e_here/hd" "$rb" server >/dev/null 2>&1 &
for _ in $(seq 100); do "$e2e_here/hd" "$rb" workspace list >/dev/null 2>&1 && break; sleep 0.1; done

cp "$root/scripts/ssh-standin.sh" "$side/ssh"
chmod +x "$side/ssh"
printf '[{"id":"%s","label":"bravo","target":"bravo-host","session":"%s","enabled":true,"selected":false}]\n' \
  "$bravo" "$rb" >"$side/machines.json"
write_config <<EOF
machines = true
machines_file = "$side/machines.json"
ssh = "$side/ssh"
EOF

sed -e "s|@FX@|$fx|g" "$e2e_here/wshints.azs.in" >"$out/wshints.azs"
run_azs "$out/wshints.azs" wshints
! grep -q FAIL "$checks"
