# Azulejo for macOS

Swift 6 + AppKit shell over the Rust core (`core/crates/ffi`). `just mac`
builds `target/macos/Azulejo.app` (XcodeGen + xcodebuild with full Xcode,
SwiftPM otherwise).

## Dev profile: use this for all GUI testing

`just mac-run` launches the real thing: your config, your saved session and
**your own herdr session** (ADR 0008). Don't test with it. The dev profile
keeps everything under `.dev/` in the repo and uses its own herdr session,
`azj-dev`:

| | Dev profile |
| --- | --- |
| Config | `.dev/config/azulejo/config.toml` (generated, `[herdr] session = "azj-dev"`) |
| State | `.dev/state/state.json` |
| API socket | `.dev/run/azulejo.sock` |
| herdr session | `azj-dev` (override with `AZJ_DEV_SESSION`; `default` is refused) |
| Log | `.dev/logs/app.log` |

```sh
just mac-dev            # build, then launch on the dev profile (comes to the front)
just mac-dev --bg       # launch without activating or raising any window
just azj-dev tile list  # azj against the dev app
just mac-automation     # reset, then the in-process keyboard walkthrough
apps/macos/dev.sh --stop
just dev-reset          # quit, stop + delete the azj-dev herdr session, wipe .dev state
```

Several instances can run side by side (one per agent or experiment):
`AZJ_DEV_NAME=<name>` moves everything to `.dev/<name>/` and uses herdr session
`azj-dev-<name>`. Every recipe above honours it, e.g.
`AZJ_DEV_NAME=term just mac-dev --bg`, `AZJ_DEV_NAME=term just azj-dev tile list`,
`AZJ_DEV_NAME=term just dev-reset`. Names are letters, digits, `-` and `_`.

Two guards keep it off your real herdr session: `dev.sh` refuses
`AZJ_DEV_SESSION=default` (or a dev config pointing at it), and the app itself,
launched with `AZULEJO_PROFILE=dev`, refuses to boot unless its core options
name a herdr session other than `default`.

`dev.sh` passes everything through `open --env`:

- `AZULEJO_CORE_OPTIONS`: JSON for `azj_core_new_with_options` (paths, herdr session, `api_required`)
- `AZULEJO_PROFILE=dev`: the guard above; also turns on logging to `.dev/logs/app.log`
- `AZULEJO_PIDFILE`: so `--stop` can send SIGTERM (the app quits properly: state flushed, socket removed)
- `AZULEJO_BACKGROUND=1`: never `NSApp.activate()`; windows are ordered to the back
- `AZULEJO_AUTOMATION` / `AZULEJO_AUTOMATION_OUT`: run a script, then quit

## Automation (no OS input)

`apps/macos/automation/*.azs` scripts drive the app from inside: each `key`
line becomes an `NSEvent` posted to the app's own queue, so it goes through
the same local monitor and key router as a real keypress. Nothing reaches
other apps and no window comes to the front (automation runs as an accessory
app). `shot` renders the window's content to a PNG with `cacheDisplay`, so it
needs no Screen Recording permission. See the comment at the top of
`Automation.swift` for the commands. Results land in `<out>/automation.log`
and `<out>/result`.

## Layout of the code

| File | What |
| --- | --- |
| `main.swift` | Launch environment, dev-profile guard, boots `Core` |
| `Core.swift` | C ABI wrapper: `send`, `execute` (replies), `key`, `action`, `keymap`, `info`; events hop to the main queue |
| `AppDelegate.swift` | `AppController`: windows, refresh from `session.get`, actions the shell resolves (switch tab N, last tab, cycling, renames) |
| `WindowController.swift` | One window: sidebar, tab bar, tiles, HUD; persists its frame via `window.set_frame` |
| `TileContainerView.swift` | Lays out the active tab's tree; keeps one view per tile across tabs and layout changes; focus ring |
| `TileView.swift` | The `TileView` protocol and the placeholder tile |
| `KeyRouter.swift` | Local keyDown monitor → `azj_core_key` → action, or pass through |
| `MainMenu.swift` | Menus; shows each item's chord from the user's keymap |
| `Automation.swift` | The `.azs` runner |
| `TerminalTileView.swift` | Terminal tile: attach / read-only observe / ended / retrying, overlay, Respawn |
| `TerminalSurfaceView.swift` | One libghostty surface: keys (doubled `ctrl+b`), mouse, Edit menu |
| `TerminalRuntime.swift` | The process-wide `ghostty_app_t` and its callbacks (user's Ghostty config) |
| `../AzjObserve` | `azj-observe`: decodes `herdr terminal session observe` frames for read-only tiles |

## Terminal tiles

Each terminal tile is a libghostty surface (`GhosttyKit`, built from
`vendor/ghostty` by `scripts/build-ghostty.sh`; `bundle.sh` builds it when
missing) whose child is `herdr terminal attach <terminal-id>`. The core builds
that command (`azj_core_terminal_command`) with the inherited `HERDR_*`,
`CLAUDE*`, `AZULEJO_*` and `AZJ_*` variables removed. libghostty loses exit
codes on macOS, so the command writes its status and herdr's stderr to
`$TMPDIR/azulejo-term/<tile>/`, and `azj_core_attach_exit` reads them back
(ADR 0003):

| Attach ended because | Tile shows |
| --- | --- |
| another client took over | read-only (`observe`) + **Take back** |
| another client already had it | read-only + **Take over** |
| the pane is gone | **Session ended**: Respawn (new pane in the same slot) / Close |
| herdr unreachable / other failure | "herdr isn't answering", retries after 1, 2, 4 … 30 s |
| `ctrl+b q` (detach) | reattaches at once (3 in 10 s → retry backoff) |

Never retakes on its own. On the dev profile, overlay buttons can be pressed
without OS input by posting the distributed notification
`dev.azulejo.debug.terminal` with object `"takeover|respawn|close|retry <tile-id>"`.


### Key routing

Chords are resolved in the core (`azj_core_key`), not in Swift, so config
overrides, per-tile contexts (`cmd+a` toggles the sidebar only in terminals)
and prefix mode behave the same as they will on Linux. The router keeps
prefix state in the core and sticky resize mode in Swift. Bound chords are
consumed before AppKit sees them; anything unbound goes on to the focused
tile and then to menu key equivalents (⌘Q, ⌘C, ⌘V…). Chords use physical key
codes with US key names, so `alt+h` matches even though Option+H types `˙`.

### Writing a tile kind (1.7 terminal, 1.8 browser, 1.9 preview)

Implement `TileView` and register a maker at launch:

```swift
TileViews.register(.terminal) { tile in TerminalTileView(tile) }
```

- `update(_:)` runs on every core change to the tile (title, URL, path, herdr
  binding, `ended`). Make it cheap and idempotent.
- `setFocused(_:)`: take first responder when focused; the container draws the ring.
- `tileWillClose()` runs once before the view goes for good.
- The container reuses the view across splits, swaps, moves, zoom and tab
  switches, and only ever sets its frame.
- `keyContext` overrides the keymap context (defaults to the tile's kind).

Closing the last window quits the app (the session is saved and comes back on
relaunch); closing any other window closes it in the core, detaching its
terminals.
