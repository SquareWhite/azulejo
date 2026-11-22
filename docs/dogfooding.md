# Running a daily build

How to use Azulejo day to day on your own Mac, built from the latest commit,
and how to update or roll it back.

## Install

You need herdr on your `PATH`, and the repo set up for macOS builds (`mise
install`; `vendor/ghostty` checked out and built with `just ghostty`). Then,
from the repo:

```sh
just dogfood
```

That builds HEAD, installs it as `/Applications/Azulejo.app` and launches it.
A build made on this Mac carries no quarantine flag, so Gatekeeper doesn't
ask. (A .dmg copied from elsewhere does: right-click Azulejo in Applications
→ **Open**, or run `xattr -dr com.apple.quarantine /Applications/Azulejo.app`.)

Then choose **Install Command-Line Tool…** in Azulejo's app menu. It asks
first, then links `azj` into `~/.local/bin` or `/usr/local/bin`
([details](release.md#install-command-line-tool)). Open a new terminal and
try `azj help`. The link points into the app bundle, so `azj` updates with
the app.

## Updating

```sh
just dogfood              # build HEAD, quit Azulejo, swap it into /Applications, relaunch
just dogfood --check      # run just build, test and lint on that commit first
just dogfood --dirty      # build the checkout as it is, uncommitted changes included
just dogfood --no-launch  # install without relaunching
```

- It builds the last commit in a separate worktree
  (`~/Library/Caches/Azulejo/dogfood-src`), so uncommitted changes in your
  checkout, including agents' work in progress, stay out. The worktree is
  kept between runs, so rebuilds are quick.
- That worktree reuses your checkout's `vendor/ghostty`. If the submodule
  isn't at the commit HEAD pins, the script stops: run `git submodule update`
  and `just ghostty`, then try again.
- Quitting saves the layout, and herdr keeps every terminal and agent
  running, so an update costs a second or two of reattaching. If Azulejo
  doesn't quit within 10 s the script stops and changes nothing;
  `--force-kill` sends SIGKILL instead, losing layout changes since the last
  save.
- The new build's signature is checked before anything is swapped.
- It relaunches only when its output is a terminal. From a script or an
  agent, run it as `script -q /dev/null just dogfood < /dev/null`.

## Rolling back

```sh
just dogfood --rollback   # the build before the installed one (again to go further back)
just dogfood --list       # kept builds; * marks the installed one
just dogfood --to <hash>  # reinstall a kept build by commit
```

The last three builds, plus the installed one, stay in
`~/Library/Application Support/Azulejo/dogfood-builds/`. An app you installed
by hand (from a .dmg) is kept there too the first time `just dogfood`
replaces it, so `--rollback` can get back to it.

Each build records its commit (`AzulejoCommit` in `Info.plist`); `just
dogfood --list` shows it. Mention it when you report a problem.

## First launch

Azulejo connects to your **default herdr session**, the one plain `herdr`
uses.

- **Adoption.** Every herdr workspace shows up as an Azulejo workspace, every
  herdr tab as a tab with the same splits, every pane as a terminal tile.
  Adoption only reads herdr; it moves, closes and re-lays-out nothing.
- **Layout mirror.** After that, one Azulejo tab is one herdr tab and each
  terminal tile a pane in it. Splits, resizes and swaps you make in Azulejo
  show up in the herdr TUI, and the other way round. `[herdr] mirror_layout =
  false` gives each tile a herdr tab of its own instead.
- **Closing.** `alt+w` closes the tile and the pane keeps running in herdr.
  `cmd+w` closes the tile and kills its pane at once, with no confirmation. A
  closed adopted tile stays closed on relaunch. `alt+cmd+w` closes the whole
  workspace in herdr too, and asks first when something still runs there.
  Renames go both ways.
- **Pane size.** While a tile shows a pane, Azulejo sets that pane's size. If
  the herdr TUI (or your phone) shows the same pane, it looks cropped or
  padded there until you close the tile.
- **Read-only tiles.** If another client already holds a pane, the tile shows
  it read-only with a **Take over** button. Azulejo never grabs it back by
  itself.
- **Environment.** Panes inherit the environment of whoever started the herdr
  server. If you started it from inside an agent, `CLAUDE_*` variables can
  show up in panes; restart the herdr server from a plain terminal.

## Setting up

### Keyboard tools

External keyboard remappers can intercept shortcuts before Azulejo sees them.
If you use Hammerspoon or Karabiner, scope conflicting rules to exclude
Azulejo's bundle ID, `dev.azulejo.Azulejo`. A global window-movement rule can
take `alt+shift+hjkl`. Adjust your rules, or override Azulejo's shortcuts under
`[keys.macos]` ([keybindings](keybindings.md)).

### Terminal font and colours

Azulejo's terminals read Ghostty's config, including
`~/Library/Application Support/com.mitchellh.ghostty/config.ghostty`. Font and
colour settings there apply to terminals. Ghostty's colours win over
Azulejo's theme by default; set `[appearance] terminal_colors = "theme"` to
let the theme colour terminals too ([themes](themes.md#terminal-colours)).

### Dock badge

The Dock badge counts unread agent tiles; `[appearance] dock_badge = false`
hides it ([themes](themes.md#dock-badge)).

## Everyday use

- herdr's chords carry over: `alt+hjkl` focus, `alt+shift+hjkl` swap,
  `ctrl+alt+shift+hjkl` resize, `ctrl+cmd+t` tall split, `alt+enter` promote,
  `alt+o` rotate, `alt+shift+e` equalize, `alt+shift+m` zoom. `cmd+t` and
  `cmd+1..8` for workspaces, `cmd+shift+t` and `ctrl+1..8` for tabs. Full list
  and overrides: [keybindings](keybindings.md).
- `cmd+k` (or the ⌘ button, top right) opens quick open: every action (with
  its chord) and every workspace, tab and tile (by name, cwd, path or agent),
  fuzzy, agents that need you first. Start with `>` for commands only or `@`
  for places only. In terminals
  `cmd+k` doesn't clear the screen as in Ghostty: use `ctrl+l`.
- Agent state shows in the sidebar, the tab strip, ⌘K, tile headers and the
  Dock badge ([themes](themes.md#agent-state-in-tile-headers)). Right-click a
  tile's header and choose Mark as Unread to come back to it later.
- `cmd+s` in a terminal labels the URLs, paths and commands on screen to copy,
  open or run them.
- The mouse: drag the gutter between tiles to resize (double-click evens the
  split), drag tabs in the top bar or workspaces in the sidebar to reorder
  them, and drop files, images or text onto a terminal to paste their paths or
  contents. Files split the terminal in two while you drag: the top half
  moves them into the shell's folder (hold ⌥ to copy; a remote pane gets them
  uploaded with scp), the bottom half pastes their paths. A name that's
  already there gets a number ("notes 2.txt"); nothing is overwritten.
- Text selection: a plain drag selects at a shell prompt. Programs that take
  the mouse (vim, most agent TUIs) get plain drags; hold shift to select.
  Programs on `[terminal] select_on_drag` (`["omp"]` by default) select on a
  plain drag but still get clicks and the wheel. Add yours:

  ```toml
  [terminal]
  select_on_drag = ["omp", "htop"]
  ```
- `azj files open --path plan.md` from a terminal edits the file beside it
  (`vim` in a new terminal), as Return does in the sidebar's Files view.
  Pick another editor, or a Mac app:

  ```toml
  [editor]
  command = "nvim -p"   # in a new pane, on the file's machine
  app = "Zed"           # Local files; with Zed, remote ones too (over SSH)
  ```

  With `app = "Zed"` (or Zed Preview), a remote workspace's files open in
  Zed's own SSH remoting, as `zed://ssh/<target>/<path>`. Zed connects with
  the machine's SSH target as herdr saved it and reads `~/.ssh/config`
  itself. Zed opens the file on its own: a Zed window already showing that
  host's project picks it up. Other apps, and a `[herdr] ssh` wrapper that
  Zed's SSH can't follow, fall back to `command`.

  Folders open too: Open Folder in Editor in the Files view's right-click
  menu (on a folder, or on empty space for the root), or ⌘⇧O (a file's
  folder when a file is selected). The app gets the folder (Zed opens it as
  a project, a remote one over SSH); otherwise `command <folder>` runs in a
  new pane in that folder, where vim and nvim show their file browser.
- The Files view makes, deletes, copies and pastes files: ⌘N, ⌘⇧N, ⌘⌫
  (to the Trash; a remote machine's after a yes), ⌘C, ⌘V, or its right-click
  menu. Drag files in from Finder and out to Finder or a terminal.

- Config lives in `~/.config/azulejo/config.toml`. Saving the file applies
  it; problems show in a banner over the tiles, and the previous config stays
  until the file is fixed. `reload_config` in ⌘K reads it again by hand.

## Where things live

| What | Where |
| --- | --- |
| Session state | `~/Library/Application Support/Azulejo/state.json` |
| Config | `~/.config/azulejo/config.toml` |
| API socket | `$TMPDIR/azulejo/azulejo.sock` |
| Kept builds | `~/Library/Application Support/Azulejo/dogfood-builds/` |
| Crash reports | `~/Library/Logs/DiagnosticReports/Azulejo-*.ips` |
| Logs | Off by default. For a session with logs, quit Azulejo and run `AZULEJO_VERBOSE=1 /Applications/Azulejo.app/Contents/MacOS/Azulejo 2>>~/Library/Logs/Azulejo.log &` |

## Reporting a problem

Note the build's commit (`just dogfood --list`), what happened, what you
expected, and the context (tile kinds, herdr TUI open, after sleep…). Attach
the crash report or log lines if you have them, with private paths and names
taken out.

## Known limits

- herdr machines are tested against an SSH stand-in only, and remote
  workspaces show no git branch.
- OSC 52 clipboard writes and terminal notifications don't pass through
  herdr ([herdr notes](herdr-notes.md)).
- A build without a Developer ID is ad-hoc signed and runs only on the Mac
  that built it ([release](release.md)).

## Checks that need you

Automated tests never post real input or bring windows to the front, so a
few things need a person at the Mac:

- Real typing feel, IME input (Japanese, Chinese) and Claude Code's
  interactive keys (shift+enter, esc esc, ctrl+r) in a terminal tile.
- System clipboard copy and paste, mouse selection and the scroll wheel.
- Real drags onto a terminal: files from Finder, an image from a browser, a
  screenshot thumbnail.
- Sleep and wake: close the lid for a minute with several terminal tiles
  open; they should reattach within 2 s.
- Sharing a pane with the herdr TUI.
- `scripts/latency/run-os-input.sh`: keystroke latency through the real input
  path, against standalone Ghostty.app. It posts real keystrokes for several
  minutes, so only run it when nobody needs the Mac
  ([details](../scripts/latency/README.md)).
