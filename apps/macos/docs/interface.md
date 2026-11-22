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
until it's set back to `nil`.
`TileContainerView.header(of:)` finds a tile's strip. Checked by
`headers.sh`.

### Monocle tabs

In a Monocle tab with more than one tile, the strip shows the stack as tabs
(`TileTabs.swift`), the focused one filled. Each shows the tile's icon,
title, last folder and agent status. A click focuses the tile, a right-click
opens its menu, and a drag reorders the stack (`layout.move_order`). Below
`headerTabMinWidth` each, the strip keeps the tabs that fit, always including
the selected one, plus a "+N" menu for the rest.

Scripts use `tile tabs click N|drag N M|more N` and the `tile.tabs*` and
`tile.header.*` keys. Checked by `e2e/mtabs.sh`.

## Window chrome and toolbar

There's no visible title bar. The traffic lights sit in the sidebar card's
top band (`TrafficLights`), and the 52 pt toolbar row holds the tab strip
(`TabStrip.swift`), the layout picker (`LayoutPicker.swift`) and the ⌘
button (`QuickOpenButton` in `ToolbarButtons.swift`), which opens ⌘K. The
picker follows the active tab's layout and runs the same `toggle_layout_*`
action as the keys; it hides first when the window gets narrow, the button
never does. Empty toolbar space drags the window.

Tabs reorder by dragging: past a 4 pt threshold the tab lifts and follows
the pointer, and letting go sends the core's `tab.move`. Escape, or letting
go well above or below the strip, cancels. Tab order is Azulejo's own, saved
with the session and never sent to herdr.

The strip sits where the window server moves the window on a drag before the
app sees it, so each tab, picker segment and the ⌘ button cut themselves
out of that region (`_opaqueRectForWindowMoveWhenInTitlebar`).
`chrome.winmove.*` reads the region AppKit last computed.

Checked by `toolbar.sh` (where clicks land), `e2e/topbar.sh` (the picker and
the tab menu) and `e2e/tabdrag.sh` (`tabdrag …` hands mouse events straight
to the tabs; the suite also checks the drag region and that the mirror log
stays quiet).

## Sidebar

`Sidebar.swift` has a Workspaces | Files switch and a footer counting
workspaces by state in words ("3 waiting  1 working  4 idle", zero counts
left out). The view was called Agents; its key keeps the old id
(`toggle_agents_sidebar`) and `sidebar mode agents` still works in scripts.

