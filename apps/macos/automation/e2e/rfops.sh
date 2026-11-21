#!/usr/bin/env bash
# File operations in a remote workspace's Files view (rfops.azs.in), dev
# instance e2e-rfops. The "remote" machine is a second throwaway herdr
# session reached only through the ssh stand-in (scripts/ssh-standin.sh):
# New File and New Folder over SSH, Delete behind its confirmation bar
# (captured, dark and light) and rm -rf, Copy (a download into the app's
# temp folder) and Paste (an upload), a drop that uploads, a remote entry's
# file promise (a download where it's dropped), the entry dragged onto the
# machine's terminal (its remote path), the editor command running on the
# machine with an `[editor] app` that isn't Zed, and a fake Zed (a bundle
# with a `dev.zed.Zed-*` ID) getting the file as a `zed://ssh/…` link that
# the automation opener only records; Open Folder in Editor the same way
# (`command <folder>` in a pane there, Zed's `zed://ssh/…/folder`
# for a folder, a file's folder and the root). Nothing connects to a real machine
# and nothing launches; fixtures live in a temp folder removed on exit.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-rfops}" E2E_SUITE=rfops
source "$(dirname "$0")/lib.sh"
side="$root/.dev/$AZJ_DEV_NAME.machines"
remote="$session-r"
id=0123456789abcdef0123456789abcdef

rfops_cleanup() {
  e2e_cleanup
  [[ "${E2E_KEEP:-}" == 1 ]] && return 0
  "$e2e_here/hd" "$remote" server stop >/dev/null 2>&1 || true
  sleep 0.5
  "$e2e_here/hd" "$remote" session delete "$remote" >/dev/null 2>&1 || true
  rm -rf "$side"
}
trap rfops_cleanup EXIT
reset_instance
rm -rf "$side"
mkdir -p "$side"

# Fixtures, resolved (herdr reports a shell's cwd with symlinks followed).
fx="$(mktemp -d "${TMPDIR:-/tmp}/azj-e2e.XXXXXX")"
fx="$(cd "$fx" && pwd -P)"
mkdir -p "$fx/remote-proj/src" "$fx/remote-proj/docs" "$fx/local" "$fx/promised" "$fx/trash"
printf 'a\n' >"$fx/remote-proj/alpha.txt"
printf '# Notes\n' >"$fx/remote-proj/notes.md"
printf 'pub fn f() {}\n' >"$fx/remote-proj/src/lib.rs"
printf 'odd\n' >"$fx/remote-proj/src/a b&c%.md"
printf 'up\n' >"$fx/local/up.txt"
printf 'drag\n' >"$fx/local/drag.txt"
# A stand-in for Zed: an app bundle by its ID alone (no executable, no URL
# schemes, an ID of its own so it never stands for the real Zed).
mkdir -p "$fx/apps/Fake Editor.app/Contents"
cat >"$fx/apps/Fake Editor.app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.zed.Zed-E2E</string>
<key>CFBundleName</key><string>Fake Editor</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
EOF
# Remote deletes never use a Trash; this one stays empty.
export AZJ_TRASH_DIR="$fx/trash"

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

sed -e "s|@FX@|$fx|g" -e "s|@SIDE@|$side|g" -e "s|@ID@|$id|g" "$e2e_here/rfops.azs.in" >"$out/rfops.azs"
run_azs "$out/rfops.azs" rfops
check "remote deletes are for good: nothing in a Trash" test -z "$(ls -A "$fx/trash")"
! grep -q FAIL "$checks"
