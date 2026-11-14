#!/usr/bin/env bash
# Builds Azulejo.app. Uses XcodeGen + xcodebuild when full Xcode is installed,
# otherwise SwiftPM plus a hand-assembled bundle (Command Line Tools only).
set -euo pipefail
cd "$(dirname "$0")"
root="$(cd ../.. && pwd)"
out="$root/target/macos"

if xcodebuild -version >/dev/null 2>&1; then
  xcodegen generate --quiet
  xcodebuild -project Azulejo.xcodeproj -scheme Azulejo -configuration Debug \
    -derivedDataPath "$out/DerivedData" build -quiet
  rm -rf "$out/Azulejo.app"
  cp -R "$out/DerivedData/Build/Products/Debug/Azulejo.app" "$out/Azulejo.app"
else
  echo "Full Xcode not found; building with SwiftPM." >&2
  (cd "$root" && cargo build -p azulejo-ffi)
  swift build
  app="$out/Azulejo.app"
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS"
  cp "$(swift build --show-bin-path)/Azulejo" "$app/Contents/MacOS/Azulejo"
  cp Resources/Info.plist "$app/Contents/Info.plist"
  codesign --force --sign - "$app" >/dev/null 2>&1
fi
echo "$out/Azulejo.app"
