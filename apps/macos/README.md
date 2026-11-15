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

## Tile headers and branches (F26, F28, F29; PLAN 2.14, 2.15)

`TileViews.make` wraps every tile view in a `TileFrame`: a 28 pt header strip
(kind icon, bold title, muted subtitle) above the tile's own view. The text is
the core's `tile.header`, so every shell says the same; header-only changes
don't reach the tile view (a terminal doesn't re-settle on every command). A
tile view that wants controls in the strip conforms to `TileHeaderAccessory`
(preview: syntax, Preview | Source, Open, Reveal; files: filter, hidden). The
frame forwards `TileView`, `MotionAware` and `AutomatableTile`, adding
`tile.header.title` / `tile.header.subtitle` to automation state.

The sidebar shows each workspace's branch from `workspace.git`.
`apps/macos/automation/headers.sh` checks headers, a branch switch in a tile
and the preview toggle across a relaunch, on dev instance `hdr`.

## Window chrome (M2.5 D.2, D.4, D.8)

No visible title bar: the traffic lights sit in the sidebar card's top band
(`TrafficLights` makes the transparent title bar as tall as the toolbar row and
places them), and the toolbar row holds the tab strip (`TabStrip.swift`: state
icon + name per tab, "+") and the search field (`ToolbarSearch.swift`: opens
⌘K as a dropdown under itself; its badge shows `quick_open`'s chord). Empty
toolbar space drags the window; a double click there follows the system's
title-bar setting. `apps/macos/automation/toolbar.sh` checks where clicks land
under the title bar, the tab icons, "+", the dropdown and the hidden-sidebar
row, and takes window captures for comparing with `docs/design/mockup-v1.png`.

## Sidebar (M2.5 D.3)

`Sidebar.swift` is what sits in that card below its top band: an Agents |
Files switch (remembered per viewer), then either the workspace list or the
Files view, and a footer counting workspaces by state. Agents: one group per
herdr machine (connection dot, or a spinner and "Reconnecting…"), workspace
rows with a status icon (blocked, then done-unseen, working, done, idle), the
name and the git branch, and "+" per group. Files: a `files.panel` on the
focused workspace's repository (else its cwd), the files tile's tree and keys
without its card; Return opens a file beside the focused tile, Esc gives the
keyboard back, and refreshes don't take it while the tree has it
(`KeepsKeyboard` in `TileFocus.swift`). `apps/macos/automation/sidebar.sh`
checks both views with fixture repositories (`sidebar mode|select|focus|files
…` in scripts; `sidebar.*` state keys).

## Right-click menus

Workspace rows (Agents view) and tile header strips have standard NSMenus
(`ContextMenus.swift`). Rows: Rename…, New Tab, Reveal Folder in Finder, Open
Folder in Files Tile, Copy Path, Close Workspace (the folder items only when
the workspace has a repository or cwd; Reveal and Open in Files Tile only on
Local). Headers: Split Right, Split Down, Zoom, Rename…, Copy Title, Copy
Working Directory (terminals), Reveal in Finder, Close Tile, and Kill Pane…
for bound terminals (asks first). The browser has no header strip: these
items follow the page's own in its chrome's menu. Every keymap item runs its
action through `AppController.perform(action:target:)`, aimed at the row's
workspace or the header's tile, and shows its chord from `commands.list`.
A terminal's body keeps its own right-click (libghostty, mouse-reporting
apps). `apps/macos/automation/menus.sh` builds both menus without a mouse
(`menu workspace N|tile [invoke TITLE]`, `menu answer`, `menu shot`; state
`menu.items`, `menu.copied`, `menu.revealed`, `workspace.names`), checks herdr
keeps a closed workspace's panes, and that a relaunch doesn't bring it back.

## Layout motion (F4, PLAN 2.1)

Layout changes animate: split, close, move, swap, resize, zoom, the tall
actions and the sidebar opening or closing (`toggle_sidebar`: the card slides
and fades while the tile area grows or shrinks with it, a gutter apart), plus
tab/workspace switches. Every tile goes to its
final frame at the start of a motion and Core Animation carries it there
visually, so nothing re-lays out per frame. A terminal that changes size first
covers itself with an IOSurface copy of its last frame, resizes its PTY once
(ADR 0004; at most one per 50 ms while a key repeats), and shows the live
surface again on the first fresh frame at the new size, or 250 ms after the
motion at the latest. A change that arrives mid-motion starts from where every
tile appears on screen.

`style` picks what content does on the way:

