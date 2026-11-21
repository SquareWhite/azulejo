#!/usr/bin/env bash
# Key and focus suite, in-process on its own dev instance
# (AZJ_DEV_NAME=focus): which action each chord resolves to and where first
# responder ends up, across terminals, the sidebar's Files view, the ⌘K
# menu, layouts, tabs, workspaces, herdr changes, take-over and input
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
# Resolved (/var is a link to /private/var), as the Files view shows it.
fixtures="$(cd "$fixtures" && pwd -P)"

cleanup() {
  kill "${watchdog:-}" 2>/dev/null || true
  rm -rf "$fixtures"
  [[ "${1:-}" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null
}
trap 'cleanup "${KEEP:-}"' EXIT
[[ "${1:-}" == "--keep" ]] && KEEP=--keep

# A small git repo for the sidebar's Files view.
mkdir -p "$fixtures/tree/sub"
for f in alpha.txt beta.txt gamma.txt; do echo "$f" >"$fixtures/tree/$f"; done
echo inner >"$fixtures/tree/sub/inner.txt"
git -C "$fixtures/tree" init -q

"$root/apps/macos/dev.sh" --reset >/dev/null
mkdir -p "$out"
sed -e "s|@DIR@|$fixtures|g" -e "s|@HERE@|$here|g" \
  "$here/focus.azs.in" >"$fixtures/focus.azs"

# A hung run must not wait forever: stop the app after 5 minutes.
( sleep 300; "$root/apps/macos/dev.sh" --stop ) &
watchdog=$!

"$root/apps/macos/dev.sh" --automation "$fixtures/focus.azs" --out "$out"
result="$(cat "$out/result")"
scenarios="$(grep -c '^ *[0-9]* # === S' "$out/automation.log" || true)"
[[ "$result" == ok ]] || { echo "focus: $scenarios scenarios run; $result"; exit 1; }
echo "focus: ok, $scenarios scenarios ($out)"
