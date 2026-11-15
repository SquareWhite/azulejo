#!/usr/bin/env bash
# Regenerates the app icon from scripts/icon/draw.swift (`just icon`):
#   apps/macos/Resources/AppIcon.icns       the bundle's icon (iconutil, 16–1024 px)
#   apps/macos/Resources/AppIconDark.icns   the dark look, swapped into the Dock at run time
#   scripts/icon/composer/*.svg             the chosen icon's layers, for Icon Composer
#   docs/design/icon/*.png                  candidates, variants, sheet.png, prompt.png
#   scripts/icon/AppIcon.sha256             stamp: draw.swift and the .icns files it made
#
#   scripts/icon/build.sh            # regenerate everything
#   scripts/icon/build.sh --check    # fail if an .icns or draw.swift changed without a regenerate (just lint)
#
# Command Line Tools only: swift and /usr/bin/iconutil.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
src=scripts/icon/draw.swift
icns=apps/macos/Resources/AppIcon.icns
dark=apps/macos/Resources/AppIconDark.icns
stamp=scripts/icon/AppIcon.sha256

stamp_now() {
  shasum -a 256 "$src" "$icns" "$dark"
}

if [[ "${1:-}" == --check ]]; then
  if [[ ! -f "$icns" || ! -f "$dark" || ! -f "$stamp" ]]; then
    echo "error: $icns, $dark or $stamp is missing; run 'just icon'" >&2
    exit 1
  fi
  if ! diff -q <(stamp_now) "$stamp" >/dev/null; then
    echo "error: $icns or $dark is stale against $src (or was edited by hand); run 'just icon'" >&2
    diff <(stamp_now) "$stamp" >&2 || true
    exit 1
  fi
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
swift "$src" --iconset "$work/AppIcon.iconset" --iconset-dark "$work/AppIconDark.iconset" \
  --composer scripts/icon/composer --docs docs/design/icon
/usr/bin/iconutil -c icns "$work/AppIcon.iconset" -o "$icns"
/usr/bin/iconutil -c icns "$work/AppIconDark.iconset" -o "$dark"
stamp_now > "$stamp"
echo "==> $icns"
echo "==> $dark"
echo "==> docs/design/icon/sheet.png, prompt.png"
