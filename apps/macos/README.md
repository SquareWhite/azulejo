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
- `AZJ_DEBUG_MIRROR`, when set in `dev.sh`'s environment: the core logs every mirror push and pull (ADR 0012) to the app log
- `AZJ_FILES_LIST_DELAY_MS`, likewise: the core holds every folder listing back that long, standing in for a slow folder (`automation/files-slow.sh`)

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

### Monocle tabs

In a Monocle tab with more than one tile, the strip on screen shows the whole
stack as tabs (`TileTabs.swift`): one per tile in stack order, the focused one
filled (`headerTabSelected`). Each tab has the tile's kind icon, its title and
the subtitle's last folder ("claude — azulejo"), and its agent's status icon at
its right end, so the strip's own status icon or badge stands down; the
tooltip has the whole title. A click focuses the tile through `tile.focus`, as
alt+h / alt+l do, so the stacked tiles dissolve as they do for the keys. A
right-click opens that tile's menu (Close among it; there's no close button,
as the top tabs have none). A drag along the strip moves the tile in the stack
(`layout.move_order`), Safari-style as the top tabs. The tabs share the width
evenly; below `headerTabMinWidth` each, the strip keeps as many as fit with the
selected one always among them and a "+N" chip at the end, whose menu lists
the rest (no dragging then). One tile in Monocle, or any other layout, shows
the plain title. Into or out of Monocle the title and the tabs cross-fade over
the layout motion (instant with `[animation]` off or Reduce Motion); tiles
leaving the screen keep their title on the way out. A browser tile, which has
no strip of its own, gets one for the tabs above its toolbar.

The strip's middle is a swappable slot (`TileHeaderView.contentKind`: `title`,
`tabs` or `override`): `overrideContent = view` puts any view in it for a while
(laid out in `contentRect`), over the title or the tabs, and `nil` brings back
what was there. The controls on the right and the badge stay.
`TileContainerView.header(of:)` finds a tile's strip.

Automation: `tile tabs click N|drag N M|more N` (mouse events handed to the
tabs, no OS input) and the keys `tile.header.content`, `tile.tabs`,
`tile.tabs.selected`, `.titles`, `.tips`, `.shown`, `.window`, `.overflow`,
`.statuses`, `.even`, `tile.header.opacity` and `tile.header.fading`.
`automation/e2e/mtabs.sh` covers it.

The sidebar shows each workspace's branch from `workspace.git`.
`apps/macos/automation/headers.sh` checks headers, a branch switch in a tile
and the preview toggle across a relaunch, on dev instance `hdr`.

## Window chrome (M2.5 D.2, D.4, D.8)

