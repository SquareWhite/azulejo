#!/usr/bin/env bash
# Builds Azulejo.app. Uses XcodeGen + xcodebuild when full Xcode is installed,
# otherwise SwiftPM plus a hand-assembled bundle (Command Line Tools only).
set -euo pipefail
cd "$(dirname "$0")"
root="$(cd ../.. && pwd)"
out="$root/target/macos"

# libghostty (terminal tiles) and its resources. Built once from
# vendor/ghostty; rebuild with scripts/build-ghostty.sh after bumping the
# submodule.
share="$root/vendor/ghostty/zig-out/share"
if [[ ! -f "$root/vendor/ghostty/macos/GhosttyKit.xcframework/macos-arm64/libghostty.a" \
  || ! -d "$share/ghostty/themes" || ! -e "$share/terminfo/78/xterm-ghostty" ]]; then
  "$root/scripts/build-ghostty.sh"
fi

# Ghostty's resources go where libghostty looks for them: it climbs from the
# executable to Contents/Resources/terminfo/78/xterm-ghostty and then uses
# Contents/Resources/ghostty (themes, shell integration).
add_ghostty_resources() {
  local res="$1/Contents/Resources"
  mkdir -p "$res"
  rm -rf "$res/ghostty" "$res/terminfo"
  cp -R "$share/ghostty" "$res/ghostty"
  cp -R "$share/terminfo" "$res/terminfo"
}

# The app icon (Info.plist's CFBundleIconFile) and its dark look, which the
# app swaps into the Dock in dark mode (DockIcon.swift). `just icon`
# regenerates both from scripts/icon/draw.swift; docs/design/icon/README.md.
add_icon() {
  mkdir -p "$1/Contents/Resources"
  cp Resources/AppIcon.icns "$1/Contents/Resources/AppIcon.icns"
  cp Resources/AppIconDark.icns "$1/Contents/Resources/AppIconDark.icns"
}

if xcodebuild -version >/dev/null 2>&1; then
  xcodegen generate --quiet
  xcodebuild -project Azulejo.xcodeproj -scheme Azulejo -configuration Debug \
    -derivedDataPath "$out/DerivedData" build -quiet
  rm -rf "$out/Azulejo.app"
  cp -R "$out/DerivedData/Build/Products/Debug/Azulejo.app" "$out/Azulejo.app"
  cp "$out/DerivedData/Build/Products/Debug/azj-observe" "$out/Azulejo.app/Contents/MacOS/azj-observe"
  add_ghostty_resources "$out/Azulejo.app"
  add_icon "$out/Azulejo.app"
  codesign --force --sign - "$out/Azulejo.app" >/dev/null 2>&1
else
  echo "Full Xcode not found; building with SwiftPM." >&2
  (cd "$root" && cargo build -p azulejo-ffi)
  # SwiftPM doesn't track the Rust static library or its generated header, so
  # a core change alone wouldn't relink (or recompile against a new header).
  # Drop the stale outputs so it has to.
  lib="$root/target/debug/libazulejo_ffi.a"
  header="$root/core/crates/ffi/include/azulejo.h"
  ghostty="$root/vendor/ghostty/macos/GhosttyKit.xcframework/macos-arm64/libghostty.a"
  bindir="$(swift build --show-bin-path)"
  bin="$bindir/Azulejo"
  if [[ -f "$bin" && "$header" -nt "$bin" ]]; then
    rm -rf "$bindir/Azulejo.build" "$bindir/Modules" "$bindir/ModuleCache" "$bin"
    # SwiftPM's cached build plan still points into Azulejo.build (its
    # output-file-map.json): drop it too, or the next build fails on the
    # missing file instead of planning again.
    rm -f "$bindir/description.json" "$bindir/plugin-tools-description.json"
  elif [[ -f "$bin" && ( "$lib" -nt "$bin" || "$ghostty" -nt "$bin" ) ]]; then
    rm -f "$bin"
  fi
  swift build
  app="$out/Azulejo.app"
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS"
  cp "$bindir/Azulejo" "$app/Contents/MacOS/Azulejo"
  cp "$bindir/AzjObserve" "$app/Contents/MacOS/azj-observe"
  cp Resources/Info.plist "$app/Contents/Info.plist"
  add_ghostty_resources "$app"
  add_icon "$app"
  codesign --force --sign - "$app" >/dev/null 2>&1
fi
echo "$out/Azulejo.app"