- `crop` (default): the tile's frame is a clip. Its bounds and position
  animate; content stays at 1:1 and sticks to the edge that moves least, so
  the moving edge reveals or cuts it off. A shrinking terminal shows its
  snapshot at 1:1 until the motion ends; a growing one shows live content as
  soon as it has drawn at the new size. New tiles sweep in with the divider,
  closing tiles shrink away from the neighbour taking their space, and the
  tall rotate cross-fades.
- `scale`: a transform scales the whole tile, so text stretches mid-motion.
- `outline`: tiles jump to their new place hidden, outlines move instead, and
  content fades in over 80 ms at the end.

```toml
[animation]
enabled = true            # false: changes are instant
duration_ms = 200         # 0–1000
curve = "ease_out"        # "ease_out", "ease_in_out", "linear"
style = "crop"            # "crop", "scale", "outline"
tab_switch = "fade"       # tab and workspace switches: "fade", "slide", "none"
reduce_motion = "system"  # "system": no motion while macOS Reduce Motion is on; "ignore"
```

### Focus: unfocused tiles dim

There's no outline round the focused tile. Every other tile on screen dims,
header strip included: the window colour laid over it at 25 %
(`alpha.unfocusedDim`, `color.dimFill` in `shared/design/tokens.json`), which
turns the mockup's focused tile colours into its unfocused ones. Nothing dims
with one tile on screen (a lone tile, Monocle, zoom). The dim follows the
focused tile, not the keyboard: with ⌘K open, the sidebar's Files panel or the
search field in use, the last focused tile stays bright. It doesn't change
when the window isn't key, as in Ghostty. Focus moving cross-fades it in
120 ms on the motion curve; instantly with motion off or under Reduce Motion.

Ghostty's own settings for this win when its config sets them, read by the
core from Ghostty's config files: `unfocused-split-opacity` (0.15–1; the dim
is 1 − that, so `1` turns dimming off) and `unfocused-split-fill` (hex). A
dev profile reads `.dev/<name>/config/ghostty/config` instead, so dev runs
never depend on your own Ghostty config for them. `[animation] dim_inactive`
is retired: it only warns now.

Changes apply on save (config hot reload). `apps/macos/automation/anim.sh`
runs a 60-animation stress on a dev instance (frame pacing from the display
link, PTY resizes per motion, a capture after the stress); `--slow` runs
1.5 s motions with mid-motion window captures. `AZJ_ANIM_STYLE=crop|scale|outline`
runs one style regardless of the config. The app can't run
`screencapture` itself, so `capture` lines drop a request the script takes.

## Layout of the code

| File | What |
| --- | --- |
| `main.swift` | Launch environment, dev-profile guard, boots `Core` |
| `Core.swift` | C ABI wrapper: `send`, `execute` (replies), `key`, `action`, `keymap`, `info`; events hop to the main queue |
| `AppDelegate.swift` | `AppController`: windows, refresh from `session.get`, actions the shell resolves (switch tab N, last tab, cycling, renames) |
| `WindowController.swift` | One window: sidebar, tab bar, tiles, HUD; persists its frame via `window.set_frame` |
| `TileContainerView.swift` | Lays out the active tab's tree; keeps one view per tile across tabs and layout changes; the focus dim; layout motion, the sidebar's included |
| `Animation.swift` | `[animation]` settings, crop and scale geometry, `MotionAware` (terminal freeze/reveal), frame clock, motion stats |
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
- `setFocused(_:)`: take first responder when focused; the container dims the other tiles.
- `tileWillClose()` runs once before the view goes for good.
- The container reuses the view across splits, swaps, moves, zoom and tab
  switches, and only ever sets its frame.
- `keyContext` overrides the keymap context (defaults to the tile's kind).

Closing the last window quits the app (the session is saved and comes back on
relaunch); closing any other window closes it in the core, detaching its
terminals.

## herdr machines (ADR 0011)

Dev profiles use a named herdr session, so herdr's saved machines are off
there: a dev run never reaches a real machine. To try the sidebar's machine
groups, `apps/macos/automation/machines.sh` sets up an instance
(`AZJ_DEV_NAME=mm`) whose config points at a fixture machine list and the ssh
stand-in (`scripts/ssh-standin.sh`), with a throwaway herdr session playing
the remote:

```toml
[herdr]
session = "azj-dev-mm"
machines = true                 # on despite the named session
machines_file = "/…/machines.json"  # instead of `herdr machine list --json`
ssh = "/…/ssh"                  # the stand-in instead of ssh
```

