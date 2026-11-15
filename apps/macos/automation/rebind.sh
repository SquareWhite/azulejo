#!/usr/bin/env bash
# Live rebinding of every keymap context (PLAN 2.3; docs/keybindings.md), in
# process on its own dev instance (AZJ_DEV_NAME=rebind by default). One key in
# each of palette, find_bar, resize, browser, preview and files moves to a new
# chord through the instance's config file, and the terminal gets a send_text
# line and loses its cmd+l one: the new chord works without a
# restart, the old one doesn't, "" and "none" unbind, and putting the old
# file back restores the defaults. Also checks that `azj config keys` shows the
# overrides and that "Open config file" writes `azj config defaults` when the
# file is missing. No OS input; the app runs in the background.
#
#   apps/macos/automation/rebind.sh [--keep]   (--keep: leave the dev instance)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-rebind}"
dev="$root/.dev/$AZJ_DEV_NAME"
out="$dev/out/rebind"
sock="$dev/run/azulejo.sock"
fixtures="$(mktemp -d "${TMPDIR:-/tmp}/azj-rebind.XXXXXX")"
port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')"

cleanup() {
  kill "${server:-}" "${watchdog:-}" 2>/dev/null || true
  rm -rf "$fixtures"
  [[ "${1:-}" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null
}
trap 'cleanup "${KEEP:-}"' EXIT
[[ "${1:-}" == "--keep" ]] && KEEP=--keep

cargo build -q -p azulejo-cli --manifest-path "$root/Cargo.toml"
azj=(env -u HERDR_SOCKET_PATH -u HERDR_PANE_ID -u HERDR_TAB_ID -u HERDR_WORKSPACE_ID -u HERDR_ENV
  -u AZULEJO_TILE_ID -u AZULEJO_WORKSPACE_ID "AZULEJO_SOCKET=$sock" "$root/target/debug/azj" --no-caller)

mkdir -p "$fixtures/site" "$fixtures/tree/sub"
printf '<!doctype html><title>rebind page</title><p>keys</p>\n' >"$fixtures/site/index.html"
printf '# Plan\n\nRebind every context.\n' >"$fixtures/plan.md"
for f in alpha.txt beta.txt gamma.txt; do echo "$f" >"$fixtures/tree/$f"; done
echo inner >"$fixtures/tree/sub/inner.txt"
# One key per context, plus an unbind with "none" and one with "".
cat >"$fixtures/rebind.toml" <<'TOML'

[keys]
quick_open = "cmd+j"

[keys.macos.palette]
palette_next = "ctrl+down"
palette_previous = "none"

[keys.macos.find_bar]
find_close = "ctrl+g"

[keys.macos.resize]
resize_tile_right = "d"
resize_mode_exit = "q"

[keys.macos.browser]
browser_focus_address = "cmd+e"

[keys.macos.preview]
preview_toggle_source = "cmd+shift+u"

[keys.macos.files]
files_down = "n"
files_toggle_hidden = ""

[keys.macos.terminal]
"shift+enter" = { send_text = "\n" }
"cmd+l" = { send_text = "" }
TOML

python3 -m http.server "$port" --bind 127.0.0.1 --directory "$fixtures/site" >/dev/null 2>&1 &
server=$!
for _ in $(seq 50); do curl -fs "http://127.0.0.1:$port/" >/dev/null && break; sleep 0.1; done

"$root/apps/macos/dev.sh" --reset >/dev/null
mkdir -p "$out"
sed -e "s|@URL@|http://127.0.0.1:$port/|g" -e "s|@DIR@|$fixtures|g" -e "s|@AZJ@|${azj[*]}|g" \
  "$here/rebind.azs.in" >"$fixtures/rebind.azs"

# A hung run must not wait forever: stop the app after 4 minutes.
( sleep 240; "$root/apps/macos/dev.sh" --stop ) &
watchdog=$!

"$root/apps/macos/dev.sh" --automation "$fixtures/rebind.azs" --out "$out"
result="$(cat "$out/result")"
[[ "$result" == ok ]] || { echo "rebind: $result"; exit 1; }

# `azj config keys` saw the overrides, with their defaults and source.
python3 - "$out/keys-rebound.json" <<'PY'
import json, sys
keys = json.load(open(sys.argv[1]))["keys"]
def row(action, context):
    return next(k for k in keys if k["action"] == action and k["context"] == context)
checks = {
    ("palette_next", "palette"): (["down", "ctrl+n", "ctrl+j"], ["ctrl+down"]),
    ("palette_previous", "palette"): (["up", "ctrl+p", "ctrl+k"], []),
    ("find_close", "find_bar"): (["escape"], ["ctrl+g"]),
    ("resize_mode_exit", "resize"): (["escape", "enter"], ["q"]),
    ("browser_focus_address", "browser"): (["cmd+l"], ["cmd+e"]),
    ("preview_toggle_source", "preview"): (["alt+cmd+u"], ["cmd+shift+u"]),
    ("files_down", "files"): (["j", "down"], ["n"]),
    ("files_toggle_hidden", "files"): (["period"], []),
    ("quick_open", "global"): (["cmd+k"], ["cmd+j"]),
}
for (action, context), (default, current) in checks.items():
    r = row(action, context)
    assert (r["default"], r["current"], r["source"]) == (default, current, "user"), r
assert row("browser_back", "browser")["source"] == "default"
# send_text: one row per chord, with the text.
texts = {(k["default"] or k["current"])[0]: k for k in keys if k["action"] == "send_text"}
newline, clear = texts["shift+enter"], texts["cmd+l"]
assert (newline["default"], newline["current"], newline.get("text"), newline["source"]) \
    == ([], ["shift+enter"], "\n", "user"), newline
assert (clear["default"], clear["current"], clear.get("text"), clear["default_text"], clear["source"]) \
    == (["cmd+l"], [], None, "\x0c", "user"), clear
print(f"rebind: azj config keys lists {len(keys)} rows with the overrides")
PY

# "Open config file" wrote exactly the template `azj config defaults` prints.
cmp -s "$out/created-config.toml" "$out/defaults.toml" \
  || { echo "rebind: the created config isn't the defaults template"; diff "$out/defaults.toml" "$out/created-config.toml" | head; exit 1; }
grep -q '^\[keys.macos.find_bar\]$' "$out/created-config.toml"
grep -q '^# files_down = \["j", "down"\]' "$out/created-config.toml"
scenarios="$(grep -c '^ *[0-9]* # === R' "$out/automation.log" || true)"
echo "rebind: ok, $scenarios scenarios ($out)"
