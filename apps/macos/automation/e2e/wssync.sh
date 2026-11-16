#!/usr/bin/env bash
# Workspace close and rename reach herdr, on dev instance
# e2e-sync: after every step, Azulejo's workspaces (`azj workspace list`) and
# herdr's (`herdr workspace list` on the dev session) must match, IDs and
# names. Covers close and rename made in Azulejo, rename and close made in
# herdr, a relaunch, a name that drifted while Azulejo was down (Azulejo's
# wins), and a workspace closed here before closes reached herdr (left
# running in herdr and reported in the log, never adopted or closed). Then
# the shell's confirmation (confirm.azs): closing a workspace that runs a
# command asks first (cancel keeps it, Close ends it in herdr), an idle one
# closes without asking.
#   AZJ_DEV_NAME=e2e-sync apps/macos/automation/e2e/wssync.sh
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-sync}" E2E_SUITE=wssync
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null
herdr_fresh
cargo build -q -p azulejo-cli

# One workspace the user already runs in herdr; adoption brings it in.
hd workspace create --label legacy >"$out/ws-legacy.json"
legacy="$(jq -r '.result.workspace.workspace_id' "$out/ws-legacy.json")"

# "herdr-id name" lines, sorted, for each side.
azj_ws() { azj --json workspace list | jq -r '.workspaces[] | "\(.herdr_workspace // "-") \(.name)"' | sort; }
herdr_ws() { hd workspace list | jq -r '.result.workspaces[] | "\(.workspace_id) \(.label // "")"' | sort; }
ws_id() { azj --json workspace list | jq -r --arg n "$1" '.workspaces[] | select(.name == $n) | .id'; }
herdr_of() { azj --json workspace list | jq -r --arg n "$1" '.workspaces[] | select(.name == $n) | .herdr_workspace'; }
in_sync() { [[ "$(azj_ws)" == "$(herdr_ws)" ]]; }
# Azulejo's list is herdr's without herdr workspace $1.
in_sync_except() { [[ "$(azj_ws)" == "$(herdr_ws | grep -v "^$1 ")" ]]; }
all_bound() { [[ "$(azj --json workspace list | jq '[.workspaces[] | select(.herdr_workspace)] | length')" == "$1" ]]; }
herdr_label_is() { hd workspace get "$1" 2>/dev/null | jq -e --arg l "$2" '.result.workspace.label == $l' >/dev/null; }
herdr_gone() { ! hd workspace get "$1" >/dev/null 2>&1; }
no_panes_in() { [[ "$(hd pane list | jq --arg w "$1" '[.result.panes[] | select(.workspace_id == $w)] | length')" == 0 ]]; }
azj_named() { [[ -n "$(ws_id "$1")" ]]; }
azj_shows() { azj --json workspace list | jq -e --arg h "$1" 'any(.workspaces[]; .herdr_workspace == $h)' >/dev/null; }
azj_hides() { ! azj_shows "$1"; }
# until_ok SECONDS COMMAND…: retry a check until it passes.
until_ok() {
  local deadline=$(( $(date +%s) + $1 )); shift
  until "$@" >/dev/null 2>&1; do
    [[ $(date +%s) -ge $deadline ]] && return 1
    sleep 0.1
  done
}
snapshot() { { echo "# azulejo"; azj_ws; echo "# herdr"; herdr_ws; } >"$out/$1.txt"; }

say "launch: legacy is adopted"
launch_bg >"$out/launch.txt"
wait_ping
check "legacy adopted, lists match" until_ok 10 in_sync
snapshot 01-adopted

say "create three workspaces"
for n in alpha bravo charlie; do
  id="$(azj --json workspace create --name "$n" | jq -r .workspace)"
  azj tab create --workspace "$id" >/dev/null
done
check "every new workspace gets a herdr workspace" until_ok 10 all_bound 4
check "lists match after create (names included)" until_ok 5 in_sync
snapshot 02-created

say "rename bravo and close charlie in Azulejo"
bravo_h="$(herdr_of bravo)"
charlie_h="$(herdr_of charlie)"
azj workspace rename --workspace "$(ws_id bravo)" --name bravo-2 >/dev/null
azj workspace close --workspace "$(ws_id charlie)" >/dev/null
check "herdr takes the rename" until_ok 5 herdr_label_is "$bravo_h" bravo-2
check "herdr closes charlie's workspace" until_ok 5 herdr_gone "$charlie_h"
check "no pane left in charlie's workspace" no_panes_in "$charlie_h"
check "lists match after rename and close" until_ok 5 in_sync
snapshot 03-azulejo-side

