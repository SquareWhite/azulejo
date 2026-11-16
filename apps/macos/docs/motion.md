# macOS shell: motion and the focus dim

## Layout motion

Layout changes animate: split, close, move, swap, resize, zoom, tab and
workspace switches, and the sidebar opening or closing. A change that
arrives mid-motion starts from wherever every tile is on screen.

```toml
[animation]
enabled = true             # false: changes are instant
duration_ms = 200          # 0–1000
curve = "ease_out"         # "ease_out", "ease_in_out", "linear"
style = "reflow"           # "reflow", "crop", "scale", "outline"
tab_switch = "fade"        # "fade", "slide" (sideways), "none"
workspace_switch = "slide" # "slide", "fade", "none" (experimental)
workspace_switch_ms = 260  # a workspace slide's length, 0–1000; not tied to duration_ms
reduce_motion = "system"   # "system": no motion while macOS Reduce Motion is on; "ignore"
```

Changes apply when the config is saved.

### Styles

**`reflow`** (default). Content lays out live, like a window being
drag-resized. A terminal's grid runs 100 ms ahead of its tile: the PTY
resizes to the size the tile will have 100 ms later, at most every 33 ms and
only by whole cells, and reaches the final size before the motion ends. The
lead covers the redraw's trip through libghostty and herdr, whose attach
client checks its size only every 100 ms, so text lands with the frame. A
terminal born mid-motion starts at its final size. herdr sees no layout
change, so the mirror stays quiet. `AZJ_REFLOW_MS` and `AZJ_REFLOW_LEAD_MS`
(dev profile) or `anim reflow-ms` / `anim reflow-lead` (scripts) tune the two
intervals; `reflow-lag.sh` measures the lag.

**`crop`**. Every tile takes its final frame at once and Core Animation
carries it there. A terminal that changes size covers itself with a snapshot
of its last frame, resizes its PTY once, and shows the live surface again on
its first fresh frame at the new size (250 ms after the motion at the
latest). Content stays at 1:1 and the moving edge reveals or cuts it off.
Closing tiles and the ones Monocle hides also leave this way under `reflow`.

**`scale`**. A transform scales the whole tile, so text stretches mid-motion.

**`outline`**. Tiles jump to their new place hidden, outlines move instead,
and content fades in at the end.

### Workspace slides (experimental)

With `workspace_switch = "slide"`, a workspace switch moves the whole tile
area vertically in the sidebar's order (`SidebarView.ordered`): going down
the list, the old workspace leaves at the top and the new one comes in from
below. The sidebar and toolbar stay still. Nothing re-lays out: both
workspaces' tile views ride in one sheet moved by a single
`sublayerTransform` animation, so focus and typing land in the new workspace
at once. Checked by `e2e/wsslide.sh`.

## Focus: unfocused tiles dim

There's no outline round the focused tile. Every other tile on screen dims,
header included: the window colour laid over it at 25 %
(`alpha.unfocusedDim`, `color.dimFill` in `shared/design/tokens.json`).
Nothing dims with one tile on screen (a lone tile, Monocle, zoom). The dim
follows the focused tile, not the keyboard, so ⌘K or the Files panel leave
it bright. It cross-fades over 120 ms when focus moves.

Ghostty's `unfocused-split-opacity` (the dim is 1 minus that, so `1` turns it
off) and `unfocused-split-fill` win when Ghostty's config sets them. A dev
profile reads them from `.dev/<name>/config/ghostty/config` instead, so dev
runs never depend on your own Ghostty config. Checked by `e2e/dim.sh`.

## Measuring motion

`apps/macos/automation/anim.sh` runs a stress of 50+ motions: frame pacing
from the display link, PTY resizes per motion, and captures. `--slow` runs
1.5 s motions with mid-motion window captures, and `AZJ_ANIM_STYLE` picks one
style. In scripts, `anim stats` writes the numbers and `anim expect-clean
[N]` fails on more than N dropped frames or the wrong number of PTY resizes.
