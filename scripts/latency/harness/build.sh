#!/usr/bin/env bash
# Build LatHarness.app (copy of spikes/macos-terminal + in-process bench).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
XCF="$ROOT/vendor/ghostty/macos/GhosttyKit.xcframework/macos-arm64"
APP="$HERE/../build/LatHarness.app"

[[ -f "$XCF/libghostty.a" ]] || "$ROOT/scripts/build-ghostty.sh"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
sed -e 's/AzjSpike/LatHarness/g' -e 's/spike\.terminal/latency.harness/g' \
  -e 's#</dict>#  <key>LSUIElement</key><true/>\n</dict>#' "$HERE/Info.plist" > "$APP/Contents/Info.plist"

swiftc -O -target arm64-apple-macos14.0 -module-name LatHarness \
  -I "$XCF/Headers" -L "$XCF" -lghostty -lc++ \
  -framework AppKit -framework Metal -framework MetalKit -framework QuartzCore \
  -framework CoreText -framework CoreGraphics -framework CoreVideo -framework IOSurface \
  -framework Carbon -framework UniformTypeIdentifiers -framework UserNotifications \
  -framework ScreenCaptureKit -framework CoreMedia \
  "$HERE"/Sources/*.swift -o "$APP/Contents/MacOS/LatHarness"

codesign --force --sign - "$APP" >/dev/null
echo "==> $APP"
