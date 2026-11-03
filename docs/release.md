# Releasing Azulejo for macOS

How to build, sign, notarise and ship the macOS app (PLAN step 1.14).

## What you need

| Item | Why | How to check |
| --- | --- | --- |
| Apple Developer Program membership | Developer ID certificates and notarisation | developer.apple.com/account |
| **Developer ID Application** certificate in your login keychain | Signs the app, its helpers and the .dmg | `security find-identity -v -p codesigning` lists `Developer ID Application: Name (TEAMID)` |
| A **notarytool keychain profile** | Lets the script submit without putting a password on the command line | `xcrun notarytool history --keychain-profile <name>` |
| herdr on the target Mac | Azulejo doesn't bundle herdr | `herdr --version` |

Full Xcode isn't required. The Command Line Tools ship `codesign`, `notarytool` and `stapler`, and the build uses SwiftPM plus the runtime-Metal Ghostty patch.

## One-off setup

1. Create the certificate: Xcode ▸ Settings ▸ Accounts ▸ Manage Certificates ▸ **+** ▸ *Developer ID Application*. Without Xcode, make a CSR in Keychain Access and create the certificate on developer.apple.com. Either way, it ends up in your login keychain.
2. Make an app-specific password at appleid.apple.com (Sign-In and Security ▸ App-Specific Passwords).
3. Store the notary credentials once:

   ```sh
   xcrun notarytool store-credentials azulejo-notary \
     --apple-id you@example.com --team-id TEAMID --password <app-specific-password>
   ```

## Build

```sh
just release-mac
```

This does the following:

1. Checks that `VERSION` matches the workspace version in `Cargo.toml`.
2. Builds the core and `azj` with `cargo build --release` (arm64).
3. Builds GhosttyKit in ReleaseFast if it's missing. `AZJ_REBUILD_GHOSTTY=1` forces a rebuild, which is worth doing after bumping the Ghostty submodule.
4. Builds the Swift shell with `swift build -c release`, linking `target/release/libazulejo_ffi.a` through `AZJ_CORE_LIB_DIR`. It uses its own scratch path, so dev builds are left alone.
5. Assembles `target/release-mac/Azulejo.app`. `CFBundleShortVersionString` comes from `VERSION` and `CFBundleVersion` from the git commit count.
6. Signs with the hardened runtime, nested executables first. It uses `DEVELOPER_ID`, or the first `Developer ID Application` identity in the keychain. With neither, it signs **ad-hoc with a loud warning**: an ad-hoc build runs on this Mac but can't be notarised.
7. Writes `Azulejo-<version>.dmg` (the app plus an `/Applications` link, signed when there's an identity) and `Azulejo-<version>.zip`.

To pick a specific identity:

```sh
DEVELOPER_ID="Developer ID Application: Your Name (TEAMID)" just release-mac
```

### What's in the bundle

| Path | What |
| --- | --- |
| `Contents/MacOS/Azulejo` | The app, with the Rust core and libghostty statically linked |
| `Contents/MacOS/azj` | The CLI. "Install Command-Line Tool…" links it onto your PATH |
| `Contents/MacOS/azj-observe` | Decoder for read-only (observed) terminal tiles |
| `Contents/Resources/ghostty` | Ghostty's themes and shell integration |
| `Contents/Resources/terminfo` | Compiled `xterm-ghostty` terminfo |

libghostty finds the resources on its own: it climbs up from the executable to `Contents/Resources/terminfo/78/xterm-ghostty` and uses `Contents/Resources/ghostty` next to it. A `GHOSTTY_RESOURCES_DIR` in the app's environment takes priority. Without them, a named `theme =` in the Ghostty config fails with `theme "…" not found`. The dev build (`just mac`) bundles them too.

With the resources found, libghostty starts each tile's child (`herdr terminal attach`) with `TERM=xterm-ghostty` and `TERMINFO` pointing into the bundle; without them it falls back to `xterm-256color`. That only affects the attach client. Programs inside a pane see the `TERM` herdr sets, `xterm-256color`.

`azulejod` isn't shipped. The app embeds the core, and the headless daemon is a development and test tool (`just e2e`).

### Entitlements

`apps/macos/Resources/Azulejo.entitlements` is deliberately empty:

- WKWebView's JavaScript JIT runs in WebKit's own WebContent process, which carries its own entitlements.
- libghostty's runtime Metal shader compilation goes through the system Metal compiler service.
- Everything is statically linked, so library validation stays on.
- The app isn't sandboxed. It spawns herdr, previews arbitrary files and serves a Unix socket, and none of that fits the App Sandbox.

If browser tiles ever need a camera or microphone (`getUserMedia`), add `com.apple.security.device.camera` and `com.apple.security.device.audio-input`, plus the matching `NS…UsageDescription` strings.

## Notarise

Notarising uploads the .dmg to Apple, so it only runs when you ask for it explicitly:

```sh
AZJ_NOTARIZE=1 AZJ_NOTARY_PROFILE=azulejo-notary just release-notarize
```

It refuses ad-hoc or non-Developer-ID builds before uploading anything. It then runs `notarytool submit --wait`, staples the ticket to the .dmg and the app, rebuilds the .zip so it carries the ticket, and checks both with `spctl`.

## Check a build

```sh
codesign --verify --deep --strict --verbose=2 target/release-mac/Azulejo.app
spctl --assess --type execute --verbose=2 target/release-mac/Azulejo.app   # "accepted" only once notarised
hdiutil attach -nobrowse -readonly target/release-mac/Azulejo-*.dmg       # then detach
```

PLAN 1.14 is done when a notarised .dmg installs on a clean Mac and `azj` works from a fresh terminal after Azulejo ▸ Install Command-Line Tool….

## Install Command-Line Tool…

The menu item links `Contents/MacOS/azj` into:

1. `~/.local/bin`, when it's on your login shell's PATH (read with `$SHELL -l -c 'printenv PATH'`), else
2. `/usr/local/bin`, when it exists and is writable, else
3. `~/.local/bin`, with a hint to add it to your PATH.

It asks first and replaces only a symlink, such as one pointing at an older Azulejo.app. It never replaces a regular file. `AZJ_CLI_INSTALL_DIR` overrides the target directory. `just test-cli-installer` tests the logic against temp directories.

Whether the CLI should also ship as a Homebrew formula is still an open question in the PRD.
