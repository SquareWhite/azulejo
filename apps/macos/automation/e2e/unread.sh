#!/usr/bin/env bash
# Mark as unread and the Dock badge on dev instance e2e-unr: unread.azs
# marks agent tiles unread from the header's menu, ⌘K and the keymap
# action, checks the mark looks like an unread finish everywhere, survives
# status changes and stays while its tile stays focused, then clears once
# the user comes back to the tile; the Dock badge counts unread agent tiles
# across two workspaces, empties at 0 and follows `[appearance] dock_badge`.
# unread-relaunch.azs: the mark comes back after a relaunch. Agents are
# faked with `herdr pane report-agent`; every folder is a fixture.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-unr}" E2E_SUITE=unread
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null
# A fixture folder per workspace (terminals, headers and the sidebar show
# these, never the real home folder); e2e_cleanup removes it.
fx="$(mktemp -d "${TMPDIR:-/tmp}/azj-unr.XXXXXX")"
for w in alpha bravo; do
  mkdir -p "$fx/$w"
  printf '# %s\n' "$w" >"$fx/$w/README.md"
done
sed "s|@FX@|$fx|g" "$e2e_here/unread.azs" >"$out/unread.azs"
sed "s|@FX@|$fx|g" "$e2e_here/unread-relaunch.azs" >"$out/unread-relaunch.azs"

# How many agents in the saved session carry an unread mark.
saved_marks() { python3 -c "
import json, sys
def agents(v):
    if isinstance(v, dict):
        if isinstance(v.get('agent'), dict): yield v['agent']
        for x in v.values(): yield from agents(x)
    elif isinstance(v, list):
        for x in v: yield from agents(x)
print(sum(1 for a in agents(json.load(open('$state'))) if a.get('unread')))"; }

run_azs "$out/unread.azs" unread
check "the saved session carries one unread mark" test "$(saved_marks)" = 1
say "relaunch keeps the mark"
run_azs "$out/unread-relaunch.azs" relaunch
check "coming back after the relaunch read it" test "$(saved_marks)" = 0
! grep -q FAIL "$checks"
