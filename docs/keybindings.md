# Default keybindings

Azulejo's default keymap starts from the maintainer's herdr bindings (`~/.config/herdr/config.toml`), so the muscle memory carries over. The proposed defaults live in [`shared/config/keymap.default.toml`](../shared/config/keymap.default.toml), the file step 2.3 (F25) will load. Step 1.6 hard-codes the same table until then.

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

One more thing from the config: `tall.py` encodes a placement rule, kitty's `tall` layout. There's one master on the left and an evenly divided stack on the right, and new panes always append to the bottom of the stack. That's how the maintainer actually tiles day to day. PRD F3 (master-stack) is P2 today; see [Open decisions](#open-decisions).

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
| `promote_tile` | `alt+enter` | herdr `tall.py promote` |
| `rotate_layout` | `alt+o` | herdr `tall.py rotate` |
| `equalize_layout` | `alt+shift+e` | herdr `tall.py equalize` |
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
| `command_palette` | `cmd+shift+p` | new (Zed / VS Code convention) |
| `goto` (workspace, tab or tile picker) | `cmd+p` | new, covers `prefix+w` and `prefix+g` |
| `find` (scrollback search via herdr, ADR 0005; page find in browser) | `cmd+f` | new |
| `new_window` | `cmd+n` | macOS convention |
| `open_config` | `cmd+,` | macOS convention |

Rename, close workspace, kill the herdr pane, help and edit-scrollback are left to the palette and to prefix mode.

### Linux defaults

There's no Hammerspoon on Linux, and the window manager owns Super: GNOME uses `super+1..9`, `super+arrows` and more, and Hyprland and Sway put almost everything on Super. So Azulejo stays off Super. `cmd` maps to **`ctrl+shift`**, the convention Linux terminals use (kitty, GNOME Terminal, Ptyxis). `alt` chords stay as they are. The exceptions are where a straight translation would collide or break a desktop default:

| Azulejo action | Linux | Why it differs from a plain cmd→ctrl+shift swap |
| --- | --- | --- |
| tile actions (`alt+…`) | same as macOS | |
| `split_auto` | `ctrl+shift+enter` | `ctrl+alt+t` opens a terminal in Ubuntu; kitty uses `ctrl+shift+enter` for "new window" |
| `split_down` / `split_right` | `ctrl+shift+minus` / `ctrl+shift+backslash` | visual mnemonics, no 4-modifier chords |
| `new_workspace` / `new_tab` | `ctrl+shift+t` / `ctrl+alt+shift+t` | |
| `switch_workspace` 1–8, `last_workspace` | `alt+1..8`, `alt+9`/`alt+0` | `ctrl+shift+digit` yields `!@#` on many layouts, and `alt+N` is the Linux terminal convention |
| `switch_tab` 1–8, `last_tab` | `ctrl+1..8`, `ctrl+9` | |
| `close_tab` | `ctrl+shift+w` | |
| `toggle_sidebar` | `ctrl+shift+b` | `ctrl+shift+a` is select-all in GTK terminals |
| `command_palette` | `ctrl+shift+p` | |
| `goto` | unbound (palette) | `ctrl+p` is shell history |
| `find` | `ctrl+shift+f` | |

Match keys by physical key (hardware keycode), not by the shifted keyval. Otherwise `ctrl+shift+minus` arrives as `underscore` on GTK, and `alt+shift+h` arrives as `H`.

### Optional prefix mode

Off by default. When on, it mirrors herdr's prefix sequences exactly (`prefix+x`, `prefix+v`, `prefix+minus`, `prefix+1..9`, `prefix+shift+w`, …), so anything without a direct chord is still a two-key sequence away.

- **Prefix: `cmd+b` on macOS, `ctrl+shift+b` on Linux** (Linux then moves the sidebar toggle to the palette and prefix mode). It's the same letter as herdr's `ctrl+b`, but it never reaches the PTY. So it can't trip the attach client's `ctrl+b q` detach, and the shell keeps `ctrl+b` (backward-char).
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
| `cmd+a` | select all | conflicts in browser tiles, preview tiles and text fields (address bar, palette). `toggle_sidebar` is bound **only when a terminal tile is focused**; elsewhere `cmd+a` is select-all. Needs a `when` context in the keymap |
| `cmd+t` / `cmd+shift+t` | new tab / reopen closed tab | used for workspace / tab, as in herdr. No clash because browser tiles have no inner tabs |
| `cmd+p` | print | taken by `goto`. Printing a browser tile goes through the palette |
| `cmd+k` | clear (Ghostty, Terminal.app) | not bound. A local clear does nothing under herdr's re-rendering. A later `clear_screen` action should send `ctrl+l` to the pane |
| `ctrl+cmd+f` / `ctrl+cmd+q` / `ctrl+cmd+space` | full screen / lock screen / emoji | avoided |
| `alt+<letter>` | types `˙∆˚¬…` in text fields | in browser tiles `alt+h/j/k/l/w/o` won't type those characters, which is acceptable. Shells lose fish's `alt+h` (help), `alt+l` (list dir) and `alt+w` (whatis), as they already do under herdr |

**Browser-tile shortcuts:** `cmd+l` (address bar), `cmd+r` (reload), `cmd+[` / `cmd+]` (back/forward) and `cmd+f` (find) don't collide with anything above. `cmd+[` / `cmd+]` were deliberately not used for tab cycling; `ctrl+tab` covers that.

### Hammerspoon

If Hammerspoon owns a chord globally, it may intercept it before Azulejo. Exclude `dev.azulejo.Azulejo` from conflicting rules, or override the corresponding action in Azulejo's keymap.

For example, a global `alt+shift+h/j/k/l` window mover must stand aside for Azulejo if those chords should swap tiles instead.

## Open decisions

1. **Pull the "tall" placement rule into the MVP?** `split_auto`, `promote_tile`, `rotate_layout` and `equalize_layout` are how the maintainer tiles today. They're rules on the split tree rather than a full layout mode, so they're cheap: a few core ops in `model`. Proposal: implement them in M1 (1.1 / 1.6), and keep F3's other modes (scrolling columns) at P2.
2. **`cmd+a` for the sidebar:** keep it terminal-only as proposed, or move the sidebar to `cmd+b`? That would free `cmd+a` everywhere, with prefix mode moving to `cmd+shift+b`.
3. **Linux modifier:** `ctrl+shift` as proposed, or Super for users whose WM leaves it free (configurable, not default)?
