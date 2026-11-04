#!/usr/bin/env bash
# B7 (terminal at a path), B10 (changes made in herdr), B14 (API, CLI,
# Python) against a background dev app driven only through `azj`, herdr's CLI
# on the dev session and the Python client. Dev instance e2e-api.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-api}" E2E_SUITE=B14-api
source "$(dirname "$0")/lib.sh"
ev_pid=""
cleanup() { [[ -n "$ev_pid" ]] && kill "$ev_pid" 2>/dev/null || true; e2e_cleanup; }
trap cleanup EXIT
reset_instance
make_fixtures
write_config </dev/null
launch_bg >"$out/launch.txt"
wait_ping
for _ in $(seq 100); do [[ "$(azj --json tile list | jq '.tiles | length')" -ge 1 ]] && break; sleep 0.1; done
sleep 1.5

say "B14 socket permissions"
mode_sock="$(stat -f %Lp "$sock")"; mode_dir="$(stat -f %Lp "$(dirname "$sock")")"
echo "socket $mode_sock dir $mode_dir owner $(stat -f %Su "$sock")" | tee "$out/perms.txt"
check "socket is 0600" test "$mode_sock" = 600
check "socket dir is 0700" test "$mode_dir" = 700

say "B14 events --follow (all events, and a tile.* filter)"
azj --json events --follow >"$out/events-all.ndjson" 2>&1 &
ev_pid=$!
azj --json events --follow --filter tile. >"$out/events-tile.ndjson" 2>&1 &
ev2=$!
sleep 0.5
t1="$(azj --json layout split --direction right | jq -r '.tile')"
sleep 1
azj tile focus --tile "$t1" >/dev/null
sleep 1.5
azj --json tile get --tile "$t1" >"$out/tile1.json"
pane1="$(jq -r '[.. | .pane_id? | strings][0] // empty' "$out/tile1.json")"
echo "tile $t1 pane $pane1" >>"$out/notes.txt"
hd pane report-agent "$pane1" --source e2e --agent claude --state working >/dev/null
sleep 0.4
hd pane report-agent "$pane1" --source e2e --agent claude --state blocked >/dev/null
sleep 1.5
kill "$ev_pid" "$ev2" 2>/dev/null || true; ev_pid=""
for e in tile.created focus.changed agent.state_changed layout.changed; do
  check "events stream has $e" grep -q "\"$e\"" "$out/events-all.ndjson"
done
check "events lines are JSON" python3 -c "import json,sys; [json.loads(l) for l in open('$out/events-all.ndjson') if l.strip()]"
check "filter tile. passes only tile.* events" python3 -c "
import json; ev=[json.loads(l) for l in open('$out/events-tile.ndjson') if l.strip()]
names={e.get('event') or e.get('params',{}).get('event') for e in ev}; print(names)
assert ev and all(str(n).startswith('tile.') for n in names)"
head -c 600 "$out/events-all.ndjson" >"$out/events-sample.txt"

say "B14 inside a tile: env, caller resolution, A drives B"
tA="$(azj --json tile list | jq -r '.tiles[] | select(.kind=="terminal") | .id' | head -1)"
tB="$t1"
azj terminal send-text --tile "$tA" --text "env | grep '^AZULEJO_' | sort > $out/tileA-env.txt; $azj_bin --json tile get > $out/tileA-self.json; $azj_bin terminal send-text --tile $tB --text 'echo hello-from-A-\$AZULEJO_TILE_ID'; $azj_bin terminal send-keys --tile $tB --keys Enter
" >/dev/null
sleep 3
cat "$out/tileA-env.txt" || true
check "tile env has AZULEJO_TILE_ID of tile A" grep -q "AZULEJO_TILE_ID=$tA" "$out/tileA-env.txt"
check "tile env has AZULEJO_WORKSPACE_ID" grep -q "AZULEJO_WORKSPACE_ID=ws_" "$out/tileA-env.txt"
check "azj inside tile A resolves itself as the caller" grep -q "\"$tA\"" "$out/tileA-self.json"
azj terminal read --tile "$tB" --source recent >"$out/tileB-read.txt" 2>&1 || true
check "tile B shows text sent by tile A (terminal read)" grep -q "hello-from-A-$tB" "$out/tileB-read.txt"

