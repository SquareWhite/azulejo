#!/usr/bin/env bash
# Build AzjSpike.app with swiftc (CommandLineTools is enough; no Xcode).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
XCF="$ROOT/vendor/ghostty/macos/GhosttyKit.xcframework/macos-arm64"
APP="$HERE/build/AzjSpike.app"

[[ -f "$XCF/libghostty.a" ]] || "$ROOT/scripts/build-ghostty.sh"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$HERE/Info.plist" "$APP/Contents/Info.plist"

swiftc -O -target arm64-apple-macos14.0 -module-name AzjSpike \
  -I "$XCF/Headers" -L "$XCF" -lghostty -lc++ \
  -framework AppKit -framework Metal -framework MetalKit -framework QuartzCore \
  -framework CoreText -framework CoreGraphics -framework CoreVideo -framework IOSurface \
  -framework Carbon -framework UniformTypeIdentifiers -framework UserNotifications \
  "$HERE"/Sources/*.swift -o "$APP/Contents/MacOS/AzjSpike"

codesign --force --sign - "$APP" >/dev/null
echo "==> $APP"
