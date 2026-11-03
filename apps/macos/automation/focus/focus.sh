#!/usr/bin/env bash
# Key and focus suite (PLAN 2.11), in-process on its own dev instance
# (AZJ_DEV_NAME=focus): which action each chord resolves to and where first
# responder ends up, across terminal, browser, preview and files tiles, the
# ⌘K menu, layouts, tabs, workspaces, herdr changes, take-over and input
# methods. No OS input; terminal copy and paste use a private pasteboard.
#
#   apps/macos/automation/focus/focus.sh [--keep]   (--keep: leave the dev instance)
#
# XCUITest would need full Xcode; this runs on the same in-process automation
# as the other walkthroughs. Each scenario is a `log === Sxx` block of plain
# steps and `expect`s, so it can move to XCUITest one block at a time.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-focus}"
out="$root/.dev/$AZJ_DEV_NAME/out/focus"
fixtures="$(mktemp -d "${TMPDIR:-/tmp}/azj-focus.XXXXXX")"
port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')"

cleanup() {
  kill "${server:-}" "${watchdog:-}" 2>/dev/null || true
  rm -rf "$fixtures"
  [[ "${1:-}" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null
}
trap 'cleanup "${KEEP:-}"' EXIT
[[ "${1:-}" == "--keep" ]] && KEEP=--keep

# A page with a text input, a markdown file to preview, a small tree to browse.
mkdir -p "$fixtures/site" "$fixtures/tree/sub"
cat >"$fixtures/site/index.html" <<'HTML'
<!doctype html><title>focus page</title>
<input id="q" autocomplete="off"><p id="out"></p>
<script>document.getElementById('q').addEventListener('keydown', e => {
  document.getElementById('out').textContent += e.key + ',';
});</script>
HTML
printf '# Plan\n\nKeys and focus.\n' >"$fixtures/plan.md"
for f in alpha.txt beta.txt gamma.txt; do echo "$f" >"$fixtures/tree/$f"; done
echo inner >"$fixtures/tree/sub/inner.txt"

python3 -m http.server "$port" --bind 127.0.0.1 --directory "$fixtures/site" >/dev/null 2>&1 &
server=$!
for _ in $(seq 50); do curl -fs "http://127.0.0.1:$port/" >/dev/null && break; sleep 0.1; done

"$root/apps/macos/dev.sh" --reset >/dev/null
mkdir -p "$out"
sed -e "s|@URL@|http://127.0.0.1:$port/|g" -e "s|@DIR@|$fixtures|g" -e "s|@HERE@|$here|g" \
  "$here/focus.azs.in" >"$fixtures/focus.azs"

# A hung run must not wait forever: stop the app after 5 minutes.
( sleep 300; "$root/apps/macos/dev.sh" --stop ) &
watchdog=$!

"$root/apps/macos/dev.sh" --automation "$fixtures/focus.azs" --out "$out"
result="$(cat "$out/result")"
scenarios="$(grep -c '^ *[0-9]* # === S' "$out/automation.log" || true)"
[[ "$result" == ok ]] || { echo "focus: $scenarios scenarios run; $result"; exit 1; }
echo "focus: ok, $scenarios scenarios ($out)"