say "B10 changes made in herdr"
before="$(azj --json tile list | jq '.tiles | length')"
s="$(now_ms)"
hd pane split "$pane1" --direction down >"$out/herdr-split.json"
newpane="$(jq -r '.result.pane.pane_id // .result.pane_id // empty' "$out/herdr-split.json")"
got=""
for _ in $(seq 60); do
  [[ "$(azj --json tile list | jq '.tiles | length')" -gt "$before" ]] && { got="$(( $(now_ms) - s ))"; break; }
  sleep 0.025
done
echo "herdr pane split → tile: ${got:-never} ms" | tee -a "$out/notes.txt"
check "herdr split shows up as a tile within 1 s" test -n "$got" -a "${got:-9999}" -lt 1000
s="$(now_ms)"; got=""
hd pane close "$newpane" >/dev/null 2>&1 || true
for _ in $(seq 60); do
  [[ "$(azj --json tile list | jq '.tiles | length')" -eq "$before" ]] && { got="$(( $(now_ms) - s ))"; break; }
  sleep 0.025
done
echo "herdr pane close → tile gone: ${got:-never} ms" | tee -a "$out/notes.txt"
check "herdr close removes the tile within 1 s" test -n "$got" -a "${got:-9999}" -lt 1000

say "B10 an azj script builds a two-workspace layout"
w1="$(azj --json workspace create --name e2e-one | jq -r '.workspace')"
azj tab create --workspace "$w1" >/dev/null
azj layout split-auto >/dev/null; azj layout split-auto >/dev/null
azj layout split --direction right --content browser --url http://127.0.0.1:9/ >/dev/null
w2="$(azj --json workspace create --name e2e-two | jq -r '.workspace')"
azj tab create --workspace "$w2" >/dev/null
azj layout split --direction right --content preview --path "$fx/plan.md" >/dev/null
sleep 1
azj --json session get >"$out/session-two-ws.json"
check "workspace e2e-one has 4 tiles, e2e-two has 2" python3 - "$out/session-two-ws.json" <<'PY'
import json, sys
s = json.load(open(sys.argv[1])); s = s.get("session", s)
ws = {w["name"]: w for win in s["windows"] for w in win["workspaces"]}
count = lambda w: sum(len(t["tiles"]) for t in w["tabs"])
print(count(ws["e2e-one"]), count(ws["e2e-two"]))
assert count(ws["e2e-one"]) == 4 and count(ws["e2e-two"]) == 2
PY

say "B7 files: open a terminal at a path"
azj workspace focus --workspace "$w1" >/dev/null
ft="$(azj --json layout split --direction right --content files --root "$fx/azulejo" | jq -r '.tile')"
sleep 1
mkdir -p "$fx/azulejo/src"
nt="$(azj --json files open --tile "$ft" --path "$fx/azulejo/src" --target terminal | jq -r '.tile')"
sleep 2.5
azj --json tile get --tile "$nt" >"$out/files-terminal-dir.json"
check "files.open --target terminal on a folder (absolute path) opens a terminal there" python3 -c "
import json; t=json.load(open('$out/files-terminal-dir.json')); print(t.get('kind'), t.get('cwd'), t.get('pane'))
assert t['kind']=='terminal' and str(t.get('pane',{}).get('cwd','')).endswith('azulejo/src')"
nt2="$(azj --json files open --tile "$ft" --path README.md --target terminal | jq -r '.tile')"
sleep 2.5
azj --json tile get --tile "$nt2" >"$out/files-terminal-file.json"
check "azj files open --path README.md (relative to the tile root, as documented) opens a terminal in its folder" python3 -c "
import json; t=json.load(open('$out/files-terminal-file.json')); print(t.get('kind'), t.get('cwd'), t.get('pane'))
assert t['kind']=='terminal' and str(t.get('pane',{}).get('cwd','')).endswith('/azulejo')"

