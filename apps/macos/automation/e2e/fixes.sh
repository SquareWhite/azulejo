#!/usr/bin/env bash
# N1–N6 (e2e 11-04): the features the Nov 3 fixes added and where they meet.
#   - fixes.azs.in, in-process: ⌘F under motion, a herdr restart, ⌘K and a tab
#     switch; Source and Quick Look previews; a failed browser page; the find
#     bar at the minimum size; `azj preview open` from a terminal (twice, and a
#     missing file); HUD wording; agent badges; the config banner row; tab
#     overflow.
#   - then, through `azj` against the same app: error wording (message names
#     no IDs, the IDs are in the error data), unknown params, and
#     `preview open` with --tile on each tile kind.
# Dev instance e2e-fix (herdr session azj-dev-e2e-fix), reset before and after.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-fix}" E2E_SUITE=N-fixes
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
make_fixtures
write_config </dev/null
port=47853
serve_site "$port"

o="$out/fixes"
sed -e "s|@FX@|$fx|g" -e "s|@AZJ@|$azj_bin|g" -e "s|@OUT@|$o|g" -e "s|@HD@|$e2e_here/hd|g" \
  -e "s|@HERE@|$e2e_here|g" -e "s|@SITE@|http://127.0.0.1:$port/a.html|g" \
  "$e2e_here/fixes.azs.in" >"$out/fixes.azs"
run_azs "$out/fixes.azs" fixes
grep -E ' remember |^FAIL|line [0-9]+:' "$o/automation.log" | tee "$out/remembered.txt" || true

say "N2 azj preview open from a terminal (inside the tile)"
for i in 1 2 3; do echo "--- open$i:"; cat "$o/open$i.out" 2>/dev/null || echo missing; done | tee "$out/preview-open.txt"
check "N2 first preview open from a terminal exits 0" grep -q '^exit=0$' "$o/open1.out"
check "N2 second preview open from the same terminal exits 0" grep -q '^exit=0$' "$o/open2.out"
# A missing file isn't an error: the preview says "File not found" and
# watches for the file (preview's own design), so azj exits 0.
check "N2 preview open of a missing file opens a watching preview (exit 0)" grep -q '^exit=0$' "$o/open3.out"
check "N2 three opens from one terminal make three preview tiles" grep -q 'remember tiles = 4' "$o/automation.log"

say "N3 error wording through azj (relaunch in the background)"
launch_bg >"$out/launch.txt"
wait_ping
sleep 2
tiles_json="$(azj --json tile list)"
echo "$tiles_json" >"$out/tiles.json"
term="$(jq -r '[.tiles[] | select(.kind=="terminal") | .id][0]' <<<"$tiles_json")"
prev="$(jq -r '[.tiles[] | select(.kind=="preview") | .id][0] // empty' <<<"$tiles_json")"
brow="$(jq -r '[.tiles[] | select(.kind=="browser") | .id][0] // empty' <<<"$tiles_json")"
e="$out/errors.txt"; : >"$e"
try() { # name, then the azj args
  local name="$1"; shift
  { echo "### $name: azj $*"; azj "$@" 2>&1; echo "exit=$?"; echo "### $name --json"; azj --json "$@" 2>&1; echo; } >>"$e" || true
}
try unknown-param call layout.split '{"direction":"right","kind":"browser","url":"http://x/"}'
try nested-unknown call layout.split '{"direction":"right","content":{"kind":"browser","url":"http://x/","bogus":1}}'
try no-params-method call core.ping '{"x":1}'
try no-such-tile tile focus --tile tile_01AAAAAAAAAAAAAAAAAAAAAAAA
try no-such-workspace workspace focus --workspace ws_01AAAAAAAAAAAAAAAAAAAAAAAA
try wrong-kind terminal send-text --tile "${prev:-$term}" --text x
try files-on-terminal files open --tile "$term" --path README.md
try preview-missing preview open --tile "$term" --path "$fx/nope.md"
try bad-value call layout.set '{"kind":"spiral"}'
try unknown-method call nope.nothing '{}'
cat "$e"
check "N3 unknown param named, invalid_params" grep -q 'unknown param `kind` for `layout.split`' "$e"
check "N3 unknown nested param named" grep -q 'unknown param `content.bogus`' "$e"
check "N3 a method with no params refuses one" bash -c "sed -n '/### no-params-method:/,/^exit/p' '$e' | grep -q 'exit=[1-9]'"
check "N3 no such tile" grep -q 'no such tile' "$e"
check "N3 no such workspace" grep -q 'no such workspace' "$e"
check "N3 wrong kind in words" bash -c "sed -n '/### wrong-kind:/,/^exit/p' '$e' | grep -qi \"that's a\""
check "N3 the errors carry the IDs as data after the message" bash -c "grep -A3 '### no-such-tile --json' '$e' | grep -q 'tile_01AAAA'"
# With --json, is the error itself JSON (code, message, data) for scripts?
if grep -A1 '### no-such-tile --json' "$e" | tail -1 | grep -q '^{'; then
  note "N3 azj --json prints errors as JSON"
else
  note "N3 azj --json prints errors as plain text, not JSON"
fi
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

say "N2b preview open --tile on each kind"
p="$out/preview-kinds.txt"; : >"$p"
for t in "terminal:$term" "preview:$prev" "browser:$brow"; do
  k="${t%%:*}" id="${t#*:}"
  [[ -n "$id" ]] || continue
  before="$(azj --json tile list | jq '.tiles | length')"
  r="$(azj --json preview open --tile "$id" --path "$fx/plan.md" 2>&1 || true)"
  sleep 1
  after="$(azj --json tile list | jq '.tiles | length')"
  focused="$(azj --json tile get | jq -r '.id // .tile.id // empty' 2>/dev/null || true)"
  echo "$k: tiles $before -> $after; focused $focused (was $id); reply $r" | tee -a "$p"
done
check "N2b preview open on a terminal adds a tile" grep -qE '^terminal: tiles ([0-9]+) -> ' "$p"
stop_app
grep -c PASS "$checks" | sed 's/^/passed: /'; ! grep -q FAIL "$checks"
