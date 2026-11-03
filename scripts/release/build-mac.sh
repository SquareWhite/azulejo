#!/usr/bin/env bash
# Release build of Azulejo.app, plus a .dmg and a .zip (docs/release.md).
#
#   scripts/release/build-mac.sh            # sign with a Developer ID if one is found, else ad-hoc
#   DEVELOPER_ID="Developer ID Application: Name (TEAMID)" scripts/release/build-mac.sh
#   AZJ_REBUILD_GHOSTTY=1 scripts/release/build-mac.sh   # force a fresh ReleaseFast GhosttyKit
#
# Output: target/release-mac/{Azulejo.app, Azulejo-<version>.dmg, Azulejo-<version>.zip}
#
# Always builds the Swift shell with SwiftPM (works with or without full
# Xcode), against the release core in target/release. Never notarises: that's
# scripts/release/notarize.sh, and it needs explicit opt-in.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

version="$(tr -d '[:space:]' < VERSION)"
cargo_version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1 || true)"
if [[ "$version" != "$cargo_version" ]]; then
  echo "error: VERSION ($version) and Cargo.toml workspace version ($cargo_version) differ" >&2
  exit 1
fi
build_number="$(git rev-list --count HEAD 2>/dev/null || echo 1)"
out="$root/target/release-mac"
app="$out/Azulejo.app"

echo "==> Azulejo $version ($build_number)"

# 1. Core and CLI, release, arm64.
cargo build --release -p azulejo-ffi -p azulejo-cli

# 2. GhosttyKit. scripts/build-ghostty.sh builds ReleaseFast by default; it is
#    only rebuilt here when missing or when asked, because it takes minutes.
ghostty="$root/vendor/ghostty/macos/GhosttyKit.xcframework/macos-arm64/libghostty.a"
share="$root/vendor/ghostty/zig-out/share"
if [[ ! -f "$ghostty" || ! -d "$share/ghostty/themes" || ! -e "$share/terminfo/78/xterm-ghostty" \
  || "${AZJ_REBUILD_GHOSTTY:-0}" == 1 ]]; then
  OPTIMIZE=ReleaseFast scripts/build-ghostty.sh
fi

# 3. Swift shell, release. A separate scratch path keeps the release manifest
#    (AZJ_CORE_LIB_DIR) and objects apart from the dev build's.
swift_scratch="$root/target/macos/swiftpm-release"
(
  cd apps/macos
  AZJ_CORE_LIB_DIR="$root/target/release" \
    swift build -c release --arch arm64 --scratch-path "$swift_scratch"
)
bindir="$(cd apps/macos && AZJ_CORE_LIB_DIR="$root/target/release" \
  swift build -c release --arch arm64 --scratch-path "$swift_scratch" --show-bin-path)"

# 4. Assemble the bundle. azulejod is not shipped: the app embeds the core,
#    and the headless daemon is a development and test tool.
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bindir/Azulejo" "$app/Contents/MacOS/Azulejo"
cp "$bindir/AzjObserve" "$app/Contents/MacOS/azj-observe"
cp "$root/target/release/azj" "$app/Contents/MacOS/azj"
cp apps/macos/Resources/Info.plist "$app/Contents/Info.plist"
plutil -replace CFBundleShortVersionString -string "$version" "$app/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$build_number" "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
# Ghostty's resources, where libghostty finds them from the executable:
# Contents/Resources/terminfo/78/xterm-ghostty marks Contents/Resources/ghostty.
cp -R "$share/ghostty" "$app/Contents/Resources/ghostty"
cp -R "$share/terminfo" "$app/Contents/Resources/terminfo"

# 5. Sign. Developer ID if given or found, otherwise ad-hoc (runs locally,
#    can't be notarised, Gatekeeper rejects it on other Macs).
identity="${DEVELOPER_ID:-}"
if [[ -z "$identity" ]]; then
  identity="$(security find-identity -v -p codesigning 2>/dev/null \
    | sed -n 's/.*"\(Developer ID Application:.*\)"/\1/p' | head -1 || true)"
fi
entitlements="$root/apps/macos/Resources/Azulejo.entitlements"
if [[ -n "$identity" ]]; then
  echo "==> signing with: $identity"
  sign=(codesign --force --options runtime --timestamp --sign "$identity")
else
  cat >&2 <<'WARN'

  ************************************************************************
  *  WARNING: no Developer ID Application identity found.                *
  *  Signing AD-HOC. This build runs on this Mac only; it can't be       *
  *  notarised and Gatekeeper will block it elsewhere.                   *
  *  Set DEVELOPER_ID or install your certificate (docs/release.md).     *
  ************************************************************************

WARN
  sign=(codesign --force --options runtime --sign -)
fi
# Nested executables first, then the app with its entitlements.
for exe in azj azj-observe; do
  "${sign[@]}" "$app/Contents/MacOS/$exe"
done
"${sign[@]}" --entitlements "$entitlements" "$app"
codesign --verify --deep --strict --verbose=2 "$app"

# 6. .dmg (app + /Applications link) and .zip.
dmg="$out/Azulejo-$version.dmg"
zip="$out/Azulejo-$version.zip"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp -R "$app" "$stage/Azulejo.app"
ln -s /Applications "$stage/Applications"
rm -f "$dmg" "$zip"
hdiutil create -quiet -volname "Azulejo $version" -srcfolder "$stage" -ov -format UDZO "$dmg"
if [[ -n "$identity" ]]; then
  codesign --force --timestamp --sign "$identity" "$dmg"
fi
ditto -c -k --keepParent "$app" "$zip"

echo "==> $app"
echo "==> $dmg"
echo "==> $zip"
if [[ -z "$identity" ]]; then
  echo "==> ad-hoc signed: not notarisable (docs/release.md)"
fi