say "rename alpha and close bravo-2 in herdr"
alpha_h="$(herdr_of alpha)"
hd workspace rename "$alpha_h" alpha-tui >/dev/null
hd workspace close "$bravo_h" >/dev/null
check "Azulejo follows herdr's rename" until_ok 5 azj_named alpha-tui
check "Azulejo follows herdr's close" until_ok 5 azj_hides "$bravo_h"
check "lists match after herdr's changes" until_ok 5 in_sync
snapshot 04-herdr-side

say "relaunch"
stop_app
launch_bg >"$out/relaunch.txt"
wait_ping
sleep 1.5
check "lists match after a relaunch" in_sync
snapshot 05-relaunched

say "state from before the fix: legacy closed here only, alpha renamed offline"
stop_app
python3 - "$state" "$legacy" <<'PY'
import json, sys
path, legacy = sys.argv[1], sys.argv[2]
doc = json.load(open(path))
s = doc["session"]
def refs(node, out):
    if isinstance(node, dict):
        h = node.get("herdr")
        if isinstance(h, dict) and "pane_id" in h:
            out.append(h)
        for v in node.values():
            refs(v, out)
    elif isinstance(node, list):
        for v in node:
            refs(v, out)
for w in s["windows"]:
    for ws in list(w["workspaces"]):
        if ws.get("herdr_workspace") == legacy:
            found = []
            refs(ws, found)
            s.setdefault("detached_herdr", []).extend(found)
            w["workspaces"].remove(ws)
        elif ws.get("name") == "alpha-tui":
            ws["name"] = "alpha-offline"
    if not any(x["id"] == w["active_workspace"] for x in w["workspaces"]):
        w["active_workspace"] = w["workspaces"][0]["id"]
json.dump(doc, open(path, "w"))
PY
: >"$applog"
launch_bg >"$out/relaunch-legacy.txt"
wait_ping
check "Azulejo's name wins: herdr takes alpha-offline" until_ok 5 herdr_label_is "$alpha_h" alpha-offline
check "the orphan still runs in herdr" hd workspace get "$legacy"
sleep 1
check "the orphan isn't adopted back" azj_hides "$legacy"
check "the orphan is reported in the log, by ID" until_ok 5 grep -q "not shown in Azulejo, left running: $legacy" "$applog"
grep "not shown in Azulejo" "$applog" >"$out/orphan-log.txt" || true
check "lists match apart from the orphan" until_ok 5 in_sync_except "$legacy"
snapshot 06-legacy

say "closing a workspace with something running asks first"
for n in busy idle; do
  id="$(azj --json workspace create --name "$n" | jq -r .workspace)"
  azj tab create --workspace "$id" >/dev/null
done
check "busy and idle get herdr workspaces" until_ok 10 all_bound 3
busy_h="$(herdr_of busy)"
idle_h="$(herdr_of idle)"
busy_tile="$(azj --json tile list --workspace "$(ws_id busy)" | jq -r '.tiles[0].tile.id // .tiles[0].id')"
azj terminal send-text --tile "$busy_tile" --text "sleep 600" >/dev/null
azj terminal send-keys --tile "$busy_tile" --keys enter >/dev/null
busy_runs() { azj --json tile list --workspace "$(ws_id busy)" | jq -e '[.. | objects | select(.command? == "sleep 600")] | length > 0' >/dev/null; }
check "the core sees sleep running in busy" until_ok 10 busy_runs
stop_app
# Rows: 1 alpha-offline, 2 busy, 3 idle. Through the sidebar menu, so the
# test doesn't depend on close_workspace's chord.
cat >"$out/confirm.azs" <<'EOF'
log confirm before closing a workspace that runs something
until workspaces 3 10000
wait 2500
expect prompt.asks 0
menu answer cancel
menu workspace 2 invoke Close Workspace
wait 300
expect prompt.asks 1
expect workspaces 3
menu workspace 3 invoke Close Workspace
until workspaces 2 3000
log idle closed without a prompt
expect prompt.asks 1
menu answer Close
menu workspace 2 invoke Close Workspace
until workspaces 1 3000
expect prompt.asks 2
EOF
run_azs "$out/confirm.azs" confirm
grep -E "prompt|confirm|idle|FAIL" "$out/confirm/automation.log" >"$out/confirm-log.txt" 2>/dev/null || true
check "cancel kept busy running, then Close ended it in herdr" until_ok 5 herdr_gone "$busy_h"
check "idle closed in herdr without a prompt" herdr_gone "$idle_h"
check "no pane left in busy's workspace" no_panes_in "$busy_h"
grep -c PASS "$checks" | sed 's/^/passed: /'
! grep -q FAIL "$checks"
