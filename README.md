# Azulejo

A tiling terminal workspace for macOS and Linux: terminals share one keyboard-driven layout, with a file tree in the sidebar; terminal sessions live in [herdr](https://herdr.dev).

## Setup

```sh
mise install        # Rust, Zig, XcodeGen, just, uv at pinned versions
just build          # core crates
just test
just mac            # macOS app → target/macos/Azulejo.app
just linux          # GTK4 shell (Linux only)
```

`just mac` uses XcodeGen and xcodebuild when full Xcode is installed, and falls back to SwiftPM with the Command Line Tools.

## Layout

| Path | What |
| --- | --- |
| `core/crates/` | Rust core: `model`, `protocol`, `herdr`, `engine`, `api`, `ffi`, `cli` (`azj`) |
| `apps/macos/` | Swift/AppKit shell; links the core through `ffi`'s C ABI |
| `apps/linux/` | GTK4 shell in Rust; links the core directly |
| `clients/python/` | Python client, generated from the schema |
| `schema/` | Generated JSON Schema for the socket API |

The Linux shell is a workspace member but not a default member, so `cargo build` works on macOS without GTK.

## Docs

- [apps/macos/README.md](apps/macos/README.md): the macOS shell, dev profile, automation
- [docs/keybindings.md](docs/keybindings.md): every default key
- [docs/themes.md](docs/themes.md): themes, terminal colours, `[appearance]`
- [docs/layout-files.md](docs/layout-files.md): open a project's tabs and tiles with one command
- [docs/herdr-notes.md](docs/herdr-notes.md): what herdr does and doesn't do
- [docs/dogfooding.md](docs/dogfooding.md): daily build and setup
- [clients/python/README.md](clients/python/README.md): the Python client

Agent working rules are in [CLAUDE.md](CLAUDE.md).
