#!/usr/bin/env bash
# Terminal mouse, with in-app mouse events only: in a shell, plain drags and
# double/triple-clicks select and ⌘C copies (to a private pasteboard); with a
# mouse-aware program in the foreground (mouse-app.py), plain clicks reach it
# and shift-drags select; ⌘-hover highlights links and ⌘-click opens them
# (recorded, never opened) in both.
#
#   AZJ_DEV_NAME=e2e-mouse apps/macos/automation/mouse.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-mouse}"
case "$AZJ_DEV_NAME" in
  e2e*) ;;
  *) echo "mouse: use an e2e-prefixed dev name" >&2; exit 2 ;;
esac
fixtures="$(mktemp -d /tmp/azj-mouse.XXXXXX)"
fixtures="$(cd "$fixtures" && pwd -P)"
out="$root/.dev/e2e.out/mouse-$AZJ_DEV_NAME"
cleanup() {
  kill "${watchdog:-}" 2>/dev/null || true
  "$root/apps/macos/dev.sh" --reset >/dev/null
  rm -rf "$fixtures"
}
trap cleanup EXIT
"$root/apps/macos/dev.sh" --reset >/dev/null
mkdir -p "$out"
# Row 4 is an OSC 8 hyperlink (herdr passes those through); row 5 a link
# Azulejo refuses to open (not web or mail).
printf '%s\n' 'alpha beta gamma' 'see https://example.invalid/docs/page?x=1 for more' \
  'last line here' 'mail mailto:someone@example.invalid now' > "$fixtures/screen.txt"
printf 'go \033]8;;https://example.invalid/osc8\033\\click here\033]8;;\033\\ now\n' >> "$fixtures/screen.txt"
printf 'go \033]8;;ftp://example.invalid/file\033\\ftp link\033]8;;\033\\ now\n' >> "$fixtures/screen.txt"
cp "$here/mouse-app.py" "$fixtures/"
sed -e "s|@DIR@|$fixtures|g" "$here/mouse.azs.in" > "$fixtures/mouse.azs"
(sleep 180; "$root/apps/macos/dev.sh" --stop) &
watchdog=$!
"$root/apps/macos/dev.sh" --automation "$fixtures/mouse.azs" --out "$out"
result="$(cat "$out/result")"
[[ "$result" == ok ]] || { echo "mouse: $result"; exit 1; }
echo "terminal mouse: ok ($out)"
