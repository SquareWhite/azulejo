# macOS shell: the interface

How the window's parts work, and the scripts and automation hooks that check
them. Scripts live in `apps/macos/automation/`.

## Tile headers

Every tile has a 28 pt header strip (`TileFrame`): kind icon, title,
subtitle, then any controls. The text is the core's `tile.header`, so every
shell shows the same. A tile view adds controls by conforming to
`TileHeaderAccessory`.

The strip's middle is a swappable slot (`TileHeaderView.contentKind`:
`title`, `tabs` or `override`). Setting `overrideContent` puts any view there
until it's set back to `nil`; smart select's legend uses it.
`TileContainerView.header(of:)` finds a tile's strip. Checked by
`headers.sh`.

### Monocle tabs

In a Monocle tab with more than one tile, the strip shows the stack as tabs
(`TileTabs.swift`), the focused one filled. Each shows the tile's icon,
title, last folder and agent status. A click focuses the tile, a right-click
opens its menu, and a drag reorders the stack (`layout.move_order`). Below
`headerTabMinWidth` each, the strip keeps the tabs that fit, always including
the selected one, plus a "+N" menu for the rest. A browser tile, which has no
strip, gets one above its toolbar for the tabs.

Scripts use `tile tabs click N|drag N M|more N` and the `tile.tabs*` and
`tile.header.*` keys. Checked by `e2e/mtabs.sh`.

## Window chrome and toolbar

There's no visible title bar. The traffic lights sit in the sidebar card's
top band (`TrafficLights`), and the 52 pt toolbar row holds the tab strip
(`TabStrip.swift`), the layout picker (`LayoutPicker.swift`) and the search
field (`ToolbarSearch.swift`, which opens ⌘K as a dropdown). The picker
follows the active tab's layout and runs the same `toggle_layout_*` action as
the keys; it hides first when the window gets narrow. Empty toolbar space
drags the window.

Tabs reorder by dragging: past a 4 pt threshold the tab lifts and follows
the pointer, and letting go sends the core's `tab.move`. Escape, or letting
go well above or below the strip, cancels. Tab order is Azulejo's own, saved
with the session and never sent to herdr.

The strip sits where the window server moves the window on a drag before the
app sees it, so each tab, picker segment and the search field cut themselves
out of that region (`_opaqueRectForWindowMoveWhenInTitlebar`).
`chrome.winmove.*` reads the region AppKit last computed.

Checked by `toolbar.sh` (where clicks land), `e2e/topbar.sh` (the picker and
the tab menu) and `e2e/tabdrag.sh` (`tabdrag …` hands mouse events straight
to the tabs; the suite also checks the drag region and that the mirror log
stays quiet).

## Sidebar

`Sidebar.swift` has an Agents | Files switch and a footer counting
workspaces by state.

- **Agents**: one group per herdr machine (with a connection dot), workspace
  rows with a status icon, name and git branch.
- **Files**: the files tile's tree on the focused workspace's repository or
  cwd. Return opens a file beside the focused tile; Esc gives the keyboard
  back. Refreshes don't steal the keyboard (`KeepsKeyboard`).

Each view has its own key: ⌘A (`toggle_agents_sidebar`, in terminals and the
files list) and ⌘F (`toggle_files_sidebar`, which also gives the tree the
keyboard). Collapsed, the key opens the sidebar on its view; open on the
other view, it switches; open on its own view, it collapses. The toolbar
button keeps `toggle_sidebar`, which leaves the view alone. Find is ⇧⌘F.

Drag the card's inner edge to resize it (`SidebarResize.swift`): 180 to
520 pt, never leaving the tiles less than 200 pt. The tile area follows live
and terminals reflow by whole cells, as in a divider drag. The width is saved
on release, one for every window, and kept across launches (user defaults,
`dev.`-prefixed in the dev profile). A double-click on the edge puts the
default 230 pt back. Scripts use `sidebar width drag W` and `sidebar width
double`; checked by `e2e/sbwidth.sh`.

Folder listings never block the main thread: files tiles and the Files view
list through `Core.executeAsync`, show "Loading…" until rows land, and drop
listings for a root that has since changed. `files-slow.sh` checks this with
every listing held back 2.5 s.

Collapsed, the sidebar is a rail (`SidebarRail.swift`) of machine letters and
workspace numbers. In the full sidebar, holding ⌘ alone shows each row's ⌘
number after 100 ms (`SidebarHints.swift`), so a chord typed at speed never
flashes them; checked by `e2e/wshints.sh`.

Workspace rows reorder by drag in the list and the rail (`SidebarDrag.swift`),
running `workspace.move`. Workspace order is Azulejo's, not herdr's. It all
happens in the window, with no drag session or pasteboard. Scripts use
`sidebar drag …`; checked by `e2e/reorder.sh`. `sidebar.sh` checks the rest
(`sidebar …` verbs, `sidebar.*` keys).

## Right-click menus

Workspace rows, top tabs and tile headers have standard menus
(`ContextMenus.swift`). Keymap items run through
`AppController.perform(action:target:)` aimed at the clicked workspace, tab
or tile, without switching to it, and show their chords. A terminal's body
keeps its own right-click.

- **Tabs**: Rename…, Layout ▸, New Tab, Move Left / Right, Close Tab, Close
  Other Tabs.
- **Workspace rows**: Rename…, New Tab, Mark All as Read, folder items
  (Reveal, Open in Files Tile, Copy Path), Close Workspace.
- **Tile headers**: Split Right / Down, Zoom, Rename…, Mark as Unread, Copy
  Title, Copy Working Directory, Reveal in Finder, Close Tile, Kill Pane….

Close Tile detaches: herdr keeps the pane. Close Workspace closes the herdr
workspace too, so its processes end, and asks first when something runs
there.

`menus.sh` builds the menus without a mouse (`menu workspace N|tab N|tile
[invoke TITLE]`, `menu answer`, `menu shot`; `menu.*` keys) and checks herdr
afterwards: Close Workspace removes exactly that workspace's panes, Close
Tile leaves its pane running, and a relaunch doesn't bring a closed workspace
back.

## Mouse in terminal tiles

herdr's attach client turns mouse reporting on for every pane and never says
whether the program inside wants the mouse. So Azulejo goes by the pane's
foreground process, which the core polls from herdr:

- **A shell**, or nothing polled yet: selection works as in Ghostty (drag,
  double- and triple-click, shift-click, option-drag); ⌘C copies.
- **A program on `[terminal] select_on_drag`** (`["omp"]` by default): a
  plain drag selects, as in a shell, and the program never sees it. Clicks
  (double-clicks too) and the wheel still reach the program, so its own
  click and scroll handling keeps working. The press waits until the
  pointer moves a cell's width or height (a drag) or the button comes up
  (a click: the program gets the press and release together). Shift + drag
  and shift + click select, as below. Names match the foreground program as
  the tile header shows it; an empty list turns this off. Saving the config
  applies it.
- **Anything else** (an agent's TUI, vim, less): clicks and drags go to the
  program. Shift + drag or click selects locally, as in Ghostty under mouse
  capture.

The wheel and the right and middle buttons always go to herdr. ⌘-click opens
a link (URL or OSC 8) when it's `http`, `https` or `mailto`; other schemes
and files are refused, since terminal output picks the target.

`mouse.sh` checks this with in-app mouse events in a shell and over a
mouse-aware fixture (`mouse-app.py`), reading the private pasteboard and
recording links instead of opening them. A small native stand-in
(`mouse-standin.c`, run as `omp` and under another name) turns on omp's
mouse modes and logs what reaches it.
