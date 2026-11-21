#!/usr/bin/env bash
# N1–N6 (e2e 11-04): the features the Nov 3 fixes added and where they meet.
#   - fixes.azs.in, in-process: ⇧⌘F under motion, a herdr restart, ⌘K and a tab
#     switch; the find bar at the minimum size; `azj files open` from a
#     terminal (three files, one of them new); HUD wording; agent state in
#     headers; the config banner row; tab overflow.
#   - then, through `azj` against the same app: error wording (message names
#     no IDs, the IDs are in the error data) and unknown params.
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

say "N3 error wording through azj (relaunch in the background)"
launch_bg >"$out/launch.txt"
wait_ping
sleep 2
tiles_json="$(azj --json tile list)"
echo "$tiles_json" >"$out/tiles.json"
term="$(jq -r '[.tiles[] | select(.kind=="terminal") | .id][0]' <<<"$tiles_json")"
e="$out/errors.txt"; : >"$e"
try() { # name, then the azj args
  local name="$1"; shift
  { echo "### $name: azj $*"; azj "$@" 2>&1; echo "exit=$?"; echo "### $name --json"; azj --json "$@" 2>&1; echo; } >>"$e" || true
}
try unknown-param call layout.split '{"direction":"right","kind":"terminal"}'
try nested-unknown call layout.split '{"direction":"right","content":{"kind":"terminal","cwd":"/tmp","bogus":1}}'
try no-params-method call core.ping '{"x":1}'
try no-such-tile tile focus --tile tile_01AAAAAAAAAAAAAAAAAAAAAAAA
try no-such-workspace workspace focus --workspace ws_01AAAAAAAAAAAAAAAAAAAAAAAA
try files-relative files open --tile "$term" --path README.md
try bad-value call layout.set '{"kind":"spiral"}'
try unknown-method call nope.nothing '{}'
cat "$e"
check "N3 unknown param named, invalid_params" grep -q 'unknown param `kind` for `layout.split`' "$e"
check "N3 unknown nested param named" grep -q 'unknown param `content.bogus`' "$e"
check "N3 a method with no params refuses one" bash -c "sed -n '/### no-params-method:/,/^exit/p' '$e' | grep -q 'exit=[1-9]'"
check "N3 no such tile" grep -q 'no such tile' "$e"
check "N3 no such workspace" grep -q 'no such workspace' "$e"
check "N3 a relative path without a panel is refused in words" bash -c "sed -n '/### files-relative:/,/^exit/p' '$e' | grep -q \"isn't absolute\""
check "N3 the errors carry the IDs as data after the message" bash -c "grep -A3 '### no-such-tile --json' '$e' | grep -q 'tile_01AAAA'"
# With --json, is the error itself JSON (code, message, data) for scripts?
check "N3 azj --json prints an error as a JSON object (code, message, data)" bash -c \
  "grep -A1 '### no-such-tile --json' '$e' | tail -1 | jq -e '.code == \"not_found\" and (.message | type == \"string\") and (.data | type == \"object\")' >/dev/null"
# Every human message (the line azj prints first) is free of raw IDs.
python3 - "$e" <<'PY' | tee -a "$checks"
import re, sys
text = open(sys.argv[1]).read()
bad = []
for block in re.split(r"^### ", text, flags=re.M):
    if not block or " --json" in block.split("\n", 1)[0]:
        continue
    lines = block.split("\n")[1:]
    first = next((l for l in lines if l.strip()), "")
    if not first.startswith("azj: "):
        continue  # not an error
    # azj prints the error's data (the IDs) after the message, in brackets.
    first = re.sub(r"\s\([^()]*\)$", "", first)
    if re.search(r"\b(tile|ws|tab|win)_[0-9A-Z]{20,}", first):
        bad.append(block.split(":", 1)[0] + ": " + first)
print("PASS N3 first error lines name no raw IDs" if not bad else "FAIL N3 raw IDs in: " + " | ".join(bad))
PY

stop_app
grep -c PASS "$checks" | sed 's/^/passed: /'; ! grep -q FAIL "$checks"