No visible title bar: the traffic lights sit in the sidebar card's top band
(`TrafficLights` makes the transparent title bar as tall as the toolbar row and
places them), and the toolbar row (52 pt, the lights level with its middle)
holds the tab strip (`TabStrip.swift`: state icon + name per tab, "+"), the
layout picker and the search field (`ToolbarSearch.swift`: opens ⌘K as a
dropdown under itself; its badge shows `quick_open`'s chord). The layout
picker (`LayoutPicker.swift`) sits left of the search field: Manual, Master
(Tile), Three Column and Monocle, the active tab's kind filled. It follows the
tab whatever changes its layout (alt+[ / alt+], ⌘K, herdr) and picking a kind
runs its `toggle_layout_*` action, as the key does (motion and the herdr
mirror alike); tooltips show the chords. It hides first when the window gets
too narrow for it. Empty toolbar space drags the window; a double click there
follows the system's title-bar setting. `apps/macos/automation/toolbar.sh`
checks where clicks land under the title bar, the tab icons, "+", the picker,
the dropdown and the hidden-sidebar row, and takes window captures for
comparing with `docs/design/mockup-v1.png`. `apps/macos/automation/topbar.sh
LABEL` captures the toolbar row in all four themes with the sidebar full, as
a rail and hidden, into `docs/testing/topbar/LABEL/`.

Tabs reorder by dragging, as in Safari: past a 4 pt threshold the pressed
tab lifts (filled, with a shadow) and follows the pointer along the strip
while the others slide aside; letting go sends the core's `tab.move`, so the
order shows in the API and the CLI and is saved. A shorter press is a
click, which selects on mouse up as before. Escape (caught by `KeyRouter`
first), or letting go more than 18 pt above or below the strip, puts it
back. Dragging doesn't select a tab, and the active tab stays active
wherever it goes; ctrl+number follows the new order. The slides follow
`[animation]` and Reduce Motion; an overflowing strip scrolls when the tab
nears a faded edge, and there's no drag when only the selected tab fits.
The herdr mirror sees nothing: tab order is in no mirror target, so a
reorder sends no push. Like workspace order (ADR 0013) it's Azulejo's own,
saved with the session; herdr 0.9.5's API has `tab.move {tab_id,
insert_index}`, but nothing sends it yet.
The strip sits in the title bar's top band, where the window server moves
the window on a drag before the app sees it, everywhere AppKit calls
draggable. Each tab cuts itself out of that region
(`_opaqueRectForWindowMoveWhenInTitlebar`), as do the layout picker's
segments and the search field ("+" and the sidebar toggle are buttons,
which AppKit already cuts out); without that a click on a tab selected it
but a drag moved the window. Empty toolbar space still moves it. `chrome.winmove.*` reads the
region AppKit last computed (`moves` / `stays` per part).
`apps/macos/automation/e2e/tabdrag.sh` drives drags with mouse events made in
the app and handed to the tabs (`tabdrag …` in scripts), and checks the
mirror's log (`AZJ_DEBUG_MIRROR=1`) stays quiet. Handing events straight to
the tabs skips the window server, so the suite also checks that every tab
stays out of the window's drag region (`chrome.winmove.tabs`). Not yet: dragging a tab
onto a workspace in the sidebar, or out into its own window.

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

Folder listings never block the main thread: a home folder with thousands of
entries, a network volume or a folder behind a permission prompt can take
seconds. Files tiles and the Files view list through `Core.executeAsync`
(`azj_core_execute_async`), show "Loading…" until the root's rows land, and
drop a listing for a root that changed since. `apps/macos/automation/files-slow.sh`
holds every listing back 2.5 s and checks that the script keeps running,
the rows arrive and a stale listing is dropped, on dev instance `fslow`.

Collapsed, the sidebar is a rail (`SidebarRail.swift`): each machine group's
letter and dot, then its workspaces' ⌘ numbers and status icons, centred
between equal insets, and `»`. `apps/macos/automation/rail.sh LABEL` captures
it in all four themes into `docs/testing/rail/LABEL/`.

Expanded, the rows show their numbers only while ⌘ is held alone
(`SidebarHints.swift`, as Safari and Arc show tab shortcuts): after 100 ms
(`motion.sidebarHintDelay`, so a chord typed at speed never flashes them)
each row's chord fades in (80 ms) over its branch as a small key badge,
⌘1…⌘8 and ⌘9 on the last row (rows between have none past nine), the same
numbers as the rail and the tooltips. They fade out as fast (both at once
with motion off or Reduce Motion) on release, on any key, when another modifier joins
⌘ (until ⌘ is let go), when the window resigns key or the app goes to the
background. Opening the sidebar with ⌘ kept down (⌘A, then still holding
⌘) shows them too, once its motion ends; with ⌘ up (a click on the rail,
⌘K) it doesn't. Keys reach the terminal untouched. `apps/macos/automation/e2e/wshints.sh`
checks it (`sidebar hints cmd|none|…` in scripts, `sidebar.hints*` keys).

Workspace rows reorder by drag, in the full list and the rail
(`SidebarDrag.swift`): the row lifts, its group's rows slide aside round an
accent slot, and the drop runs `workspace.move` (ADR 0013: Azulejo's order,
not herdr's). Esc or a drop away from the card cancels. It's all in-window,
no drag session or pasteboard. Scripts drive it with `sidebar drag N | to N |
out | drop | click N`, which hand mouse events to the row itself (never the
window or the real pointer); state `drag.*`. `apps/macos/automation/e2e/reorder.sh`
covers it.

## Right-click menus

Workspace rows (Agents view), the top tabs and tile header strips have
standard NSMenus (`ContextMenus.swift`). Tabs: Rename…, Layout ▸ (Manual,
Master (Tile), Three Column, Monocle, the tab's kind ticked), New Tab, Move
Left / Move Right (`tab.move`), Close Tab and Close Other Tabs; they act on
the right-clicked tab without switching to it. `apps/macos/automation/e2e/topbar.sh`
checks this menu and the layout picker. Rows: Rename…, New Tab, Reveal Folder in Finder, Open
Folder in Files Tile, Copy Path, Close Workspace (the folder items only when
the workspace has a repository or cwd; Reveal and Open in Files Tile only on
Local). Headers: Split Right, Split Down, Zoom, Rename…, Copy Title, Copy
Working Directory (terminals), Reveal in Finder, Close Tile, and Kill Pane…
for bound terminals (asks first). The browser has no header strip: these
items follow the page's own in its chrome's menu. Every keymap item runs its
action through `AppController.perform(action:target:)`, aimed at the row's
workspace, the tab or the header's tile, and shows its chord from `commands.list`.
A terminal's body keeps its own right-click (libghostty, mouse-reporting
apps). `apps/macos/automation/menus.sh` builds both menus without a mouse
(`menu workspace N|tab N|tile [invoke TITLE]`, `invoke Layout/Monocle` for a submenu item, `menu answer`, `menu shot`; state
`menu.items`, `menu.copied`, `menu.revealed`, `workspace.names`), checks herdr
keeps a closed workspace's panes, and that a relaunch doesn't bring it back.

## Layout motion (F4, PLAN 2.1)

Layout changes animate: split, close, move, swap, resize, zoom, the tall
actions and the sidebar opening or closing (`toggle_sidebar`: the card slides
and fades while the tile area grows or shrinks with it, a gutter apart), plus
tab/workspace switches. A change that arrives mid-motion starts from where
every tile appears on screen.

`style` picks what content does on the way:

- `reflow` (default): content lays out live, like a window being
  drag-resized. Each moving tile's frame steps along the curve once per
  display frame, and a terminal's grid follows it 100 ms ahead: its PTY
  resizes to the size the tile will have 100 ms on, when that's a whole cell
  more or less and the last resize was 33 ms ago, and takes the exact final
  size once that's within reach, before the motion ends (ADR 0004, Nov 4
  reflow and reflow-lead amendments). The lead covers the redraw's trip
  through libghostty and herdr (herdr's attach client only looks at its size
  every 100 ms), so text lands with the frame (median 18 ms ahead, at worst
  ~55 ms behind) instead of 50–160 ms behind it, and the last redraw lands
  with the motion's end. `apps/macos/automation/reflow-lag.sh` measures it.
  So text rewraps and TUIs redraw as the tile moves, one or two times per
  motion (herdr's 100 ms poll caps it). libghostty's
  renderer thread draws each new size, so the main thread never waits on the
  GPU; until it has, the last frame stays at the top left. A terminal whose
  pane starts mid-motion (a split) is born at its final size. Closing tiles
  and the ones Monocle hides leave as in `crop`. herdr sees no layout change
  from any of this (pane sizes come from the attach client), so the mirror
  stays quiet.
- `crop`: every tile goes to its final frame at the start of a motion and
  Core Animation carries it there visually, so nothing re-lays out per frame.
  A terminal that changes size first covers itself with an IOSurface copy of
  its last frame, resizes its PTY once (ADR 0004; at most one per 50 ms while
  a key repeats), and shows the live surface again on the first fresh frame
  at the new size, or 250 ms after the motion at the latest. The tile's
  frame is a clip. Its bounds and position
  animate; content stays at 1:1 and sticks to the edge that moves least, so
  the moving edge reveals or cuts it off. A shrinking terminal shows its
  snapshot at 1:1 until the motion ends; a growing one shows live content as
  soon as it has drawn at the new size. New tiles sweep in with the divider,
  closing tiles shrink away from the neighbour taking their space, and every
  other change (rotate, layout switches, masters, promote…) moves each tile
  from its old frame to its new one. Going into Monocle the focused tile
  grows over the others as they fade and shrink a little; coming out, they
  grow back in. Only tab switches and focus moves between Monocle's stacked
  tiles fade (ADR 0004, Nov 4 amendment); workspace switches slide (below).
- `scale`: a transform scales the whole tile, so text stretches mid-motion.
- `outline`: tiles jump to their new place hidden, outlines move instead, and
  content fades in over 80 ms at the end.

```toml
[animation]
enabled = true            # false: changes are instant
duration_ms = 200         # 0–1000
curve = "ease_out"        # "ease_out", "ease_in_out", "linear"
style = "reflow"          # "reflow", "crop", "scale", "outline"
tab_switch = "fade"       # tab switches: "fade", "slide" (sideways), "none"
workspace_switch = "slide" # workspace switches (experimental): "slide", "fade", "none"
workspace_switch_ms = 260 # a workspace slide's length, 0–1000 (0: instant); not tied to duration_ms
reduce_motion = "system"  # "system": no motion while macOS Reduce Motion is on; "ignore"
```

**Workspace switches slide (experimental).** `workspace_switch = "slide"`
moves the whole tile area vertically, like a vertical Spaces switch, in the
sidebar's order top to bottom: going down the list (⌘1 → ⌘5, a new
workspace) the old workspace leaves at the top while the new one comes in
from below; going up, the reverse. Closing a workspace slides to the one
that lands on screen. The sidebar, toolbar and window chrome stay still;
the tile area clips. It takes `workspace_switch_ms` (260 ms by default, its
own setting: `duration_ms` doesn't change it; out-of-range values clamp to
0–1000 with a warning, 0 is instant; live on save) on a cubic ease-out (or
the configured curve when it isn't `ease_out`). Nothing
re-lays out or resizes: the old workspace's tile views ride in a sheet a
gutter (8 pt) from the new one's (the seam looks like any gap between
tiles) and one `sublayerTransform` animation moves both, so
focus and typing land in the new workspace at once. A switch mid-slide
carries on from where everything shows (⌘1 ⌘5 ⌘2 never stacks). Off with
motion off or under Reduce Motion; `"fade"` is the tab cross-fade
workspace switches had before, `"none"` is instant. The direction comes from
`SidebarView.ordered`, the order the numbers follow. `apps/macos/automation/e2e/wsslide.sh` checks it
(direction, focus, turn-rounds, rapid switches, close, new, the instant
paths); `anim.sh` measures 45 switches' frame pacing (`ws-stats.json`).

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
1.5 s motions with mid-motion window captures. `AZJ_ANIM_STYLE=reflow|crop|scale|outline`
runs one style regardless of the config. The app can't run
`screencapture` itself, so `capture` lines drop a request the script takes.

## Visible terminal items

In a terminal, press **⌘S** (`select_visible_item`) to label selectable items
across the current window's visible terminal panes. Zoom and Monocle exclude
hidden panes.

Everything that reads the screen lives in the Rust core, `core/crates/hints`
([ADR 0016](../../docs/adr/0016-smart-select-in-the-core.md)), so a Linux shell
gets the same items: detection (`src/detect`), labels, paragraphs, grow and
shrink, arrows and range text (`src/select.rs`) and callout placement
(`src/callout.rs`). The overlay (`TerminalHintOverlay.swift`) reads the rows
and styled snapshot from Ghostty, makes one `azj_hints_detect` call per pane
(JSON in and out, UTF-16 offsets), keeps the returned pane handle for the
selection keys (`azj_hints_pane_*`) and draws. `TerminalHints.swift` is the
glue. Whether a path exists goes through the crate's `Filesystem` trait
(`std::fs` in the app, a fake in tests); `/home` and `/net` are never
checked, and each pass checks a path once. A full 60×120 screen with
filesystem checks takes about 0.8 ms, JSON both ways included.

Labels are Homerow-style callouts: a small rounded pill with
bold dark letters on yellow, a thin outline and a soft shadow (`hintCallout`,
`hintCalloutBorder`, `hintCalloutText`, `hintCalloutShadow`), with a short
tail pointing at the item's first cell. Placement
(`azj_hints_place_callouts`, `callout::place`) runs per pane in reading order
and is deterministic:

- A pill sits below the first cell (over the row underneath, tail up), above
  it (tail down), or before it on its own row, in the blank margin left of an
  indented item (tail pointing right). It may hang into the pane's padding.
- It tries those sides in that order (above first on the last row), keeping
  to blank space first: no text at all, then no item text (it may cover plain
  output), then nothing but a paragraph's text. So a callout covers another
  item's text only when no blank or plain-text spot exists on any side.
- Below or above, a pill slides sideways only as far as keeps its tail's foot
  within half a cell of the first cell's middle: tails stay nearly upright,
  never a long slant across a row. Too far: another side instead.
- Never over another pill or another item's first cell. When no side works (a
  column of paths at the pane's left edge, one per row), the pill sits over
  its own item's first cells, no tail, before it would cover anyone else's.
- With nowhere free at all it sits centred below and counts as a clash
  (`hints.callout.flagged`).

**Labels say what the item is.** A label's first letter is its kind: `l` link
(URL), `p` path, `c` command, `t` text paragraph, `i` IP address, `d` domain,
`h` hash, `u` UUID, `b` branch. When only one item of a kind is visible, that
letter alone selects it; otherwise the letter is followed by home-row letters
(`pa`, `ps`, `pd`…; three letters past 26 of a kind). Labels of one kind share
a length, so none is a prefix of another. Labels are numbered pane by pane in
reading order.

**Repeats get one label.** When a pane shows the same item several times
(same kind and value: a prompt's `~/proj` on every prompt line), only the
newest copy, the lowest on screen, gets a label
(`azj_hints_labels`, `select::older_repeats`). The older copies stay unlabelled and
unhighlighted while choosing; they light up while their labelled copy is
selected, and the arrows can still move to them.

**Typing filters.** After each letter, items whose labels no longer match fade
out (`motion.hintFilter`, 80 ms; instant with `[animation]` off or Reduce
Motion). The callouts that still match show the letters typed so far greyed
(`hintCalloutTyped`, a lighter weight) and the rest bold: after `c`, `CD`
shows a dim `C` and a bold `D`. Letters that match nothing are ignored.
Backspace takes the last letter back; Esc (or a click outside) leaves the
mode. A complete label selects its item.

**Arrows pick without typing the whole label.** While choosing, with or
without letters typed, the arrow keys move a focus ring across the items still
shown: ⌘S, `c`, then ↑ a few times walks the commands only. The focused item
gets a thicker ring just outside its highlight and its callout turns a deeper
amber with a ring of its own (`hintFocus`, `hintCalloutFocus`), so it never
looks selected. Return selects it, exactly as if its label had been typed;
with nothing focused, Return does nothing.

- The first arrow starts at the focused tile's terminal cursor (libghostty's
  IME point), usually where the latest output ends: ↑ the closest item on the
  rows above it, ↓ the first one below it or else the last one, ← the last one
  before it, → the first one after it or else the last one.
- After that, ↑ ↓ go to the nearest shown item on the rows above or below,
  closest in column; ← → to the previous or next shown item in reading order.
  ← → past a pane's first or last shown item go on to the previous or next
  pane's; ↑ ↓ stay in the pane.
- Typing more letters still filters. If the focused item drops out, the ring
  moves to the nearest item left in its pane (fewest rows apart, then
  columns), or goes away when there's none. A complete label still selects its
  item at once, whatever is focused.
- ⇧ + arrow starts a range from the focused item: it selects it and extends,
  as ⇧ + arrow does once selected. ⌥↑ / ⌥↓ do nothing while choosing.
- Backspace from a selection goes back to choosing with the ring on that item.

The core does the moving (`select::focus_step`, `select::nearest`, through
`azj_hints_pane_focus` and `azj_hints_pane_nearest`): the neighbour logic the
selection's arrows use, restricted to the shown items.

**Text paragraphs (`t`)** are runs of non-blank rows that read as prose: a row
of three or more words, at least half of them plain words, with no column
gaps, plus the rows below it at the same indent while they read as prose too
or the row above fills most of the pane (a wrapped sentence's tail). A list
marker (`- `, `* `, `1. `) starts a new paragraph, and a "Label: value" row
(`Session UUID: <uuid>`) isn't prose. Rows a command covers (half
the row, or from its start) are never prose, nor is anything below a command in
the same block (its output). Paragraphs are drawn as a dashed outline round
their rows. The heuristic lives in `select::paragraphs`.

**Once an item is selected**, the keys act on the selection (letters no longer
type a label):

| Key | Does |
| --- | --- |
| `c` (or ⌘C) | Copy |
| `o` | Open (URLs and existing local paths) |
| `r` | Run (commands): in this pane if it's an idle shell, else the nearest idle shell, else a new pane |
| ← → | Previous / next item in reading order (a command comes before the paths inside it) |
| ↑ ↓ | The nearest item on the row above / below, closest to the selection's first column |
| ⇧ + arrow | Extend the selection from its anchor |
| ⇧ + a label | Extend the selection from the anchor to that item (same pane) |
| ⌥↑ | Grow: the smallest item around the selection (a path → its command), then the rows it's on, its paragraph, its block of non-blank rows, the whole screen; steps that add nothing are skipped |
| ⌥↓ | Shrink back the way ⌥↑ came; with nothing to go back to, the first item inside (a line that is exactly one item becomes that item) |
| Backspace | Back to choosing, one letter short of the anchor's label (or the last ⇧ letter) |
| Esc | Leave the mode |

Arrows move at the selection's level: between items, between paragraphs,
between lines (non-blank rows) or between blocks, whatever ⌥↑ last grew to.
Moving starts a new ⌥↓ history. Once an item is selected the other labels
hide; arrows and ⌥↑ / ⌥↓ need none. Holding ⇧ shows the ones outside the
selection, for ⇧ + label, after a moment (`motion.hintExtendDelay`, 100 ms, so
⇧ + arrow doesn't flash them), with a quick fade (`motion.hintExtend`, 80 ms);
letting ⇧ go without a label fades them out again. The first ⇧ letter keeps
the matching ones, that letter greyed as when choosing, and hides the rest.

Copying one item copies its value (a command without continuation backslashes,
a quoted path without its quotes). Copying a range, line, block, paragraph or
screen copies the text as a terminal selection would: rows joined with
newlines (soft-wrapped rows without one), trailing blanks dropped, indents of
later rows kept. Open and Run only apply to a single item.

A small popup beside the selection has the Copy / Open / Run buttons for the
mouse, staying inside its terminal pane. Each icon has its shortcut keycap;
hovering shows its action and keystroke. It shows only the actions that apply,
never a greyed-out one: Open for a single URL or existing local path, Run for a
command with a known working directory.

The **legend** lists the keys that apply right now: the kind letters present
while choosing, then ↑↓←→ move and, once something is focused, ⏎ select; and
copy, open, run, move, extend, grow/shrink while selected
(only the ones that apply). It lives in the focused tile's header strip,
centred in place of the title, or of the tabs in Monocle
(`TileHeaderView.overrideContent`), clear of the agent badge, so it never
covers terminal text and never moves during a session. It fades in, and on
Esc fades out over the title or tabs coming back (`motion.hintFilter`). Keys are small rounded keycaps (`hintBadge`,
`hintBadgeBorder`, `hintBadgeText`, `font.hintKey`) with a quiet word beside
each (`textMuted`, `font.hintLegend`); a kind letter is a little yellow callout,
like the labels it picks. In a narrow tile it drops items from the end, keeping
Esc. Only a tile without a header strip gets a bar at the bottom of the pane
instead.

URLs, absolute and relative paths, existing local filenames, the workspace's
current branch, plain `git branch` listings, shell commands, IPv4 and IPv6
addresses (with an optional `:port`), bare domains, UUIDs and git hashes
receive hints.
Paths support quotes, escaped spaces, `:line:column` suffixes,
surrounding parentheses/brackets, and output prefixes such as `key=/path`.
An existing name without a `/` or a file extension (`docs`) only counts inside
a command, so prose words that happen to name a directory stay unmarked.
Domains need a known top-level domain from a short built-in list that leaves
out TLDs doubling as file extensions (`.md`, `.sh`, `.py`, `.rs`, `.app`, ...),
so `README.md`, `file.txt`, `2026.11` and `e.g.` aren't hosts. Git hashes are
7-40 lowercase hex characters with at least one letter and one digit, and never
part of a longer token or a UUID. Trailing sentence punctuation and surrounding
brackets stay outside every item. When items overlap, the longest wins
(`198.51.100.12:8443` over `198.51.100.12`, a URL over its host); a command
keeps its own hint next to the items inside it.
Open resolves paths against the source terminal's working directory and uses
the default macOS application. Missing paths, remote paths, branches,
addresses, domains, UUIDs and hashes remain copy-only.
Run applies only to commands with a known working directory. It picks where
to run (`azj_hints_run_plan`, `run::plan` in the crate; `TerminalRun.swift`
gathers the panes and carries it out):

1. **The current pane** (the focused tile), if it's an idle shell.
2. **The nearest idle shell** in the tab: the smallest gap between its rect
   and the current tile's in the layout tree (neighbours touch, so they tie;
   Monocle uses the Tile arrangement it hides), ties to the shell used as a
   Run target most recently, then the closest centre, then reading order.
3. **A new pane** beside the source, in the source's directory, leaving the
   original pane untouched, as before.

Only panes on the source pane's machine count: a command from a remote pane
never runs in a local shell, or the other way round. Idle means the core's
polled foreground process (`pane.process_info`, every second and at once
when herdr reports a pane change; `PaneMeta.shell`/`command`) is the shell
itself: `TileModel.idleShell`, the mouse-mode `shellInForeground` test made
strict. A pane not polled yet, ended, running an agent that's working or
blocked, or with any foreground command line beyond a bare shell name
(`vim`, `claude`, `sleep 30`, `bash script.sh`) is busy. Deciding costs no
herdr round trip: it reads the session the shell already has. A command
started less than a poll ago can still look idle; most shells retitle
themselves when a command starts, which polls at once (about 150 ms).

In an existing shell Run first clears whatever is half typed: ⌃E then ⌃U
(`terminal.send_keys`), end of line and delete back to its start in bash
(readline), zsh's emacs keymap and fish; in zsh's vi insert keymap ⌃E types
a character and ⌃U deletes back from the cursor, so text right of a moved
cursor survives there. Then it types the line (`terminal.send_text`) and
Return, in order, without blocking the main thread. When the shell sits in
another directory, the line starts with `cd '<source dir>' &&` (quoted the
way POSIX shells and fish both read; a directory with a backslash or a
control character falls back to a new pane), so the shell stays there
afterwards. The pane Run used takes focus, as a new pane does (in Monocle it
comes to the front), and the mode closes. `[hints] run_target = "new"`
always makes a new pane, as Run did before; the default is `"reuse"`.
Automation: `hints.run.target` (`current`, `reuse:<N>` for the Nth tile in
reading order, `new`) and `hints.run.idle` (`idle` / `busy` per tile).

A new shell starts with its normal environment, not the source shell's
unexported variables or temporary state. A single-line command is typed into
the shell as it is. A multi-line one (a heredoc, or a `\` continuation from a
coding agent's tool call) runs through `/bin/sh` instead
(`azj_hints_run_line`): one line,
`/bin/sh -c 'eval "$(printf %s <base64> | base64 -d)"'`, that POSIX shells and
fish parse alike, since typing it line by line breaks in fish (no `<<`) and
typed tabs trigger completion. `hints.sh` runs these with fish (without your
config or history) as the login shell when fish is installed, and checks Run
into an idle fish, zsh and bash with half-typed input.

Recognition uses visible text and terminal foreground colors, not a full shell
parser. A prompt marker followed by a space (`$`, `%`, `❯`, `➜`, or `#` and `>`
attached to a prompt such as `root@host:/etc#` or `~/proj>`) starts a command
that needs no known executable name. The text before the marker must look like
a prompt: a user@host, a path, a bracketed segment (`(venv)`, `[user@host
dir]`, `(main)`), a shell version (`bash-5.2`), a lone host or folder (`host% `)
or glyphs (starship, powerlevel10k, Kali's `└─$`). oh-my-zsh's `➜  dir
git:(main) ✗` segments, cmd.exe's `C:\dir>` and PowerShell's `PS C:\dir>` are
skipped too, and a right-hand prompt (time, duration) after a wide gap is cut
off. `>>>`, `irb>`, `postgres=#`, `mysql>` and numbered menus (`❯ 1. Yes`) are
not shells. A coding-agent `!` marker with optional space (`!cargo test`,
`! cargo test`, or `> !cargo test` as Claude Code echoes it) works the same way;
the marker is excluded from copying and execution.

Coding-agent screens get their own rules. Claude Code's `● Bash(…)` tool call is
one command, without the wrapper, across every row it takes up to its closing
parenthesis (quotes, heredocs and `$(…)` inside it included); a call cut short
with `…` is not offered. The command under a permission prompt's "Bash command"
title is one too, without the description line below it. Borders of boxes
(`│ … │`) are ignored, `⎿` output rows are never commands, and a table cell is
read on its own. Codex's `• Ran …` lines count as well. The command Claude
Code shows while or after running it (`⎿  $ cmd`, the elbow sometimes drawn as
`└`) is one command across its indented rows: a word the agent broke at the
edge rejoins without a space, and the elapsed time (`(4m 29s)`) and the
`(ctrl+b to run in background)` hint stay out. Neither a tool call cut short
with `…` nor such a row ever yields a command from its words ("gh run watch"
has no cue). In an agent's transcript `❯` starts the user's own messages
(`❯ merged`): only a line that reads as a command on its own counts there,
never a single word. oh-my-zsh's folder is skipped even when it's named like a
command (`➜  dash git:(main) npm run build`).

Unprompted commands use a built-in vocabulary of common Unix, Linux, macOS,
Windows, and PowerShell command names (developer tools included), independent
of the local `PATH`. They start a standalone or list-item line, a
backtick-delimited code span, a foreground-color run, a line in a shell code
fence (```` ```sh ````; `console` fences need prompts), or follow a cue in a
sentence ("open a tunnel with ssh -L …", "run it with npx vitest run upload",
"Clone the repo: git clone …"); the command then ends at a function word or
the sentence's punctuation. Names that are also ordinary words (`find`, `test`,
`kill`, `open`, `export`…) need the shape of a command on a line of their own:
a flag, a path, an operator or an assignment, and no English function words.
Windows names match in lower or upper case or as Verb-Noun cmdlets, never
capitalised like a sentence ("Copy the file"). A line is not a command when it
reads as prose (function words such as "the", "you", "to", or a final word with
a full stop), is tabular output (gaps of three spaces), a log line (`npm WARN`,
`test result: ok`), a usage synopsis (`[-v]`, `<command>`), a diff line, a
compiler location (`src/main.rs:4:5:`) or a single word; a lone tool name in
backticks counts only after a cue ("run `make`"). For example, an inline colored
`sudo bash ./setup.sh` gets a full-command hint as well as a separate path hint;
adjacent prose stays outside the command. On a row with colors, a whole line
counts only when its first word is a distinctive command name or carries a
color of its own (syntax highlighting). Palette and RGB colors work without
assuming a particular theme. Color alone does not make arbitrary text runnable.
Recognition is a heuristic, not an execution safety check or proof that a
command is installed; Run passes the displayed command to the source machine's
shell without OS translation. `core/crates/hints/data/hints-corpus.txt` holds
fictional screens (shells, coding agents, prose, output) with the commands each
should yield; `cargo test -p azulejo-hints --test corpus` scores recall and
precision on it (99.4% and 100%, one known miss).

Full-width token continuations, including colored commands, can span rows.
Commands also continue across rows an application wrapped itself, such as a
coding agent's indented answer: a command carries on while a row ends with
`\`, `|`, `&&` or `||`, a quote, heredoc, `$(…)` or `do`/`then` block opens
that a later row closes (blank rows inside included, except at a prompt), or
the next row has the same non-zero indent and its first word wouldn't have
fitted on this row (within two cells of the pane's right edge, or a box's
border, since wrappers often keep a margin). Shell secondary prompts (`> `,
`pipe> `, `dquote> `) are skipped. URLs and paths cut at the right edge of such
a block rejoin the same way. A multi-row item gets one label and a highlight on
every row. Copy and Run use the command as it would run: wrapped rows joined
with single spaces; real line breaks (heredoc bodies, multi-line strings, rows
of a tool call or code block, rows after a trailing `\`) kept, minus the
block's own indent; at a shell prompt, `\` continuations and unechoed quote
continuations joined with single spaces as the shell would see them.
Terminal frames freeze while choosing, so live output cannot retarget a label.
Scroll, resize, tab/focus changes, and source-directory or binding changes
cancel the mode. `just ghostty` applies the styled-text bridge patch on both
Xcode and runtime-Metal builds; rebuild GhosttyKit after changing that patch.

A second pass (`[hints] row_classifier`, on by default; `false` turns it
off) labels every visible row with a small model in the Rust core
(`core/crates/hints`): prompt plus command, continuation, code, output, prose,
chrome or blank. Commands shown in output count as code (make's echoed
lines, `RUN` steps, scripts and workflows printed by `cat`). Where the model
is sure a row is output or chrome, it drops a command that fills the row
(`jq 1.7 -> 1.7.1` from an upgrade list, a diff's context line); commands in
backticks or after a prompt, `RUN` or a cue stay. It may add a command the
detector missed on a command or code row, if it starts with a known command
and reads as one. It runs inside `azj_hints_detect`, no second call. URLs, paths and the other items never change. The model is
a linear-chain perceptron over hashed row features, trained on synthetic
screens only (`just hints-model`), and labels and refines a 50-row screen in
about 0.1 ms. On every scored set it matches or beats the detector alone:
identical on `hints-corpus.txt`, higher command precision with the same
recall on hand-written screens it never saw.

Rebind or disable `select_visible_item` under `[keys.macos.terminal]`. Other
tile kinds and text fields retain their save shortcuts. The command also
appears in quick open. `apps/macos/automation/hints.sh` runs the core's checks
(`cargo test -p azulejo-hints`: recognition boundaries, the corpus, kind-letter
labels, paragraphs, grow/shrink, arrow neighbours, callouts) and drives the GUI
in an isolated dev session, in a dark and a light theme. Copy uses the private
automation pasteboard; Open records its dispatch target without launching an
external application; Run executes a fixture command in a real new pane.

### Mouse in terminal tiles

herdr's attach client switches on mouse reporting for every pane and never
says whether the program in the pane wants the mouse (herdr forwards a click
only if it does). So Azulejo goes by what runs in the pane's foreground, which
the core polls from herdr (`pane.command`):

- A shell (bash, zsh, fish, sh, dash, ksh, tcsh, nu…), or nothing polled yet:
  selection works as in Ghostty. Drag, double-click a word, triple-click a
  line, shift-click to extend, option-drag for a rectangle; ⌘C copies. The
  surface turns libghostty's reporting off for each left-button gesture and
  back on at the release.
- Anything else (an agent's TUI, vim, htop, less): plain clicks and drags go
  to herdr and on to the program, as before. Shift-drag, shift-double-click
  and shift-triple-click select locally (Ghostty's convention under mouse
  capture).

The wheel, right and middle buttons always go to herdr, which scrolls its
history.

Hold ⌘ over a link (a URL or an OSC 8 hyperlink) to underline it, then ⌘-click
to open it. Web (`http`, `https`) and `mailto` links open in their default app;
Azulejo refuses other schemes and files, since terminal output picks the
target. `apps/macos/automation/mouse.sh` drives all of this, in a shell and over a
mouse-aware fixture program (`mouse-app.py`), with in-app mouse
events and checks the selected text, the private pasteboard and the link it
would open (automation records the link instead of opening it).

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
relaunch, and herdr keeps everything); closing any other window closes it in
the core, which closes its workspaces in herdr too, like ⌥⌘W (ADR 0014).

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

