# Keybindings

**Every keyboard shortcut in Azulejo comes from the keymap, and every one is configurable.** That covers the app-wide chords, the ones that only apply in a tile kind (browser chrome, the files list, preview), the keys of quick open, find bars and resize mode, the menu bar's key equivalents and the `ctrl+b` doubling in terminals. The few keys left to AppKit are listed under [What isn't in the keymap](#what-isnt-in-the-keymap).

- The defaults live in [`shared/config/keymap.default.toml`](../shared/config/keymap.default.toml); the core embeds that file.
- Your overrides go in `~/.config/azulejo/config.toml` (XDG paths on both platforms), under `[keys]`. Saving the file applies them: no restart. A bad line shows in the non-blocking config banner and keeps its default.
- `azj config keys` (or `azj keys list`) prints every action with its context, default chord, current chord and whether your config set it; `--json` for scripts.
- `azj config defaults` prints a fully commented `[keys]` template with every action and its default, ready to paste.
- **Open config file** (`cmd+,`, **Settings…** in the app menu) opens the config file, creating it from that template when it's missing.
- Quick open (`cmd+k`) shows each command's current chord; the menu bar shows the chords of its items, so both follow your config.

The defaults start from the maintainer's herdr bindings; [Where the defaults come from](#where-the-defaults-come-from) explains them.

## Configuring

```toml
[keys]                          # every platform
goto = "cmd+p"                  # bind an action that has no default
quit = ""                       # unbind it ("none" works too)

[keys.macos]                    # one platform; beats [keys]
split_right = "cmd+d"           # replaces the default chord(s) for this action
promote_tile = ["alt+enter", "cmd+enter"]
switch_workspace = "ctrl+1..8"  # digit ranges, for switch_workspace and switch_tab

[keys.macos.browser]            # only while a browser tile has focus
toggle_sidebar = "cmd+b"
browser_back = "alt+left"       # replaces cmd+[ there

[keys.macos.palette]            # only while quick open is open
palette_next = ["down", "ctrl+j"]

[keys.macos.resize]             # resize mode: only these keys work
zoom_tile = "z"
```

Rules:

- **A line replaces that action's chords in its section.** Everything else keeps its default.
- **`""` or `"none"` unbinds** an action, in any section.
- **Sections:** `[keys]` and `[keys.<platform>]` (`macos`, `linux`) hold the global bindings. `[keys.<platform>.<context>]` (or `[keys.<context>]` for every platform) holds bindings that only apply in that context:

  | Context | Applies while |
  | --- | --- |
  | `terminal` | a terminal tile has focus |
  | `browser` | a browser tile has focus (page or address bar) |
  | `preview` | a preview tile has focus |
  | `files` | a files tile's list, or the sidebar's Files view, has the keyboard |
  | `palette` | quick open (`cmd+k`) is open |
  | `find_bar` | a find bar's field, or a files tile's filter, has the keyboard |
  | `resize` | resize mode (`resize_mode`) is on |

  The find bar context is `find_bar`, not `find`: `[keys.macos.find]` would clash with the `find` action in `[keys.macos]` in TOML.
- **Precedence:** the context's own binding wins over a global one on the same chord, the user's file wins over the defaults, and a platform section wins over a platform-less one. Global bindings still apply inside a context unless the context rebinds that chord; resize mode is the exception and only has its own keys. In quick open only `quick_open`, `command_palette` and `goto` run among the global chords; other chords go to its search field as typing.
- **Context-only actions** (`palette_next`, `browser_back`, `files_down`, `find_next`, `resize_mode_exit`, `send_herdr_prefix`, …) land in their context even when written under `[keys]` or `[keys.<platform>]`. Under another context's section, or in `[prefix.keys]`, they're ignored with a warning.
- **Chords** name the physical key, not the character it types: `ctrl+shift+minus` stays `minus`, never `underscore`, and `alt+h` matches even though it types `˙`. Modifiers are `ctrl`, `alt`, `shift` and `cmd` (`super` on Linux; the two are the same modifier). Keys: letters, digits, `f1`..`f24`, `enter`, `tab`, `escape`, `space`, `backspace`, `delete`, arrows, `home`, `end`, `pageup`, `pagedown`, and `minus`, `equal`, `[`, `]`, `\`, `;`, `'`, `` ` ``, `,`, `.`, `/` (or their names: `bracketleft`, `period`, …).
- **Conflicts** show as warnings in the config banner, with the line they come from: two actions on one chord (and which one wins, per context), a chord the OS takes first (Spotlight, screenshots, Lock Screen), a bare `ctrl+<letter>` taken from terminals, or a binding on the prefix key.

## Reference

Every action, by context, with its default on each platform. `azj config keys` shows the same with your overrides applied. "global" means it works everywhere unless a context rebinds the chord.

<!-- keys:begin (generated by `just keys-doc` from shared/config/keymap.default.toml; don't edit by hand) -->

| Action | Context | macOS | Linux | What it does |
| --- | --- | --- | --- | --- |
| `focus_tile_left` | global | `alt+h` | `alt+h` | Focus tile left |
| `focus_tile_down` | global | `alt+j` | `alt+j` | Focus tile down |
| `focus_tile_up` | global | `alt+k` | `alt+k` | Focus tile up |
| `focus_tile_right` | global | `alt+l` | `alt+l` | Focus tile right |
| `swap_tile_left` | global | `alt+shift+h` | `alt+shift+h` | Swap tile left |
| `swap_tile_down` | global | `alt+shift+j` | `alt+shift+j` | Swap tile down |
| `swap_tile_up` | global | `alt+shift+k` | `alt+shift+k` | Swap tile up |
| `swap_tile_right` | global | `alt+shift+l` | `alt+shift+l` | Swap tile right |
| `move_tile_left` | global | unbound | unbound | Move tile left |
| `move_tile_down` | global | unbound | unbound | Move tile down |
| `move_tile_up` | global | unbound | unbound | Move tile up |
| `move_tile_right` | global | unbound | unbound | Move tile right |
| `resize_tile_left` | global | `ctrl+alt+shift+h` | `ctrl+alt+shift+h` | Resize tile left |
| `resize_tile_left` | resize | `h`, `left` | `h`, `left` | Resize tile left |
| `resize_tile_down` | global | `ctrl+alt+shift+j` | `ctrl+alt+shift+j` | Resize tile down |
| `resize_tile_down` | resize | `j`, `down` | `j`, `down` | Resize tile down |
| `resize_tile_up` | global | `ctrl+alt+shift+k` | `ctrl+alt+shift+k` | Resize tile up |
| `resize_tile_up` | resize | `k`, `up` | `k`, `up` | Resize tile up |
| `resize_tile_right` | global | `ctrl+alt+shift+l` | `ctrl+alt+shift+l` | Resize tile right |
| `resize_tile_right` | resize | `l`, `right` | `l`, `right` | Resize tile right |
| `split_left` | global | unbound | unbound | Split left |
| `split_down` | global | `ctrl+cmd+shift+t` | `ctrl+super+shift+t` | Split down |
| `split_up` | global | unbound | unbound | Split up |
| `split_right` | global | `ctrl+cmd+v` | `ctrl+super+v` | Split right |
| `resize_mode` | global | `ctrl+alt+r` | `ctrl+alt+r` | Resize mode |
| `zoom_tile` | global | `alt+shift+m` | `alt+shift+m` | Zoom tile |
| `close_tile` | global | `alt+w` | `alt+w` | Close tile (detach) |
| `kill_tile` | global | unbound | unbound | Kill tile and its process |
| `cycle_tile_next` | global | `alt+bracketright` | `alt+bracketright` | Next tile |
| `cycle_tile_previous` | global | `alt+bracketleft` | `alt+bracketleft` | Previous tile |
| `rename_tile` | global | unbound | unbound | Rename tile… |
| `split_auto` | global | `ctrl+cmd+t` | `ctrl+super+t` | New tile in stack (tall) |
| `promote_tile` | global | `alt+enter` | `alt+enter` | Promote tile to master |
| `rotate_layout` | global | `alt+o` | `alt+o` | Rotate layout |
| `equalize_layout` | global | `alt+shift+e` | `alt+shift+e` | Equalize stack |
| `cycle_layout_next` | global | `alt+backslash` | `alt+backslash` | Next layout |
| `cycle_layout_previous` | global | `alt+shift+backslash` | `alt+shift+backslash` | Previous layout |
| `toggle_layout_tile` | global | `alt+t` | `alt+t` | Toggle Tile layout |
| `toggle_layout_monocle` | global | `alt+z` | `alt+z` | Toggle Monocle layout |
| `toggle_layout_three_column` | global | unbound | unbound | Toggle Three Column layout |
| `toggle_layout_manual` | global | `alt+shift+t` | `alt+shift+t` | Toggle manual layout (freeze) |
| `increase_masters` | global | `alt+equal` | `alt+equal` | More masters |
| `decrease_masters` | global | `alt+minus` | `alt+minus` | Fewer masters |
| `grow_master_area` | global | `alt+shift+equal` | `alt+shift+equal` | Grow master area |
| `shrink_master_area` | global | `alt+shift+minus` | `alt+shift+minus` | Shrink master area |
| `move_position_next` | global | `alt+shift+bracketright` | `alt+shift+bracketright` | Move tile to next position |
| `move_position_previous` | global | `alt+shift+bracketleft` | `alt+shift+bracketleft` | Move tile to previous position |
| `rotate_part` | global | `alt+shift+o` | `alt+shift+o` | Rotate master area |
| `rotate_clockwise` | global | unbound | unbound | Rotate layout clockwise |
| `rotate_anticlockwise` | global | unbound | unbound | Rotate layout anticlockwise |
| `new_workspace` | global | `cmd+t` | `super+t` | New workspace |
| `switch_workspace_last` | global | `cmd+9` | `super+9` | Switch to last workspace |
| `last_workspace` | global | `alt+0` | `alt+0` | Previous active workspace |
| `next_workspace` | global | unbound | unbound | Next workspace |
| `previous_workspace` | global | unbound | unbound | Previous workspace |
| `close_workspace` | global | `cmd+w` | `super+w` | Close workspace (detach) |
| `rename_workspace` | global | unbound | unbound | Rename workspace… |
| `new_tab` | global | `cmd+shift+t` | `super+shift+t` | New tab |
| `switch_tab_last` | global | `ctrl+9` | `ctrl+9` | Switch to last tab |
| `last_tab` | global | unbound | unbound | Previous active tab |
| `next_tab` | global | `ctrl+tab` | `ctrl+tab` | Next tab |
| `previous_tab` | global | `ctrl+shift+tab` | `ctrl+shift+tab` | Previous tab |
| `close_tab` | global | `cmd+shift+w` | `super+shift+w` | Close tab |
| `rename_tab` | global | unbound | unbound | Rename tab… |
| `new_window` | global | `cmd+n` | `super+n` | New window |
| `close_window` | global | unbound | unbound | Close window |
| `quick_open` | global | `cmd+k` | `super+k` | Quick open |
| `command_palette` | global | unbound | unbound | Commands… |
| `goto` | global | unbound | unbound | Go to… |
| `find` | global | `cmd+f` | `super+f` | Find in terminal… |
| `open_config` | global | `cmd+comma` | `super+comma` | Open config file |
| `reload_config` | global | unbound | unbound | Reload config |
| `toggle_sidebar` | terminal | `cmd+a` | `super+a` | Toggle sidebar |
| `edit_scrollback` | global | unbound | unbound | Edit scrollback |
| `quit` | global | `cmd+q` | `ctrl+shift+q` | Quit Azulejo |
| `hide` | global | `cmd+h` | unbound | Hide Azulejo |
| `hide_others` | global | `alt+cmd+h` | unbound | Hide other apps |
| `minimize` | global | `cmd+m` | unbound | Minimise window |
| `send_herdr_prefix` | terminal | `ctrl+b` | `ctrl+b` | Type herdr's prefix (ctrl+b) into the pane |
| `take_over` | terminal | unbound | unbound | Take over or give back a terminal |
| `palette_close` | palette | `escape` | `escape` | Quick open: close |
| `palette_run` | palette | `enter` | `enter` | Quick open: run the selected row |
| `palette_next` | palette | `down`, `ctrl+n`, `ctrl+j` | `down`, `ctrl+n`, `ctrl+j` | Quick open: next row |
| `palette_previous` | palette | `up`, `ctrl+p`, `ctrl+k` | `up`, `ctrl+p`, `ctrl+k` | Quick open: previous row |
| `palette_page_down` | palette | `pagedown` | `pagedown` | Quick open: page down |
| `palette_page_up` | palette | `pageup` | `pageup` | Quick open: page up |
| `palette_complete` | palette | `tab` | `tab` | Quick open: complete the query |
| `find_next` | find_bar | `enter`, `cmd+g` | `enter`, `ctrl+g` | Find: next match |
| `find_previous` | find_bar | `shift+enter`, `cmd+shift+g` | `shift+enter`, `ctrl+shift+g` | Find: previous match |
| `find_close` | find_bar | `escape` | `escape` | Find: close the bar |
| `resize_mode_exit` | resize | `escape`, `enter` | `escape`, `enter` | Resize mode: done |
| `browser_focus_address` | browser | `cmd+l` | `ctrl+l` | Browser: edit the address |
| `browser_reload` | browser | `cmd+r` | `ctrl+r` | Browser: reload |
| `browser_reload_from_origin` | browser | `cmd+shift+r` | `ctrl+shift+r` | Browser: reload from origin |
| `browser_back` | browser | `cmd+bracketleft` | `alt+left` | Browser: back |
| `browser_forward` | browser | `cmd+bracketright` | `alt+right` | Browser: forward |
| `browser_stop` | browser | `cmd+period` | unbound | Browser: stop loading |
| `browser_zoom_in` | browser | `cmd+equal` | `ctrl+equal` | Browser: zoom in |
| `browser_zoom_out` | browser | `cmd+minus` | `ctrl+minus` | Browser: zoom out |
| `browser_zoom_reset` | browser | `cmd+0` | `ctrl+0` | Browser: actual size |
| `browser_inspector` | browser | `alt+cmd+i` | `ctrl+shift+i` | Browser: Web Inspector |
| `preview_toggle_source` | preview | `alt+cmd+u` | `ctrl+u` | Preview: toggle Preview \| Source |
| `files_down` | files | `j`, `down` | `j`, `down` | Files: next row |
| `files_up` | files | `k`, `up` | `k`, `up` | Files: previous row |
| `files_open` | files | `enter` | `enter` | Files: open |
| `files_expand` | files | `l` | `l` | Files: expand folder or open file |
| `files_collapse` | files | `h` | `h` | Files: collapse folder or go to parent |
| `files_open_external` | files | `cmd+down` | `alt+down` | Files: open in default app |
| `files_parent` | files | `cmd+up` | `alt+up` | Files: go to parent folder |
| `files_quick_look` | files | `space` | `space` | Files: Quick Look |
| `files_filter` | files | `slash` | `slash` | Files: filter |
| `files_toggle_hidden` | files | `period` | `period` | Files: show or hide hidden files |
| `files_leave` | files | `escape` | `escape` | Files: back to the tiles (sidebar) |
| `switch_workspace` | global | `cmd+1..8` | `super+1..8` | Switch to workspace N |
| `switch_tab` | global | `ctrl+1..8` | `ctrl+1..8` | Switch to tab N |

Prefix mode (`[prefix] enabled = true`): press the prefix (`cmd+b` on macOS, `super+b` on Linux), then:

| Action | Then | What it does |
| --- | --- | --- |
| `focus_tile_left` | `h` | Focus tile left |
| `focus_tile_down` | `j` | Focus tile down |
| `focus_tile_up` | `k` | Focus tile up |
| `focus_tile_right` | `l` | Focus tile right |
| `split_down` | `minus` | Split down |
| `split_right` | `v` | Split right |
| `resize_mode` | `r` | Resize mode |
| `zoom_tile` | `z` | Zoom tile |
| `close_tile` | `x` | Close tile (detach) |
| `kill_tile` | `shift+k` | Kill tile and its process |
| `cycle_tile_next` | `tab` | Next tile |
| `cycle_tile_previous` | `shift+tab` | Previous tile |
| `rename_tile` | `shift+p` | Rename tile… |
| `new_workspace` | `shift+n` | New workspace |
| `close_workspace` | `shift+d` | Close workspace (detach) |
| `rename_workspace` | `shift+w` | Rename workspace… |
| `new_tab` | `c` | New tab |
| `next_tab` | `n` | Next tab |
| `previous_tab` | `p` | Previous tab |
| `close_tab` | `shift+x` | Close tab |
| `rename_tab` | `shift+t` | Rename tab… |
| `command_palette` | `shift+slash` | Commands… |
| `goto` | `w`, `g` | Go to… |
| `find` | `bracketleft` | Find in terminal… |
| `toggle_sidebar` | `b` | Toggle sidebar |
| `edit_scrollback` | `e` | Edit scrollback |
| `switch_tab` | `1..9` | Switch to tab N |

<!-- keys:end -->

## What isn't in the keymap

These stay with the system or AppKit, on purpose:

- **Text editing in text fields.** Quick open's field, find bars, the files filter, the browser address bar and rename prompts use AppKit's field editor: `cmd+a`, `cmd+c`, `cmd+v`, `cmd+x`, `cmd+z` and `cmd+shift+z` come from the standard Edit menu, and arrows, `alt+arrows`, `home`/`end`, `backspace` and so on are AppKit's text navigation. Rebinding them would break muscle memory every Mac text field shares, and the Edit menu items also carry copy and paste for terminals. A keymap chord on the same keys still wins (that's how `cmd+a` toggles the sidebar in terminals).
- **Field-level fallbacks.** In the browser address bar `escape` puts the page's URL back; in the files filter `down` moves into the list. Both are AppKit's cancel and move commands on the field, not chords. (`find_close` and `find_next` cover `escape` and `enter` in the files filter.)
- **The files list's `left` and `right` arrows**: AppKit's outline view collapses and expands folders with them. `files_collapse` and `files_expand` (`h` and `l`) are the keymap's versions.
- **Terminal input.** Whatever the keymap doesn't bind goes to the terminal untouched; Ghostty `keybind` lines in your Ghostty config still apply to those chords. Input methods (marked text) take every key while composing.
- **Web pages' own keys** in browser and preview tiles, for chords the keymap doesn't bind.
- **System chords:** `cmd+tab`, `cmd+space`, `` cmd+` ``, screenshots, Force Quit, Lock Screen (the WM's on Linux). The config warns when you bind one.
- **Quick open's `>` and `@`.** They're typed text at the start of the query, not keys. Bind `command_palette` or `goto` to open the menu with them already typed.
- **Quick Look's panel**, once open, handles its own keys.
- **Mouse and trackpad** gestures, and the buttons on the read-only overlay (`take_over` is the keyboard way to Take over / Take back).

## Where the defaults come from

The [reference](#reference) above is generated from the keymap and is the source of truth; this part says why the defaults are what they are.

### What the herdr config does today

herdr runs inside kitty, so its bindings were shaped by three outside constraints. Only the third goes away in Azulejo:

1. kitty eats any chord it maps, so every herdr chord is commented out in `kitty.conf`.
2. Hammerspoon owns several `alt` chords globally (see [Hammerspoon](#hammerspoon)).
3. macOS never delivers `cmd` chords to a terminal program, so kitty re-encodes the `cmd` ones and forwards them to herdr. **Azulejo is a native app, so it gets `cmd` chords directly and needs no forwarding.**

The config already uses a direct-modifier layer: `alt` for panes, `cmd` for workspaces and tabs. The prefix (`ctrl+b`, herdr's default) is left for herdr-only actions.

| herdr action | the maintainer's binding | herdr default | Customised |
| --- | --- | --- | --- |
| prefix | `ctrl+b` | `ctrl+b` | no |
| focus pane left/down/up/right | `alt+h/j/k/l` | `prefix+h/j/k/l` | **yes** |
| swap pane left/down/up/right | `alt+shift+h/j/k/l` | unbound | **yes** |
| resize pane left/down/up/right | `ctrl+alt+shift+h/j/k/l` | unbound | **yes** |
| resize mode (sticky hjkl) | `prefix+r` | `prefix+r` | no |
| zoom | `alt+shift+m` | `prefix+z` | **yes** |
| split down (`split_horizontal`) | `ctrl+cmd+shift+t` | `prefix+minus` | **yes** |
| split right (`split_vertical`) | `prefix+v` | `prefix+v` | no |
| "tall" smart split (`tall.py split`) | `ctrl+cmd+t` | n/a (script) | **yes** |
| promote to master (`tall.py promote`) | `alt+enter` | n/a (script) | **yes** |
| rotate tall layout (`tall.py rotate`) | `alt+o` | n/a (script) | **yes** |
| equalize stack (`tall.py equalize`) | `alt+shift+e` | n/a (script) | **yes** |
| close pane | `prefix+x` | `prefix+x` | no |
| cycle panes | `prefix+tab` / `prefix+shift+tab` | same | no |
| new workspace | `cmd+t` | `prefix+shift+n` | **yes** |
| new tab | `cmd+shift+t` | `prefix+c` | **yes** |
| switch workspace 1–9 | `cmd+1..9` | unbound | **yes** |
| last workspace (`last.py space`) | `alt+0` (kitty forwards `cmd+9` here) | n/a (script) | **yes** |
| switch tab 1–9 | `prefix+1..9` | `prefix+1..9` | no |
| last tab (`last.py tab`) | `alt+9` (now shadowed: Hammerspoon binds `alt+9` since its app list grew to nine) | n/a (script) | **yes** |
| next / previous tab | `ctrl+tab` / `ctrl+shift+tab` | `prefix+n` / `prefix+p` | **yes** |
| close tab | `cmd+w` | `prefix+shift+x` | **yes** |
| toggle sidebar | `cmd+a` | `prefix+b` | **yes** |
| copy mode (scrollback) | `prefix+[` | `prefix+[` | yes (spelling only) |
| workspace picker / goto | `prefix+w` / `prefix+g` | same | no |
| rename tab / workspace / pane | `prefix+shift+t/w/p` | same | no |
| close workspace | `prefix+shift+d` | same | no |
| detach | `prefix+q` | same | no |
| edit scrollback in `$EDITOR` | `prefix+e` | same | no |
| help | `prefix+?` | same | no |

One more thing from the config: `tall.py` encodes a placement rule, kitty's `tall` layout. There's one master on the left and an evenly divided stack on the right, and new panes always append to the bottom of the stack. That's how the maintainer actually tiles day to day, so the four tall actions are P0 and ship in M1 (step 1.1b); see [Decisions](#decisions).

### Rule of thumb

- **`alt` = tiles** (focus, swap, resize, zoom, close, layout rules), exactly as in herdr.
- **`cmd` = workspaces, tabs, app** (new, switch, close, sidebar, palette), exactly as in herdr, now native.
- **`ctrl+cmd` = new tiles** (splits), as in herdr.
- No chord in the default layer uses `ctrl+<letter>` alone, so shells, editors and TUIs keep every control key.

### macOS defaults

Where each global default came from:

| Azulejo action | macOS | Source |
| --- | --- | --- |
| `focus_tile_left/down/up/right` | `alt+h/j/k/l` | herdr |
| `swap_tile_left/down/up/right` | `alt+shift+h/j/k/l` | herdr (needs a Hammerspoon change, below) |
| `resize_tile_left/down/up/right` | `ctrl+alt+shift+h/j/k/l` | herdr |
| `resize_mode` | `ctrl+alt+r` | new direct chord for herdr's `prefix+r` |
| `zoom_tile` | `alt+shift+m` | herdr |
| `split_auto` (tall rule) | `ctrl+cmd+t` | herdr `tall.py split` |
| `split_down` | `ctrl+cmd+shift+t` | herdr |
| `split_right` | `ctrl+cmd+v` | new direct chord for herdr's `prefix+v` |
| `promote_tile` | `alt+enter` | herdr `tall.py promote`; in a dynamic tab, moves the tile to the front of the order |
| `rotate_layout` | `alt+o` | herdr `tall.py rotate` |
| `equalize_layout` | `alt+shift+e` | herdr `tall.py equalize`; in a dynamic tab, resets the stack sizes |
| `cycle_layout_next/previous` | `alt+\` / `alt+shift+\` | Bismuth `Meta+\` / `Meta+\|` (ADR 0010) |
| `toggle_layout_tile` | `alt+t` | Bismuth `Meta+T`; bash/zsh lose `alt+t` (transpose words) inside tiles |
| `toggle_layout_monocle` | `alt+z` | Bismuth `Meta+M`; `alt+m` is Hammerspoon's and `alt+shift+m` is zoom |
| `toggle_layout_manual` (freeze to a split tree, or restore it) | `alt+shift+t` | new |
| `increase_masters` / `decrease_masters` | `alt+=` / `alt+-` | Bismuth `Meta+]` / `Meta+[` (`alt+]`/`alt+[` already cycle tiles) |
| `grow_master_area` / `shrink_master_area` | `alt+shift+=` / `alt+shift+-` | unbound in Bismuth; resizing the master edge does the same |
| `move_position_next/previous` | `alt+shift+]` / `alt+shift+[` | unbound in Bismuth |
| `rotate_part` (several masters side by side ↔ stacked) | `alt+shift+o` | Bismuth `Meta+Shift+R` |
| `close_tile` (detaches the herdr pane, F14) | `alt+w` | new: the `alt` twin of `cmd+w` |
| `cycle_tile_next/previous` | `alt+]` / `alt+[` | new direct chord for `prefix+tab` |
| `new_workspace` | `cmd+t` | herdr |
| `new_tab` | `cmd+shift+t` | herdr |
| `switch_workspace` 1–8 | `cmd+1..8` | herdr |
| `switch_workspace_last` (the last workspace in the sidebar) | `cmd+9` | browser convention: `cmd+9` is the last tab in Chrome and Safari |
| `last_workspace` (the previously active workspace) | `alt+0` | herdr `last.py space` |
| `close_workspace` (detaches, F14) | `cmd+w` | new: closes what `cmd+t` opens |
| `switch_tab` 1–8 | `ctrl+1..8` | new (herdr's own suggestion for direct tab switching) |
| `switch_tab_last` (the last tab in order) | `ctrl+9` | browser convention, as `cmd+9` |
| `last_tab` (the previously active tab) | unbound | herdr's `alt+9`; Hammerspoon owns `alt+9`, so it's palette-only |
| `next_tab` / `previous_tab` | `ctrl+tab` / `ctrl+shift+tab` | herdr |
| `close_tab` | `cmd+shift+w` | was `cmd+w` (herdr); moved so it mirrors `cmd+shift+t` |
| `toggle_sidebar` | `cmd+a`, **terminal tiles only** | herdr (see conflicts) |
| `quick_open` (one menu: commands, workspaces, tabs and tiles) | `cmd+k` | new (Raycast / Linear / mockup convention); covers `prefix+w`, `prefix+g` and herdr's help |
| `command_palette` (quick open, commands only: opens with `>`) | unbound | was `cmd+shift+p`; rebind it to get that back |
| `goto` (quick open, places only: opens with `@`) | unbound | was `cmd+p`; rebind it to get that back |
| `find` (scrollback search via herdr in terminals, ADR 0005; page find in browsers and previews; the filter in files) | `cmd+f` | new |
| `new_window` | `cmd+n` | macOS convention |
| `open_config` | `cmd+,` | macOS convention |

The app menu keeps the macOS chords, now as actions: `quit` (`cmd+q`), `hide` (`cmd+h`), `hide_others` (`alt+cmd+h`) and `minimize` (`cmd+m`). Keys that only work in one tile kind or mode (browser chrome, the files list, preview's Preview | Source, quick open, find bars, resize mode) keep the conventions they had before they were configurable; they're in the reference.

Rename, kill the herdr pane, help and edit-scrollback are left to the palette, prefix mode and the right-click menus (workspace rows in the sidebar, tile headers). So are `toggle_layout_three_column`, `rotate_clockwise` and `rotate_anticlockwise` (four-way Tile rotation) and "Set layout…".

New tabs start as a dynamic Tile layout (`[layout] default`, ADR 0010), so a plain split (`ctrl+cmd+v`, `ctrl+cmd+shift+t`) adds the tile at the end of the stack instead of cutting the focused tile in two; `alt+shift+t` switches the tab to manual splitting. Tabs adopted from herdr stay manual.

### Quick open (⌘K)

One menu for commands and places. On an empty query it lists recent rows first (commands and places mixed), then workspaces, tabs and tiles, then commands. Typing fuzzy-matches across all of them, ranked together: whole or leading matches on a name come first, places win a close tie with commands, and recently used rows get a boost. Each row says what it is (Workspace, Tab, Terminal, Browser, Preview, Files or Command), and commands show their chords.

- `>` at the start limits it to commands (what `cmd+shift+p` used to open).
- `@` limits it to workspaces, tabs and tiles (what `cmd+p` used to open).
- Commands that need input (renames, open URL, layouts) ask for it inline, as before.
- `command_palette` and `goto` still exist as actions with no default chord. Bind them to open the menu with `>` or `@` already typed:

```toml
[keys]
command_palette = "cmd+shift+p"
goto            = "cmd+p"
```

**`cmd+k` and Ghostty.** Ghostty binds `cmd+k` to `clear_screen` by default; Azulejo's key router sees the chord first, so in Azulejo `cmd+k` opens quick open. Clearing still works with `ctrl+l` (the shell clears and redraws). Chords Azulejo doesn't bind still reach the terminal surface, so Ghostty `keybind` lines for other keys keep working, e.g. `keybind = cmd+shift+k=clear_screen` in your Ghostty config. Under herdr that clear is cosmetic: herdr owns the screen and its scrollback, and redraws on the next update.

### Linux defaults

**`cmd` maps to Super** (decided), so the macOS chords carry over one for one: `cmd+t` becomes `super+t`, `ctrl+cmd+t` becomes `ctrl+super+t`, and so on. `alt` and `ctrl` chords stay as they are. There's no Hammerspoon on Linux, so nothing else changes.

| Azulejo action | Linux |
| --- | --- |
| tile and layout actions (`alt+…`) | same as macOS, including the dynamic-layout chords |
| `split_auto` / `split_down` / `split_right` | `ctrl+super+t` / `ctrl+super+shift+t` / `ctrl+super+v` |
| `new_workspace` / `new_tab` | `super+t` / `super+shift+t` |
| `switch_workspace` 1–8, `switch_workspace_last`, `last_workspace` | `super+1..8`, `super+9`, `alt+0` |
| `switch_tab` 1–8, `switch_tab_last` | `ctrl+1..8`, `ctrl+9` (`last_tab` unbound) |
| `next_tab` / `previous_tab` | `ctrl+tab` / `ctrl+shift+tab` |
| `close_workspace` / `close_tab` | `super+w` / `super+shift+w` |
| `toggle_sidebar` | `super+a`, terminal tiles only |
| `quick_open` / `find` | `super+k` / `super+f` (`command_palette` and `goto` unbound, as on macOS) |
| `new_window` / `open_config` | `super+n` / `super+comma` |

**Caveat: the window manager sees Super first.** On Wayland and X11 the compositor's own bindings win, so a clash means Azulejo never gets the key. Known clashes with stock defaults:

- **GNOME:** `super+1..9` (launch dash favourites), `super+a` (app grid), `super+p` (display switcher), `super+n` (focus notification), `super+tab`.
- **KDE Plasma:** `super+1..9` (task manager entries), `super+tab`.
- **Hyprland / Sway / i3:** depends on the user's config, but `super+1..9`, `super+w`, `super+f`, `super+enter` and `super+shift+…` are the usual workspace and window binds.

Either free those chords in the WM, or switch Azulejo to the `ctrl+shift` layout below.

#### Linux alternative: `ctrl+shift`

The Linux terminal convention (kitty, GNOME Terminal, Ptyxis). It avoids every WM clash. Copy it into `[keys.linux]` in your config to use it:

| Azulejo action | `ctrl+shift` layout | Why it differs from a plain super→ctrl+shift swap |
| --- | --- | --- |
| `split_auto` | `ctrl+shift+enter` | `ctrl+alt+t` opens a terminal in Ubuntu; kitty uses `ctrl+shift+enter` for "new window" |
| `split_down` / `split_right` | `ctrl+shift+minus` / `ctrl+shift+backslash` | visual mnemonics, no 4-modifier chords |
| `new_workspace` / `new_tab` | `ctrl+shift+t` / `ctrl+alt+shift+t` | |
| `switch_workspace` 1–8, `switch_workspace_last`, `last_workspace` | `alt+1..8`, `alt+9`, `alt+0` | `ctrl+shift+digit` yields `!@#` on many layouts, and `alt+N` is the Linux terminal convention |
| `close_workspace` / `close_tab` | `ctrl+shift+w` / `ctrl+alt+shift+w` | mirrors `new_workspace` / `new_tab` |
| `toggle_sidebar` | `ctrl+shift+b` | `ctrl+shift+a` is select-all in GTK terminals |
| `quick_open` / `find` | `ctrl+shift+p` / `ctrl+shift+f` | VS Code's palette chord; `ctrl+p` is shell history and `ctrl+shift+k` is scroll-up in kitty |
| `new_window` / `open_config` | `ctrl+shift+n` / `ctrl+comma` | |

Match keys by physical key (hardware keycode), not by the shifted keyval. Otherwise `ctrl+shift+minus` arrives as `underscore` on GTK, and `alt+shift+h` arrives as `H`.

### Optional prefix mode

Off by default. When on, it mirrors herdr's prefix sequences exactly (`prefix+x`, `prefix+v`, `prefix+minus`, `prefix+1..9`, `prefix+shift+w`, …), so anything without a direct chord is still a two-key sequence away.

- **Prefix: `cmd+b` on macOS, `super+b` on Linux** (`ctrl+shift+b` in the `ctrl+shift` layout, which then moves the sidebar toggle to the palette and prefix mode). It's the same letter as herdr's `ctrl+b`, but it never reaches the PTY. So it can't trip the attach client's `ctrl+b q` detach, and the shell keeps `ctrl+b` (backward-char).
- `ctrl+b` as the Azulejo prefix would make the key router swallow the shell's `ctrl+b`. Passing it through instead hands it to the attach client, where `ctrl+b q` detaches. Either way it's worse.

### `ctrl+b` inside terminal tiles

This is independent of prefix mode. The attach client treats a typed `ctrl+b` then `q` as detach ([herdr notes §7](herdr-notes.md), [ADR 0001](adr/0001-terminal-bridge.md)). Typing `ctrl+b` (backward-char in fish) then `q` is ordinary editing, so it will happen.

The fix is the `send_herdr_prefix` action, bound to `ctrl+b` in `[keys.<platform>.terminal]`: the key router sends every user `ctrl+b` as `ctrl+b ctrl+b`, which the attach client delivers to the pane as one literal `ctrl+b`. 1.7 checked it with and without the kitty keyboard protocol. If your herdr uses another prefix, bind `send_herdr_prefix` to that chord instead; unbinding it (`send_herdr_prefix = ""`) sends `ctrl+b` once, and ADR 0003's silent reattach is the safety net.

## Conflicts

**macOS system and app conventions**

| Chord | Convention | Decision |
| --- | --- | --- |
| `cmd+w` / `cmd+shift+w` | close tab / close window | **`close_workspace` / `close_tab`** (Nov 4, 2026), mirroring `cmd+t` / `cmd+shift+t`. Both detach: herdr keeps the workspace, tabs and panes running, and Azulejo doesn't adopt them again, so there's no confirmation. Closing the window's last workspace leaves a fresh one in its place. `cmd+w` closes the workspace in browser and preview tiles too: the key router sees it before the web view. Close Window has no default chord (the red button, or bind `close_window`) |
| `cmd+q` | quit | `quit`. Safe: sessions live in herdr |
| `cmd+h` / `cmd+m` / `cmd+`` | hide / minimise / cycle windows | `hide` and `minimize`, as in every Mac app; `` cmd+` `` is the system's. the maintainer's config avoids `cmd+m` for this reason |
| `cmd+a` | select all | **Decided:** `toggle_sidebar` is bound **only when a terminal tile is focused**; in browser and preview tiles and text fields (address bar, palette) `cmd+a` stays select-all. Done with the `[keys.<platform>.terminal]` context section |
| `cmd+t` / `cmd+shift+t` | new tab / reopen closed tab | used for workspace / tab, as in herdr. No clash because browser tiles have no inner tabs |
| `cmd+9` / `ctrl+9` | last tab (browsers) | `switch_workspace_last` / `switch_tab_last`: the last in order, as in a browser. The previously active workspace is `alt+0` |
| `cmd+p` | print | taken by `goto`. Printing a browser tile goes through the palette |
| `cmd+k` | clear (Ghostty, Terminal.app) | not bound. A local clear does nothing under herdr's re-rendering. A later `clear_screen` action should send `ctrl+l` to the pane |
| `ctrl+cmd+f` / `ctrl+cmd+q` / `ctrl+cmd+space` | full screen / lock screen / emoji | avoided |
| `alt+<letter>` | types `˙∆˚¬…` in text fields | in browser tiles `alt+h/j/k/l/w/o` won't type those characters, which is acceptable. Shells lose fish's `alt+h` (help), `alt+l` (list dir) and `alt+w` (whatis), as they already do under herdr |

**Browser-tile shortcuts** (the `browser` context): `cmd+l` (address bar), `cmd+r` (reload), `cmd+[` / `cmd+]` (back/forward) and `cmd+f` (find) don't collide with anything above. `cmd+[` / `cmd+]` were deliberately not used for tab cycling; `ctrl+tab` covers that.

**Find bar (`cmd+f`):** in terminal, browser and preview tiles a bar opens under the header. Its keys are the `find_bar` context: `enter` or `cmd+g` goes to the next match, `shift+enter` or `cmd+shift+g` to the previous one, and `esc` closes the bar and hands the keyboard back to the tile. While you type there, other chords resolve without the tile's context, so `cmd+a` selects the query. In terminals the search runs over herdr's history (at most its last 1000 rows; the count shows `+` when that cut anything off), newest match first. The pane scrolls so the match sits mid-view, libghostty highlights every match in view, and `esc` scrolls back to where you were. In files tiles, `cmd+f` opens the filter (like `/`), and the filter field uses the same `find_bar` keys: `enter` moves into the list, `esc` clears it.

### Hammerspoon

If Hammerspoon owns a chord globally, it may intercept it before Azulejo. Exclude `dev.azulejo.Azulejo` from conflicting rules, or override the corresponding action in Azulejo's keymap.

For example, a global `alt+shift+h/j/k/l` window mover must stand aside for Azulejo if those chords should swap tiles instead.

## Decisions

Settled with the maintainer on Nov 3, 2026:

1. **Tall layout is in the MVP.** `split_auto`, `promote_tile`, `rotate_layout` and `equalize_layout` are P0 and implemented in the core (step 1.1b, `layout.split_auto`, `layout.promote`, `layout.rotate`, `layout.equalize`). F3's other mode, niri-style scrolling columns, stays P2.
2. **`cmd+a` toggles the sidebar only in terminal tiles**; it's select-all everywhere else.
3. **Linux uses Super by default** for the `cmd` chords, with the `ctrl+shift` layout documented as the alternative. **Every shortcut is configurable.**
