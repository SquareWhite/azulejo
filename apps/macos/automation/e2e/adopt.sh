#!/usr/bin/env bash
# B1: first launch adopts what already runs in the (dev) herdr session.
# Two herdr workspaces and three panes exist before the app starts; the app
# must show both workspaces with their tabs, the adopted tabs must be manual,
# and there must be no extra placeholder workspace.
#   AZJ_DEV_NAME=e2e-adopt apps/macos/automation/e2e/adopt.sh
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-adopt}" E2E_SUITE=B1-adopt
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
make_fixtures
write_config </dev/null
herdr_fresh
hd workspace create --label alpha --cwd "$fx/azulejo" >"$out/ws-alpha.json"
hd workspace create --label beta --cwd "$fx/sample-service" >"$out/ws-beta.json"
alpha_pane="$(jq -r '.result.root_pane.pane_id' "$out/ws-alpha.json")"
hd pane split "$alpha_pane" --direction right >/dev/null
hd pane list >"$out/herdr-panes-before.json"
check "herdr has 3 panes before launch" test "$(jq '.result.panes | length' "$out/herdr-panes-before.json")" = 3

cat >"$out/adopt.azs" <<'EOF'
log B1 adoption
until workspaces 2 5000
expect workspace 1
expect tabs 1
expect tiles 2
expect kind manual
wait 1200
shot 01-alpha
key cmd+2
expect workspace 2
expect tiles 1
expect kind manual
wait 800
shot 02-beta
key cmd+1
expect workspace 1
EOF
run_azs "$out/adopt.azs" adopt
python3 - "$state" >"$out/state-summary.txt" <<'PY'
import json, sys
s = json.load(open(sys.argv[1]))
s = s.get("session", s)
for w in s["windows"]:
    for ws in w["workspaces"]:
        print("workspace", ws.get("name"), "tabs", len(ws["tabs"]),
              [(t.get("layout", {}).get("kind") if isinstance(t.get("layout"), dict) else t.get("kind"), len(t.get("tiles", {}))) for t in ws["tabs"]])
PY
cat "$out/state-summary.txt"
check "workspaces are alpha and beta, no placeholder" test "$(awk '{print $2}' "$out/state-summary.txt" | sort | tr '\n' ' ')" = "alpha beta "
hd pane list >"$out/herdr-panes-after.json"
check "herdr still has exactly 3 panes (adoption only reads)" test "$(jq '.result.panes | length' "$out/herdr-panes-after.json")" = 3
grep -c PASS "$checks" | sed 's/^/passed: /'; ! grep -q FAIL "$checks"