say "B14 layout apply / export"
mkdir -p "$fx/web"
cp "$root/docs/examples/azulejo.toml" "$fx/azulejo.toml"
azj --json layout apply --path "$fx/azulejo.toml" >"$out/apply-example.json"
check "layout apply docs/examples/azulejo.toml" jq -e '.workspace' "$out/apply-example.json"
sleep 1.5
azj --json layout export >"$out/export-example.json"
jq -r '.layout_file // .toml // .' "$out/export-example.json" >"$out/export-example.toml"
cat "$out/export-example.toml"
check "export keeps layout = tile and 4 tiles" python3 - "$out/export-example.toml" <<'PY'
import sys, tomllib
d = tomllib.loads(open(sys.argv[1]).read())
tabs = d.get("tab") or [d]
t = tabs[0]
print(t.get("layout"), len(t.get("tile", [])))
assert t.get("layout") == "tile" and len(t.get("tile", [])) == 4
PY
cat >"$fx/three.toml" <<'TOML'
name = "three"
layout = "three_column"
[[tile]]
title = "left"
[[tile]]
title = "middle"
focus = true
[[tile]]
title = "right"
[[tile]]
path = "plan.md"
TOML
azj --json layout apply --path "$fx/three.toml" >"$out/apply-three.json"
sleep 1.5
azj --json layout export --path "$fx/three-export.toml" >"$out/export-three.json"
cat "$fx/three-export.toml"
check "three_column round-trips through export" python3 - "$fx/three-export.toml" <<'PY'
import sys, tomllib
d = tomllib.loads(open(sys.argv[1]).read())
t = (d.get("tab") or [d])[0]
print(t.get("layout"), [x.get("title") or x.get("path") for x in t.get("tile", [])])
assert t.get("layout") == "three_column" and len(t.get("tile", [])) == 4
PY
azj --json layout apply --path "$fx/three-export.toml" >"$out/apply-three-again.json"
check "the exported file applies again" jq -e '.workspace' "$out/apply-three-again.json"

say "B14 Python clients (sync and asyncio) against the dev socket"
(cd "$root/clients/python" && "${clean_env[@]}" AZULEJO_SOCKET="$sock" uv run --locked -q python - <<'PY') >"$out/python.txt" 2>&1 || true
import asyncio, time
from azulejo import Client, AsyncClient
with Client() as c:
    t = c.layout_split("right")["tile"]
    time.sleep(1.5)
    c.terminal_send_text("echo py-sync-ok\n", tile=t)
    time.sleep(1.0)
    text = c.terminal_read(tile=t)["text"]
    assert "py-sync-ok" in text, text[-200:]
    c.tile_close(tile=t)
    print("sync ok", t)
async def main():
    async with await AsyncClient.connect() as c:
        t = (await c.layout_split("right"))["tile"]
        await asyncio.sleep(1.5)
        await c.terminal_send_text("echo py-async-ok\n", tile=t)
        await asyncio.sleep(1.0)
        text = (await c.terminal_read(tile=t))["text"]
        assert "py-async-ok" in text, text[-200:]
        await c.tile_close(tile=t)
        print("async ok", t)
asyncio.run(main())
PY
[[ -s "$out/python.txt" ]] || true
cat "$out/python.txt"
check "python sync client create/read/close" grep -q "^sync ok" "$out/python.txt"
check "python async client create/read/close" grep -q "^async ok" "$out/python.txt"

say "API leniency: unknown params"
azj call layout.split '{"direction":"right","kind":"browser","url":"http://x/"}' >"$out/unknown-params.txt" 2>&1 || true
cat "$out/unknown-params.txt"
stop_app
grep -c PASS "$checks" | sed 's/^/passed: /'; ! grep -q FAIL "$checks"
