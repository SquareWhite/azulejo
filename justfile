# Azulejo tasks. Run `mise install` first so the pinned toolchains are on PATH.

default: build

# Build the core (all crates except the Linux shell)
build:
    cargo build

# Run core tests
test:
    cargo test

# Lint: formatting and clippy on the core
lint:
    cargo fmt --all --check
    cargo clippy --all-targets -- -D warnings
    scripts/design-tokens --check
    cargo run -q -p azulejo-config --bin azj-keys-doc -- --check docs/keybindings.md
    scripts/icon/build.sh --check

# Regenerate the macOS shell's Design.swift from shared/design/tokens.json
design-tokens:
    scripts/design-tokens

# Regenerate the app icon (apps/macos/Resources/AppIcon.icns) and docs/design/icon from scripts/icon/draw.swift
icon:
    scripts/icon/build.sh

# Regenerate the keybinding reference in docs/keybindings.md from shared/config/keymap.default.toml
keys-doc:
    cargo run -q -p azulejo-config --bin azj-keys-doc -- docs/keybindings.md

# herdr adapter and engine integration tests against a real herdr in throwaway sessions
test-herdr:
    cargo test -p azulejo-herdr --test live -- --ignored --test-threads=1
    cargo test -p azulejo-engine --test live -- --ignored --test-threads=1
    cargo test -p azulejo-engine --test live_layout -- --ignored --test-threads=1

# Headless end to end: azulejod + azj against a real herdr in a throwaway session
e2e:
    cargo build -p azulejo-cli -p azulejo-daemon
    cargo test -p azulejo-daemon --test e2e -- --ignored --test-threads=1 --nocapture

# Python client (clients/python): check the generated code, then pytest
# against a headless azulejod (throwaway sockets; terminal tests use a
# throwaway herdr session).
python-test:
    cargo build -p azulejo-daemon
    scripts/codegen-python --check
    cd clients/python && uv run --locked pytest -q

# Regenerate the Python client from schema/api.v1.json
python-codegen:
    scripts/codegen-python

# Build the macOS app into target/macos/Azulejo.app
mac:
    apps/macos/bundle.sh

# Build and launch the macOS app (your real config, state and herdr session)
mac-run: mac
    open target/macos/Azulejo.app

# Build and launch the macOS app on the isolated dev profile (.dev/, herdr session azj-dev).
# Args go to apps/macos/dev.sh: --bg, --automation FILE, --wait, --stop
mac-dev *args: mac
    apps/macos/dev.sh {{args}}

# Run azj against the dev app's socket (AZJ_DEV_NAME=<name> picks an instance)
azj-dev *args:
    cargo build -q -p azulejo-cli
    env "$(apps/macos/dev.sh --env)" target/debug/azj {{args}}

# Quit the dev app, stop and delete its herdr session (azj-dev[-<AZJ_DEV_NAME>]), wipe its .dev state
dev-reset:
    apps/macos/dev.sh --reset

# Keyboard walkthrough of every F2 and tall action on the dev profile, in-process (no OS input)
# (resets the dev profile first so it starts from one tile)
mac-automation: mac
    apps/macos/dev.sh --reset
    out=.dev/out/keys; if [ -n "${AZJ_DEV_NAME:-}" ]; then out=".dev/$AZJ_DEV_NAME/out/keys"; fi; apps/macos/dev.sh --automation apps/macos/automation/keys.azs --out "$out"

# Keyboard walkthrough of the dynamic layouts (ADR 0010) on the dev profile, in-process
mac-layouts: mac
    apps/macos/dev.sh --reset
    out=.dev/out/layouts; if [ -n "${AZJ_DEV_NAME:-}" ]; then out=".dev/$AZJ_DEV_NAME/out/layouts"; fi; apps/macos/dev.sh --automation apps/macos/automation/layouts.azs --out "$out"

# Key and focus suite (2.11): chords and first responder across tiles, the ⌘K
# menu, layouts, tabs, herdr changes and input methods; in-process, dev profile
mac-focus: mac
    apps/macos/automation/focus/focus.sh

# Live rebinding of every keymap context through the dev profile's config file (no restart)
mac-rebind: mac
    apps/macos/automation/rebind.sh

# The designer mockup's scene with fixture data, captured for docs/design (D.9)
mac-mockup: mac
    apps/macos/automation/mockup.sh

# Lint the GTK4 Linux shell (needs GTK 4.14+ and libadwaita 1.5+)
lint-linux:
    cargo clippy -p azulejo-linux --all-targets -- -D warnings

# Build the GTK4 Linux shell (needs GTK 4.14+ and libadwaita 1.5+)
linux:
    cargo build -p azulejo-linux

# Regenerate schema/api.v1.json from the protocol crate. `just test` fails while it's stale.
schema:
    cargo run -q -p azulejo-protocol --bin azj-schema schema/api.v1.json
    scripts/codegen-python

# Build GhosttyKit from vendor/ghostty
ghostty:
    scripts/build-ghostty.sh

# Release build into target/release-mac (.app, .dmg, .zip); Developer ID if found, else ad-hoc
release-mac:
    scripts/release/build-mac.sh

# Notarise + staple the release .dmg (uploads to Apple; needs AZJ_NOTARIZE=1, AZJ_NOTARY_PROFILE)
release-notarize:
    scripts/release/notarize.sh

# Build HEAD, swap it into /Applications, relaunch (--check, --dirty, --list, --rollback, --to <hash>, --no-launch)
dogfood *args:
    scripts/release/dogfood.sh {{args}}

# Reinstall the build before the current one
dogfood-rollback *args:
    scripts/release/dogfood.sh --rollback {{args}}

# Tests for `just dogfood` (temp dirs; never touches /Applications or a running Azulejo)
test-dogfood:
    scripts/release/test-dogfood.sh

# Tests for the "Install Command-Line Tool" logic (temp dirs only)
test-cli-installer:
    scripts/release/test-cli-installer.sh