- **Workspaces**: one group per herdr machine. Its header: a machine glyph
  (a laptop for Local), the name in bold, a connection dot, the workspace
  count and a "+" for a new workspace there, on connected machines only;
  otherwise the state in words ("Reconnecting" with a turning arc, "Can't
  connect", "Off"), folded to its icon when the card is narrower than
  `space.sidebarCompactBelow` (200 pt). A hairline parts the groups; a
  connected machine with nothing on it says "No workspaces · New…", and a
  window with no workspaces at all shows a centred hint instead of the
  list. Workspace rows (`WorkspaceRow`) are two lines, 40 pt: the status
  icon (filled when it needs you: waiting, done; a ring on a track while
  working; a thin grey ring as wide as the others when idle; dotted while
  herdr starts it), the name and the time in that state (`agent.since_ms`),
  then the branch in mono, cut in the middle so a `feature/` prefix and the
  tail stay (`SidebarText.middle`), a dot when the tree is dirty; a
  detached HEAD shows its short hash and "detached", a new workspace
  "Starting…". Without a
  branch the row is one 30 pt line. Several agents in the winning state show
  their count on a chip. A row shows no hover card (the row already says
  it all); one whose name or branch is cut short gets a plain tooltip with
  both in full. Only the list scrolls (the switch and the
  footer stay): its edges fade where rows go on beyond them
  (`space.sidebarEdgeFade`), a pill at the bottom counts the rows below and
  the waiting ones among them ("↓ 9 more · 1 waiting"), one at the top the
  waiting rows above; a click scrolls to the nearest waiting row. The active
  row scrolls into view when the workspace changes.
- **Files**: a file tree (`FilesPanelView`, over the core's `files.panel`)
  on the focused workspace's repository or cwd, on its machine for a remote
  workspace, under a header with the folder's name, how many files changed
  and its branch, and with indent guides down each open folder. Return or a double-click opens a file in the editor: `[editor]
  command` (`vim` by default) in a new terminal split beside the focused tile
  (the core's `files.open`), on the file's machine. With `[editor] app` set,
  Local files open in that app instead; when it can't be found a HUD line
  says so and the command opens. Remote files open in the app only when it's
  Zed (bundle ID `dev.zed.Zed*`, or an app named Zed): `zed://ssh/…` from the
  core's `files.ssh_url`, the file alone. If Zed's SSH can't reach the
  machine as Azulejo does (a `[herdr] ssh` wrapper), a HUD line says why and
  the command opens on the machine; other apps always use the command.
  Return and a double-click on a folder open or close it. Open Folder in
  Editor (⌘⇧O, `files_open_folder_in_editor`; in the menu on a folder, and
  on empty space for the root) opens a folder the same way: the app gets
  the folder (Zed opens it as a project, a remote one as `zed://ssh/…`),
  with the same HUD fallbacks; the command runs as `command <folder>` in a
  new pane to the right, in that folder (`files.open`, target
  `editor_folder`). With a file selected, the key opens the file's folder.
  Esc gives the keyboard back. Refreshes don't steal the keyboard
  (`KeepsKeyboard`).

  The tree selects several rows (⌘-click, ⇧-click) and changes files
  (`FilesOperations.swift`), all through the core and off the main thread,
  with the outcome as a HUD line:

  - **New File (⌘N) / New Folder (⌘⇧N)**: a name field in the target folder
    (a folder itself, a file's folder, the root on empty space), made by
    `files.create`, which never replaces anything; a taken or invalid name
    keeps the field open. Return makes it, Esc doesn't, and the new entry
    is selected.
  - **Move to Trash (⌘⌫)**: Local entries go to the Trash (`files.delete`;
    Put Back works). A remote machine's are removed for good, after a bar
    in the panel asks (Return or Delete confirms, Esc or Cancel doesn't).
  - **Copy (⌘C) / Paste (⌘V)**: the entries go on the pasteboard as files,
    a remote machine's downloaded into the app's temp folder first
    (`files.transfer`); pasting copies files on the pasteboard into the
    target folder, uploading them for a remote workspace. Copy Path stays
    text.
  - **Drops** from Finder land in the folder under the pointer (the root on
    empty space), Finder's way: the same volume moves, another copies, ⌥
    copies, ⌘ moves; a remote workspace always uploads. Promised files are
    received into the folder. Moving entries between folders by dragging
    them inside the tree works the same way (Local only).
  - **Drags out** carry Local entries as file URLs (a terminal shows its
    drop zones) and a remote machine's as file promises that download where
    they're dropped, plus their remote paths, which a terminal on that
    machine types.

  The keys are the keymap's `files` context (`files_new_file`,
  `files_new_folder`, `files_delete`, `files_copy`, `files_paste`); the
  right-click menu shows them. Remote panels list a folder again as soon
  as the core changes it. Checked by `e2e/fops.sh` (Local, deletes into a
  temp folder through `AZJ_TRASH_DIR`) and `e2e/rfops.sh` (through the SSH
  stand-in).

Each view has its own key: ⌘A (`toggle_agents_sidebar`, in terminals and the
files list) and ⌘F (`toggle_files_sidebar`, which also gives the tree the
keyboard). Collapsed, the key opens the sidebar on its view; open on the
other view, it switches; open on its own view, it collapses. The toolbar
button keeps `toggle_sidebar`, which leaves the view alone. Find is ⇧⌘F.

Drag the card's inner edge to resize it (`SidebarResize.swift`): 180 to
520 pt, never leaving the tiles less than 200 pt. The tile area follows live
and terminals reflow by whole cells, as in a divider drag. The width is saved
on release, one for every window, and kept across launches (`SettingsStore`:
the user defaults, or a dev instance's own `state/settings.plist`). A double-click on the edge puts the
default 230 pt back. Scripts use `sidebar width drag W` and `sidebar width
double`; checked by `e2e/sbwidth.sh`.

Folder listings never block the main thread: the Files view lists through `Core.executeAsync`, show "Loading…" until rows land, and drop
listings for a root that has since changed. `files-slow.sh` checks this with
every listing held back 2.5 s.

Collapsed, the sidebar is a rail (`SidebarRail.swift`) of machine letters and
workspaces, 28 pt rows with the status icon alone (the active one on the
selected fill; idle, as in the full sidebar, a thin grey ring as wide as the
other icons), workspaces only (no Workspaces | Files switch). Its footer
counts the waiting workspaces (its hover card counts the rest) above `»`.
Resting the pointer on a row or a letter for 400 ms
(`motion.hoverCardDelay`) shows its hover card (`HoverCard`, in
`SidebarParts.swift`) to the rail's right: a row's name and ⌘ key,
branch (its dot and "Uncommitted changes" under it when dirty), state with
time and agent, and path; a letter's machine, its state and herdr's
message. Each value keeps to one line: the card is as wide as its longest
line (`space.hoverCardMinWidth` to `hoverCardMaxWidth`), and past that a
name is cut at its end, a branch in the middle (its `feature/` prefix
kept) and a path in the middle, its last folders whole; only a message or
the letter's workspace list wraps, between words. The cards, the overflow pills and the rail's
markers float as ⌘K's menu does: its `palette` fill, `border` hairline and
shadow, its corner radius and keycaps. The list scrolls between the top and
the footer, with the same edge fades; "↑2" / "↓1" markers count the waiting
rows out of view (a click scrolls to the nearest), and the active row
scrolls into view when the sidebar folds and when the workspace changes.

Holding ⌘ alone shows each workspace's chord after 100 ms
(`SidebarHints.swift`), so a chord typed at speed never flashes them: ⌘1…⌘8,
⌘9 for the last, none between. The full sidebar shows them at the end of
each row over its time; the rail shows the same key badge in each row's
icon's place, so its width and rows stay put. Both fade in and out over
80 ms (at once with motion off), and go on release, on any key, when
another modifier joins ⌘ and when the window stops being key. Opening the
sidebar or folding it to the rail with ⌘ still held (⌘A) shows them once
its motion ends. Checked by `e2e/wshints.sh`.

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
- **Workspace rows**: Rename…, New Tab, Mark All as Read, Close Workspace.
  No folder items: a workspace's panes each have their own folder, so the
  tile menu reveals or copies a pane's, and the Files sidebar shows the
  project folder.
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
