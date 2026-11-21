#!/usr/bin/env bash
# File operations in the sidebar's Files view on Local (fops.azs.in): the
# right-click menu (captured drawn open, dark and light), New File and New
# Folder through the inline name field (captured too), names that are taken
# or invalid, Move to Trash (one and several), Copy and Paste through the
# private automation pasteboard, drops from outside (Finder's move, ⌥ copy,
# ⌘ move, refused into itself), a tree entry dragged onto a terminal, and
# `[editor]`: the command in a pane, `app` through the recording opener (no
# app ever launches) and its fallback when the app isn't there, for files
# and for Open Folder in Editor (a folder, the root on empty space, ⌘⇧O).
# Deletes go to a temp folder (AZJ_TRASH_DIR), never the user's Trash.
# Dev instance e2e-fops (herdr session azj-dev-e2e-fops), reset before and
# after; fixtures in a temp folder removed on exit.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-fops}" E2E_SUITE=fops
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null

fx="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/azj-fops.XXXXXX")" && pwd -P)"
mkdir -p "$fx/proj/src" "$fx/proj/docs" "$fx/outside/dir" "$fx/trash"
printf '# proj\n' >"$fx/proj/README.md"
printf 'fn a() {}\n' >"$fx/proj/src/a.rs"
printf 'old\n' >"$fx/proj/clash.txt"
printf 'in\n' >"$fx/outside/in.txt"
printf 'inner\n' >"$fx/outside/dir/inner.md"
printf 'drag\n' >"$fx/outside/drag.txt"
printf 'keep\n' >"$fx/outside/keep.txt"
printf 'forced\n' >"$fx/outside/forced.txt"
# A repository, so `workspace.set_repo` points the Files view at it.
git -C "$fx/proj" init -q -b main
git -C "$fx/proj" add . && git -C "$fx/proj" -c user.name=e2e -c user.email=e2e@example.invalid commit -qm init
export AZJ_TRASH_DIR="$fx/trash"

sed -e "s|@FX@|$fx|g" "$e2e_here/fops.azs.in" >"$out/fops.azs"
run_azs "$out/fops.azs" fops
check "nothing went to the user's Trash: every delete is in the test's" \
  test -f "$fx/trash/clash.txt"
! grep -q FAIL "$checks"
