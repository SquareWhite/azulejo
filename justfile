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

# herdr adapter and engine integration tests against a real herdr in throwaway sessions
test-herdr:
    cargo test -p azulejo-herdr --test live -- --ignored --test-threads=1
    cargo test -p azulejo-engine --test live -- --ignored --test-threads=1

# Headless end to end: azulejod + azj against a real herdr in a throwaway session
e2e:
    cargo build -p azulejo-cli -p azulejo-daemon
    cargo test -p azulejo-daemon --test e2e -- --ignored --test-threads=1 --nocapture

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

# Run azj against the dev app's socket
azj-dev *args:
    cargo build -q -p azulejo-cli
    AZULEJO_SOCKET="{{justfile_directory()}}/.dev/run/azulejo.sock" target/debug/azj {{args}}

# Quit the dev app, stop and delete its herdr session (azj-dev), wipe .dev state
dev-reset:
    apps/macos/dev.sh --reset

# Keyboard walkthrough of every F2 and tall action on the dev profile, in-process (no OS input)
# (resets the dev profile first so it starts from one tile)
mac-automation: mac
    apps/macos/dev.sh --reset
    apps/macos/dev.sh --automation apps/macos/automation/keys.azs --out .dev/out/keys

# Lint the GTK4 Linux shell (needs GTK 4.14+ and libadwaita 1.5+)
lint-linux:
    cargo clippy -p azulejo-linux --all-targets -- -D warnings

# Build the GTK4 Linux shell (needs GTK 4.14+ and libadwaita 1.5+)
linux:
    cargo build -p azulejo-linux

# Regenerate schema/api.v1.json from the protocol crate. `just test` fails while it's stale.
schema:
    cargo run -q -p azulejo-protocol --bin azj-schema schema/api.v1.json

# Build GhosttyKit from vendor/ghostty
ghostty:
    scripts/build-ghostty.sh
