#!/usr/bin/env bash
# The full sidebar's status icons sit on the middle of their rows' text
# (rowicon.azs.in) on dev instance e2e-ricon: every state (idle, working,
# waiting, done), rows with a branch (clean, dirty, detached) and without,
# one selected, hovered and with ⌘ held, in atom-one-dark and macos-light.
# Shots 01-dark, 02-dark-cmd, 03-light, 04-light-cmd. The workspaces'
# folders are fixtures under /tmp; the shown terminal runs a bare prompt.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-ricon}" E2E_SUITE=rowicon
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null

# Under /tmp, not $TMPDIR: the terminal and its header show the folder.
fx="$(mktemp -d /tmp/azj-ric.XXXXXX)"
for r in orbit-api:main lantern:feature/tide-charts pebble-docs:fix/typo-sweep kiln:main; do
  name="${r%%:*}" branch="${r#*:}"
  mkdir -p "$fx/$name"
  git -C "$fx/$name" init -q -b "$branch"
  printf '# %s\n' "$name" >"$fx/$name/README.md"
  git -C "$fx/$name" add . && git -C "$fx/$name" -c user.name=e2e -c user.email=e2e@example.invalid commit -qm init
done
# lantern's tree is dirty; kiln's HEAD is detached.
echo wip >>"$fx/lantern/README.md"
git -C "$fx/kiln" checkout -q --detach
mkdir -p "$fx/scratch" "$fx/notes"

sed -e "s|@FX@|$fx|g" "$e2e_here/rowicon.azs.in" >"$out/rowicon.azs"
run_azs "$out/rowicon.azs" rowicon
! grep -q FAIL "$checks"
