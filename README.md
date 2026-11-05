# Azulejo

A tiling terminal workspace for macOS and Linux. Terminals, browsers, previews and a file browser share one keyboard-driven layout; terminal sessions live in [herdr](https://herdr.dev). See [docs/PRD.md](docs/PRD.md) for the what and [PLAN.md](PLAN.md) for the build plan.

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
| `shared/webview/` | JS and CSS shared by both shells' webviews |
| `clients/python/` | Python client, generated from the schema |
| `schema/` | Generated JSON Schema for the socket API |

The Linux shell is a workspace member but not a default member, so `cargo build` works on macOS without GTK.

## Repository privacy

Use fictional usernames, `example.invalid` addresses, sample project names and throwaway machines in fixtures and examples. Resolve home directories and checkout paths at runtime instead of embedding a developer's paths.

Keep app and desktop captures, terminal dumps, logs, crash reports, local configuration and signing credentials out of Git. Capture directories and common credential files are ignored; generated icon artwork remains tracked. Review issue attachments separately: ignored files can still contain private data.

Published history was sanitized to remove personal and workstation details. Do not merge an old clone's history back into this repository; start from a fresh clone or replay only reviewed changes.
