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

# herdr adapter integration tests against a real herdr in a throwaway session
test-herdr:
    cargo test -p azulejo-herdr --test live -- --ignored --test-threads=1

# Build the macOS app into target/macos/Azulejo.app
mac:
    apps/macos/bundle.sh

# Build and launch the macOS app
mac-run: mac
    open target/macos/Azulejo.app

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
