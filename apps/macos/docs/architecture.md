# macOS shell: how it fits together

The shell is thin. The Rust core owns the session, layouts, keymap, config and
the herdr connection; the shell draws what the core says and sends it keys,
actions and API calls through `Core.swift`.

## Source layout

Everything is under `apps/macos/Sources/Azulejo/` unless noted.

| File | What |
| --- | --- |
| `main.swift` | Launch environment, the dev-profile guard, boots `Core` |
| `Core.swift` | C ABI wrapper: `send`, `execute` (replies), `executeAsync`, `key`, `action`, `keymap`, `info`; events hop to the main queue |
| `AppDelegate.swift` | `AppController`: windows, refresh from `session.get`, actions the shell resolves (switch tab N, last tab, cycling, renames) |
| `WindowController.swift` | One window: sidebar, tab bar, tiles, HUD; saves its frame through `window.set_frame` |
| `TileContainerView.swift` | Lays out the active tab's tree; keeps one view per tile across tabs and layout changes; the focus dim; layout motion |
| `Animation.swift` | `[animation]` settings, crop and scale geometry, `MotionAware`, the frame clock, motion stats |
| `TileView.swift`, `TileFrame.swift` | The `TileView` protocol, the placeholder tile, the header strip around every tile |
| `KeyRouter.swift` | Local keyDown monitor → `azj_core_key` → action, or pass through |
| `MainMenu.swift` | Menus; each item shows its chord from the user's keymap |
| `Automation.swift` | The `.azs` runner; other `*Automation.swift` files add verbs |
| `TerminalTileView.swift` | Terminal tile: attach, read-only observe, ended, retrying; overlay; Respawn |
| `TerminalSurfaceView.swift` | One libghostty surface: keys, IME, mouse, Edit menu |
| `TerminalRuntime.swift` | The process-wide `ghostty_app_t` and its callbacks (reads the user's Ghostty config) |
| `Design.swift` | Generated from `shared/design/tokens.json` by `just design-tokens`; never edit by hand |
| `../AzjObserve` | `azj-observe`: decodes `herdr terminal session observe` frames for read-only tiles |

## Terminal tiles

Each terminal tile is a libghostty surface (`GhosttyKit`, built from
`vendor/ghostty` by `scripts/build-ghostty.sh`) whose child process is
`herdr terminal attach <terminal-id>`. The core builds that command
(`azj_core_terminal_command`) with inherited `HERDR_*`, `CLAUDE*`,
`AZULEJO_*` and `AZJ_*` variables removed.

libghostty loses child exit codes on macOS, so the attach command writes its
status and herdr's stderr to `$TMPDIR/azulejo-term/<tile>/`, and
`azj_core_attach_exit` reads them back:

| Attach ended because | Tile shows |
| --- | --- |
| another client took over | read-only (`observe`) and **Take back** |
| another client already had it | read-only and **Take over** |
| the pane is gone | **Session ended**: Respawn (new pane in the same slot) or Close |
| herdr unreachable, or another failure | "herdr isn't answering", retrying after 1, 2, 4 … 30 s |
| `ctrl+b q` (detach) | reattaches at once (three times in 10 s → retry backoff) |

A tile never takes a pane back on its own. On the dev profile, posting the
distributed notification `dev.azulejo.debug.terminal` with object
`"takeover|respawn|close|retry <tile-id>"` presses an overlay button without
OS input.

While a tile shows a pane, Azulejo sets that pane's size, so the herdr TUI
showing the same pane draws it cropped or padded.

## Key routing

Chords resolve in the core (`azj_core_key`), not in Swift, so config
overrides, per-tile contexts (`cmd+a` opens the Agents sidebar only in terminals and the files list)
and prefix mode behave the same in every shell. The router keeps prefix state
in the core and sticky resize mode in Swift. Bound chords are consumed before
AppKit sees them; anything unbound goes to the focused tile and then to menu
key equivalents (⌘Q, ⌘C, ⌘V…). Chords use physical key codes with US key
names, so `alt+h` matches even though Option+H types `˙`.

## Writing a tile kind

Implement `TileView` and register a maker at launch:

```swift
TileViews.register(.terminal) { tile in TerminalTileView(tile) }
```

- `update(_:)` runs on every core change to the tile (title, URL, path, herdr
  binding, `ended`). Keep it cheap and idempotent. Header-only changes don't
  reach it.
- `setFocused(_:)`: take first responder when focused. The container dims the
  other tiles.
- `tileWillClose()` runs once before the view goes for good.
- The container reuses the view across splits, swaps, moves, zoom and tab
  switches, and only ever sets its frame.
- `keyContext` overrides the keymap context (defaults to the tile's kind).
- Conform to `TileHeaderAccessory` to put controls in the header strip, and to
  `AutomatableTile` to take `tile …` verbs and add `tile.*` keys in scripts.

`TileViews.make` wraps every tile view in a `TileFrame`, which forwards
`TileView`, `MotionAware` and `AutomatableTile`.

## Windows

Closing the last window quits the app: the session is saved and comes back on
relaunch, and herdr keeps every process. Closing any other window closes it
in the core, which closes its workspaces in herdr too, as ⌥⌘W does.

## herdr machines

The sidebar groups workspaces by herdr machine. Dev profiles turn herdr's
saved machines off, so to try machine groups,
`apps/macos/automation/machines.sh` sets up an instance (`AZJ_DEV_NAME=mm`)
whose config points at a fixture machine list and the SSH stand-in
(`scripts/ssh-standin.sh`), with a second throwaway herdr session playing the
remote:

```toml
[herdr]
session = "azj-dev-mm"
machines = true                      # on despite the named session
machines_file = "/…/machines.json"   # instead of `herdr machine list --json`
ssh = "/…/ssh"                       # the stand-in instead of ssh
```

Nothing connects to a real machine. Remote workspaces show no git branch:
reading git over the machine's SSH link isn't built.
