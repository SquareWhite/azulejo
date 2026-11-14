# Default keybindings

Azulejo's default keymap starts from the maintainer's herdr bindings (`~/.config/herdr/config.toml`), so the muscle memory carries over. The defaults live in [`shared/config/keymap.default.toml`](../shared/config/keymap.default.toml), which the core's config crate (`core/crates/config`) loads in M1. **Every binding is user-overridable**; see [Overriding bindings](#overriding-bindings).

## What the herdr config does today

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

## The proposal

### Rule of thumb

- **`alt` = tiles** (focus, swap, resize, zoom, close, layout rules), exactly as in herdr.
- **`cmd` = workspaces, tabs, app** (new, switch, close, sidebar, palette), exactly as in herdr, now native.
- **`ctrl+cmd` = new tiles** (splits), as in herdr.
- No chord in the default layer uses `ctrl+<letter>` alone, so shells, editors and TUIs keep every control key.

### macOS defaults

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
| `last_workspace` | `cmd+9` and `alt+0` | herdr (natively now, no forwarding trick) |
| `switch_tab` 1–8 | `ctrl+1..8` | new (herdr's own suggestion for direct tab switching) |
| `last_tab` | `ctrl+9` | herdr's `alt+9`, moved: Hammerspoon now owns `alt+9` |
| `next_tab` / `previous_tab` | `ctrl+tab` / `ctrl+shift+tab` | herdr |
| `close_tab` | `cmd+w` | herdr |
| `toggle_sidebar` | `cmd+a`, **terminal tiles only** | herdr (see conflicts) |
| `quick_open` (one menu: commands, workspaces, tabs and tiles) | `cmd+k` | new (Raycast / Linear / mockup convention); covers `prefix+w`, `prefix+g` and herdr's help |
| `command_palette` (quick open, commands only: opens with `>`) | unbound | was `cmd+shift+p`; rebind it to get that back |
| `goto` (quick open, places only: opens with `@`) | unbound | was `cmd+p`; rebind it to get that back |
| `find` (scrollback search via herdr, ADR 0005; page find in browser) | `cmd+f` | new |
| `new_window` | `cmd+n` | macOS convention |
| `open_config` | `cmd+,` | macOS convention |

Rename, close workspace, kill the herdr pane, help and edit-scrollback are left to the palette and to prefix mode. So are `toggle_layout_three_column`, `rotate_clockwise` and `rotate_anticlockwise` (four-way Tile rotation) and "Set layout…".

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
| `switch_workspace` 1–8, `last_workspace` | `super+1..8`, `super+9` and `alt+0` |
| `switch_tab` 1–8, `last_tab` | `ctrl+1..8`, `ctrl+9` |
| `next_tab` / `previous_tab` | `ctrl+tab` / `ctrl+shift+tab` |
| `close_tab` | `super+w` |
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
| `switch_workspace` 1–8, `last_workspace` | `alt+1..8`, `alt+9`/`alt+0` | `ctrl+shift+digit` yields `!@#` on many layouts, and `alt+N` is the Linux terminal convention |
| `close_tab` | `ctrl+shift+w` | |
| `toggle_sidebar` | `ctrl+shift+b` | `ctrl+shift+a` is select-all in GTK terminals |
| `quick_open` / `find` | `ctrl+shift+p` / `ctrl+shift+f` | VS Code's palette chord; `ctrl+p` is shell history and `ctrl+shift+k` is scroll-up in kitty |
| `new_window` / `open_config` | `ctrl+shift+n` / `ctrl+comma` | |

Match keys by physical key (hardware keycode), not by the shifted keyval. Otherwise `ctrl+shift+minus` arrives as `underscore` on GTK, and `alt+shift+h` arrives as `H`.

### Overriding bindings

Every binding, including the defaults above, the per-tile-kind context sections and prefix mode, is overridable from `~/.config/azulejo/config.toml`, in the same shape as `keymap.default.toml`:

```toml
[keys.linux]
new_workspace = "ctrl+shift+t"   # replaces the default chord(s) for this action
goto = ""                        # unbinds it
promote_tile = ["alt+enter", "super+enter"]

[keys.macos.browser]
toggle_sidebar = "cmd+b"         # only while a browser tile is focused
```

A user entry replaces the default for that action only; everything else keeps its default. Overrides load at startup in M1; reloading on save comes with F25 in M2.

### Optional prefix mode

Off by default. When on, it mirrors herdr's prefix sequences exactly (`prefix+x`, `prefix+v`, `prefix+minus`, `prefix+1..9`, `prefix+shift+w`, …), so anything without a direct chord is still a two-key sequence away.

- **Prefix: `cmd+b` on macOS, `super+b` on Linux** (`ctrl+shift+b` in the `ctrl+shift` layout, which then moves the sidebar toggle to the palette and prefix mode). It's the same letter as herdr's `ctrl+b`, but it never reaches the PTY. So it can't trip the attach client's `ctrl+b q` detach, and the shell keeps `ctrl+b` (backward-char).
- `ctrl+b` as the Azulejo prefix would make the key router swallow the shell's `ctrl+b`. Passing it through instead hands it to the attach client, where `ctrl+b q` detaches. Either way it's worse.

### `ctrl+b` inside terminal tiles

This is independent of prefix mode. The attach client treats a typed `ctrl+b` then `q` as detach ([herdr notes §7](herdr-notes.md), [ADR 0001](adr/0001-terminal-bridge.md)). Typing `ctrl+b` (backward-char in fish) then `q` is ordinary editing, so it will happen.

The proposed fix: the terminal tile sends every user `ctrl+b` as `ctrl+b ctrl+b`, which the attach client delivers to the pane as one literal `ctrl+b`. On macOS that's a surface-level Ghostty keybind override (`keybind = ctrl+b=text:\x02\x02`) or the key router. ADR 0003's silent reattach stays as the safety net.

To verify in 1.7: whether this still holds when the pane has the kitty keyboard protocol active, since `ctrl+b` may then be encoded as `CSI 98;5u` rather than `0x02`.

## Conflicts

**macOS system and app conventions**

| Chord | Convention | Decision |
| --- | --- | --- |
| `cmd+w` | close window/tab | `close_tab`, consistent with the convention. Closing the last tab closes the window. Panes stay alive in herdr |
| `cmd+q` | quit | kept as quit. Safe: sessions live in herdr |
| `cmd+h` / `cmd+m` / `cmd+`` | hide / minimise / cycle windows | not bound. the maintainer's config avoids `cmd+m` for this reason |
| `cmd+a` | select all | **Decided:** `toggle_sidebar` is bound **only when a terminal tile is focused**; in browser and preview tiles and text fields (address bar, palette) `cmd+a` stays select-all. Done with the `[keys.<platform>.terminal]` context section |
| `cmd+t` / `cmd+shift+t` | new tab / reopen closed tab | used for workspace / tab, as in herdr. No clash because browser tiles have no inner tabs |
| `cmd+p` | print | taken by `goto`. Printing a browser tile goes through the palette |
| `cmd+k` | clear (Ghostty, Terminal.app) | not bound. A local clear does nothing under herdr's re-rendering. A later `clear_screen` action should send `ctrl+l` to the pane |
| `ctrl+cmd+f` / `ctrl+cmd+q` / `ctrl+cmd+space` | full screen / lock screen / emoji | avoided |
| `alt+<letter>` | types `˙∆˚¬…` in text fields | in browser tiles `alt+h/j/k/l/w/o` won't type those characters, which is acceptable. Shells lose fish's `alt+h` (help), `alt+l` (list dir) and `alt+w` (whatis), as they already do under herdr |

**Browser-tile shortcuts:** `cmd+l` (address bar), `cmd+r` (reload), `cmd+[` / `cmd+]` (back/forward) and `cmd+f` (find) don't collide with anything above. `cmd+[` / `cmd+]` were deliberately not used for tab cycling; `ctrl+tab` covers that.

### Hammerspoon

If Hammerspoon owns a chord globally, it may intercept it before Azulejo. Exclude `dev.azulejo.Azulejo` from conflicting rules, or override the corresponding action in Azulejo's keymap.

For example, a global `alt+shift+h/j/k/l` window mover must stand aside for Azulejo if those chords should swap tiles instead.

## Decisions

Settled with the maintainer on Nov 3, 2026:

1. **Tall layout is in the MVP.** `split_auto`, `promote_tile`, `rotate_layout` and `equalize_layout` are P0 and implemented in the core (step 1.1b, `layout.split_auto`, `layout.promote`, `layout.rotate`, `layout.equalize`). F3's other mode, niri-style scrolling columns, stays P2.
2. **`cmd+a` toggles the sidebar only in terminal tiles**; it's select-all everywhere else.
3. **Linux uses Super by default** for the `cmd` chords, with the `ctrl+shift` layout documented as the alternative. **Every shortcut is configurable.**
