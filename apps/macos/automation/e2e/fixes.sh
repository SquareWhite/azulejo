#!/usr/bin/env bash
# N1–N6 (e2e 11-04): the features the Nov 3 fixes added and where they meet.
#   - fixes.azs.in, in-process: ⇧⌘F under motion, a herdr restart, ⌘K and a tab
#     switch; the find bar at the minimum size; `azj files open` from a
#     terminal (three files, one of them new); HUD wording; agent state in
#     headers; the config banner row; tab overflow.
#   - azj's error wording (message names no IDs, the IDs are in the error
#     data; unknown params) moved to core/crates/cli/tests/cli.rs
#     (`error_wording_names_the_field_and_no_ids`), which needs no app.
# Dev instance e2e-fix (herdr session azj-dev-e2e-fix), reset before and after.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-fix}" E2E_SUITE=N-fixes
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
make_fixtures
write_config </dev/null

o="$out/fixes"
sed -e "s|@FX@|$fx|g" -e "s|@AZJ@|$azj_bin|g" -e "s|@OUT@|$o|g" -e "s|@HD@|$e2e_here/hd|g" \
  -e "s|@HERE@|$e2e_here|g" "$e2e_here/fixes.azs.in" >"$out/fixes.azs"
run_azs "$out/fixes.azs" fixes
grep -E ' remember |^FAIL|line [0-9]+:' "$o/automation.log" | tee "$out/remembered.txt" || true

say "N2 azj files open from a terminal (inside the tile)"
for i in 1 2 3; do echo "--- open$i:"; cat "$o/open$i.out" 2>/dev/null || echo missing; done | tee "$out/files-open.txt"
check "N2 first files open from a terminal exits 0" grep -q '^exit=0$' "$o/open1.out"
check "N2 second files open from the same terminal exits 0" grep -q '^exit=0$' "$o/open2.out"
# A file that isn't there yet opens too: the editor starts a new one.
check "N2 files open of a new file opens the editor (exit 0)" grep -q '^exit=0$' "$o/open3.out"
check "N2 three opens from one terminal make three editor tiles" grep -q 'remember tiles = 4' "$o/automation.log"

grep -c PASS "$checks" | sed 's/^/passed: /'; ! grep -q FAIL "$checks"
