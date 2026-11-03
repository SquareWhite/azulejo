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

# Build the macOS app into target/macos/Azulejo.app
mac:
    apps/macos/bundle.sh

# Build and launch the macOS app
mac-run: mac
    open target/macos/Azulejo.app

# Build the GTK4 Linux shell (needs GTK 4.14+ and libadwaita 1.5+)
linux:
    cargo build -p azulejo-linux

# Regenerate schema/api.v1.json (not implemented until step 1.2)
schema:
    @echo "schema generation lands in step 1.2" >&2

# Build GhosttyKit from vendor/ghostty
ghostty:
    scripts/build-ghostty.sh
