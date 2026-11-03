#!/usr/bin/env bash
# Compiles CLIInstaller.swift with its tests and runs them (temp dirs only).
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
# Top-level test code has to live in main.swift.
cp "$root/scripts/release/test-cli-installer.swift" "$dir/main.swift"
swiftc -O "$root/apps/macos/Sources/Azulejo/CLIInstaller.swift" "$dir/main.swift" -o "$dir/test"
"$dir/test"
