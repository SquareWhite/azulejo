#!/usr/bin/env bash
# Regenerates the app icon from scripts/icon/draw.swift (`just icon`):
#   apps/macos/Resources/AppIcon.icns   the bundle's icon (iconutil, 16–1024 px)
#   docs/design/icon/*.png              candidates, dark and tinted variants, sheet.png
#   scripts/icon/AppIcon.sha256         stamp: draw.swift and the .icns it made
#
#   scripts/icon/build.sh            # regenerate everything
#   scripts/icon/build.sh --check    # fail if the .icns or draw.swift changed without a regenerate (just lint)
#
# Command Line Tools only: swift and /usr/bin/iconutil.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
src=scripts/icon/draw.swift
icns=apps/macos/Resources/AppIcon.icns
stamp=scripts/icon/AppIcon.sha256

stamp_now() {
  shasum -a 256 "$src" "$icns"
}

if [[ "${1:-}" == --check ]]; then
  if [[ ! -f "$icns" || ! -f "$stamp" ]]; then
    echo "error: $icns or $stamp is missing; run 'just icon'" >&2
    exit 1
  fi
  if ! diff -q <(stamp_now) "$stamp" >/dev/null; then
    echo "error: $icns is stale against $src (or was edited by hand); run 'just icon'" >&2
    diff <(stamp_now) "$stamp" >&2 || true
    exit 1
  fi
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
swift "$src" --iconset "$work/AppIcon.iconset" --docs docs/design/icon
/usr/bin/iconutil -c icns "$work/AppIcon.iconset" -o "$icns"
stamp_now > "$stamp"
echo "==> $icns"
echo "==> docs/design/icon/sheet.png"
