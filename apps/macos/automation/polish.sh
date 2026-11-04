#!/usr/bin/env bash
# Captures for the visual polish fixes from the Nov 3 e2e pass (polish.azs.in):
# long cwd subtitles, the agent badge, the config banner, a new terminal's
# placeholder, a failed browser load, every tile kind at the minimum window,
# and twelve tabs. In-process, on its own dev instance (AZJ_DEV_NAME=polish-g,
# herdr session azj-dev-polish-g), reset before and after. No OS input; the
# window stays in the background.
#
#   apps/macos/automation/polish.sh [OUT]   (default .dev/polish-out)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-polish-g}"
out="${1:-$root/.dev/polish-out}"
dev_sh="$root/apps/macos/dev.sh"

"$dev_sh" --reset >/dev/null 2>&1 || true
fx="$(mktemp -d "${TMPDIR:-/tmp}/azj-polish.XXXXXX")"
site_pid=""
cleanup() {
  [[ -n "$site_pid" ]] && kill "$site_pid" 2>/dev/null
  "$dev_sh" --reset >/dev/null 2>&1 || true
  rm -rf "$fx"
}
trap cleanup EXIT

# A deep temp folder (no ~ for these), and a long folder under home: this
# checkout's own source folder, only read.
deep="$fx/clients/example-org/projects/2026/azulejo-design-review"
mkdir -p "$deep"
printf '# Plan\n\n- [ ] step one\n' >"$deep/plan.md"
home_dir="$root/apps/macos/Sources/Azulejo"
# Prints a marker against the terminal's right edge, where the badge used to sit.
printf '#!/bin/sh\nclear\nprintf "%%*s\\n" "$(tput cols)" RIGHT-EDGE-TEXT\n' >"$fx/right.sh"
chmod +x "$fx/right.sh"

(cd "$here/browser-site" && exec python3 -m http.server 47851 --bind 127.0.0.1 >/dev/null 2>&1) &
site_pid=$!

rm -rf "$out"
mkdir -p "$out"
sed -e "s|@FX@|$fx|g" -e "s|@DEEP@|$deep|g" -e "s|@HOME@|$home_dir|g" \
  -e "s|@SITE@|http://127.0.0.1:47851/a.html|g" "$here/polish.azs.in" >"$fx/polish.azs"
"$dev_sh" --automation "$fx/polish.azs" --out "$out"
result="$(cat "$out/result" 2>/dev/null || echo none)"
grep " remember " "$out/automation.log" || true
echo "polish: $result (captures in $out)"
[[ "$result" == ok ]]
