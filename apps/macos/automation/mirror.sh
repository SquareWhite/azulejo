#!/usr/bin/env bash
# ADR 0012 in the app: splits made in the dev app show up as pane splits in
# its herdr tab, and herdr's changes come back. Runs on its own dev instance
# (AZJ_DEV_NAME=mir, herdr session azj-dev-mir), --bg, no OS input.
#
#   apps/macos/automation/mirror.sh [--keep]     (build first: just mac)
#
# Checks, reading herdr's own tree (layout.export) for the dev session's tab:
#  1. a split right in the app → the same herdr tab gets a pane, split right;
#  2. a split down → right(first, down(second, third));
#  3. a ratio set in herdr (as the TUI's resize does) → the app's tree follows;
#  4. Monocle in the app → herdr zooms the focused tile's pane.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-mir}"
keep="${1:-}"
dev="$root/.dev/$AZJ_DEV_NAME"
session="azj-dev-$AZJ_DEV_NAME"
sock="$dev/run/azulejo.sock"
dev_sh="$root/apps/macos/dev.sh"

case "$session" in azj-dev-*) ;; *) echo "mirror.sh: not a dev session: $session" >&2; exit 2 ;; esac
unset HERDR_SOCKET_PATH HERDR_PANE_ID HERDR_TAB_ID HERDR_WORKSPACE_ID HERDR_ENV
clean_env=(env -u AZULEJO_TILE_ID -u AZULEJO_WORKSPACE_ID)
azj() { "${clean_env[@]}" AZULEJO_SOCKET="$sock" "$root/target/debug/azj" --no-caller --json "$@"; }
fail() { echo "FAIL: $*" >&2; exit 1; }

cleanup() {
  [[ "$keep" == "--keep" ]] && return
  "$dev_sh" --reset >/dev/null 2>&1 || true
}
trap cleanup EXIT

cargo build -q -p azulejo-cli
"$dev_sh" --reset >/dev/null
mkdir -p "$dev/config/azulejo"
cat >"$dev/config/azulejo/config.toml" <<EOF
# Dev profile config (written by automation/mirror.sh)
[herdr]
session = "$session"
mirror_layout = true
[layout]
default = "manual"
EOF

"$dev_sh" --bg >/dev/null
for _ in $(seq 100); do azj core ping >/dev/null 2>&1 && break; sleep 0.05; done
azj core ping >/dev/null 2>&1 || fail "the dev app didn't answer on $sock"

# Reads the app's session and herdr's tree for the tab of its first
# terminal, and checks them with a Python expression over `s` (the tab's
# terminals and tree) and `h` (herdr's layout). Retries for 10 s.
check() {
  local what="$1" expr="$2" session_json
  for _ in $(seq 100); do
    session_json="$(azj session get 2>/dev/null || true)"
    if SESSION_JSON="$session_json" HERDR_SESSION_NAME="$session" EXPR="$expr" python3 - <<'PY'
import json, os, re, socket, subprocess, sys
session = json.loads(os.environ["SESSION_JSON"] or "{}").get("session")
if not session:
    sys.exit(1)
tab = session["windows"][0]["workspaces"][0]["tabs"][0]
terms = [t for t in tab["tiles"] if t.get("kind") == "terminal"]
if not terms or not all(t.get("herdr") for t in terms):
    sys.exit(1)
name = os.environ["HERDR_SESSION_NAME"]
status = subprocess.run(["herdr", "--session", name, "status"], capture_output=True, text=True).stdout
path = re.search(r"socket:\s*(\S+)", status.split("server:", 1)[-1]).group(1)
assert f"/sessions/{name}/" in path, path
def rpc(method, params):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(path)
    s.sendall((json.dumps({"id": "check", "method": method, "params": params}) + "\n").encode())
    buf = b""
    while not buf.endswith(b"\n"):
        chunk = s.recv(65536)
        if not chunk:
            break
        buf += chunk
    return json.loads(buf)["result"]
h = rpc("layout.export", {"tab_id": terms[0]["herdr"]["tab_id"]})["layout"]
pane = {t["id"]: t["herdr"]["pane_id"] for t in terms}
def tree(n):
    return n["pane_id"] if n["type"] == "pane" else [n["direction"], round(n["ratio"], 2), tree(n["first"]), tree(n["second"])]
def ours(n):
    if n["type"] == "leaf":
        return pane.get(n["id"])
    return ["right" if n["axis"] == "horizontal" else "down", round(n["ratio"], 2), ours(n["a"]), ours(n["b"])]
s = {"tab": tab, "terms": terms, "tree": ours(tab["root"]), "tabs": {t["herdr"]["tab_id"] for t in terms}}
h["tree"] = tree(h["root"])
ok = eval(os.environ["EXPR"])
if os.environ.get("MIRROR_DEBUG"):
    print(s["tree"], h["tree"], h["zoomed"], ok, file=sys.stderr)
sys.exit(0 if ok else 1)
PY
    then
      echo "ok: $what"
      return 0
    fi
    sleep 0.1
  done
  fail "$what (MIRROR_DEBUG=1 prints each try)"
}

check "one terminal, one herdr tab" 'len(s["terms"]) == 1 and h["tree"] == s["tree"]'

azj layout split --direction right >/dev/null
check "split right → a pane split right in the same herdr tab" \
  'len(s["terms"]) == 2 and len(s["tabs"]) == 1 and h["tree"] == s["tree"] and h["tree"][0] == "right"'

azj layout split --direction down >/dev/null
check "split down → right(first, down(second, third))" \
  'len(s["terms"]) == 3 and len(s["tabs"]) == 1 and h["tree"] == s["tree"] and h["tree"][3][0] == "down"'

# A resize in herdr, as the TUI does it.
tab_id="$(azj session get | python3 -c 'import json,sys; t=json.load(sys.stdin)["session"]["windows"][0]["workspaces"][0]["tabs"][0]; print([x for x in t["tiles"] if x.get("herdr")][0]["herdr"]["tab_id"])')"
first="$(azj session get | python3 -c 'import json,sys; t=json.load(sys.stdin)["session"]["windows"][0]["workspaces"][0]["tabs"][0]; r=t["root"]
while r["type"]!="leaf": r=r["a"]
print([x for x in t["tiles"] if x["id"]==r["id"]][0]["herdr"]["pane_id"])')"
herdr --session "$session" pane resize --pane "$first" --direction right --amount 0.2 >/dev/null
check "a resize in herdr → the app's ratio follows" \
  'abs(s["tab"]["root"]["ratio"] - 0.7) < 0.01 and h["tree"] == s["tree"]'

azj layout set --kind monocle >/dev/null
check "Monocle → herdr zooms the focused tile's pane" \
  'h["zoomed"] and h["focused_pane_id"] == [t for t in s["terms"] if t["id"] == s["tab"]["focused"]][0]["herdr"]["pane_id"]'

echo "mirror.sh: all checks passed ($session, tab $tab_id)"
