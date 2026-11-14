#!/usr/bin/env bash
# Build GhosttyKit.xcframework from vendor/ghostty.
#
# With Xcode installed this uses Ghostty's own flow (`zig build
# -Demit-xcframework`). Without Xcode (CommandLineTools only) there is no
# offline `metal` compiler and no `xcodebuild -create-xcframework`, so we set
# AZJ_RUNTIME_METAL=1 (vendor patch: shaders compile at runtime and the
# static lib is installed directly) and assemble the xcframework by hand.
#
# Output: vendor/ghostty/macos/GhosttyKit.xcframework, plus Ghostty's
# resources (themes, terminfo, shell integration) in vendor/ghostty/zig-out/share
# for the app bundle: Contents/Resources/{ghostty,terminfo}.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GHOSTTY="$ROOT/vendor/ghostty"
ZIG_VERSION="0.17.0"
OPTIMIZE="${OPTIMIZE:-ReleaseFast}"
OUT="$GHOSTTY/macos/GhosttyKit.xcframework"

zig() { mise exec "zig@$ZIG_VERSION" -- zig "$@"; }

mise install "zig@$ZIG_VERSION" >/dev/null

cd "$GHOSTTY"

if xcrun -sdk macosx -f metal >/dev/null 2>&1 && xcodebuild -version >/dev/null 2>&1; then
  echo "==> Xcode found: official xcframework build ($OPTIMIZE, native)"
  zig build -Doptimize="$OPTIMIZE" -Demit-xcframework=true -Demit-macos-app=false \
    -Dxcframework-target=native
else
  echo "==> No Xcode: runtime-Metal build of the static lib ($OPTIMIZE)"
  PATCH="$ROOT/scripts/patches/ghostty-no-xcode.patch"
  if git apply --check "$PATCH" 2>/dev/null; then
    echo "==> applying $(basename "$PATCH")"
    git apply "$PATCH"
  elif ! git apply --reverse --check "$PATCH" 2>/dev/null; then
    echo "error: $PATCH neither applies nor is already applied" >&2
    exit 1
  fi
  AZJ_RUNTIME_METAL=1 zig build -Doptimize="$OPTIMIZE" \
    -Demit-xcframework=false -Demit-macos-app=false

  # Assemble next to the old one and swap it in, so a dev build linking the
  # old framework at the same moment never sees a half-written one.
  tmp="$OUT.tmp"
  rm -rf "$tmp"
  slice="$tmp/macos-arm64"
  mkdir -p "$slice/Headers"
  cp zig-out/lib/libghostty.a "$slice/libghostty.a"
  cp include/ghostty.h include/module.modulemap "$slice/Headers/"
  cat >"$tmp/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AvailableLibraries</key>
	<array>
		<dict>
			<key>BinaryPath</key>
			<string>libghostty.a</string>
			<key>HeadersPath</key>
			<string>Headers</string>
			<key>LibraryIdentifier</key>
			<string>macos-arm64</string>
			<key>LibraryPath</key>
			<string>libghostty.a</string>
			<key>SupportedArchitectures</key>
			<array>
				<string>arm64</string>
			</array>
			<key>SupportedPlatform</key>
			<string>macos</string>
		</dict>
	</array>
	<key>CFBundlePackageType</key>
	<string>XFWK</string>
	<key>XCFrameworkFormatVersion</key>
	<string>1.0</string>
</dict>
</plist>
PLIST
  rm -rf "$OUT.old"
  [[ -d "$OUT" ]] && mv "$OUT" "$OUT.old"
  mv "$tmp" "$OUT"
  rm -rf "$OUT.old"
fi

for sentinel in share/ghostty/themes share/ghostty/shell-integration share/terminfo/78/xterm-ghostty; do
  [[ -e "zig-out/$sentinel" ]] || { echo "error: zig-out/$sentinel missing after the build" >&2; exit 1; }
done

echo "==> $OUT"
echo "==> $GHOSTTY/zig-out/share/{ghostty,terminfo}"
