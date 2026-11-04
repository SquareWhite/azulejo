# 0010: Bismuth-style dynamic layouts per tab

Status: **accepted** · Nov 3, 2026 · M2 (PLAN 2.12; accepted by the maintainer on Nov 3, 2026) · research: [Bismuth notes](../research/bismuth.md)

## Context

the maintainer wants tiling "like in Bismuth". Bismuth (KDE Plasma, archived at v3.1.4) keeps an **ordered list** of windows per surface and a **layout rule** that turns the list into rectangles: Tile (master-stack), Monocle, Three Column, Spiral, Quarter, Spread, Stair, Floating. Inserting, swapping and promoting only reorder the list; resizing only changes the layout's parameters. See the research note for the source-level detail.

Azulejo today has a **binary split tree** per tab (`core/crates/model/src/layout.rs`). The tall ops (`split_auto`, `promote`, `rotate`, `equalize`; commit `a5ef71c`) read master-stack geometry out of any tree and rebuild a canonical shape. That covers the maintainer's `tall.py` habit with one master, but:

- the tree has to be rebuilt by hand-written ops for every rule, so a second master, a centred master or a monocle view each need new tree surgery;
- plain `layout.split` and `layout.move` (tmux-style, `8542156`) produce shapes that the tall ops then have to read back, and a stack that's been split by hand stops behaving as a stack (`equalize` skips it);
- there's no way to switch a tab between arrangements and back.

The split tree has to stay. Tabs adopted from herdr mirror herdr's own split geometry exactly (ADR 0008, 0009), layout files can describe arbitrary trees (2.4), and some people tile by hand.

## Decision

### 1. A tab has a layout kind; "manual" is one of them

```rust
pub struct Tab {
    // existing: id, name, root, tiles, focused, zoomed
    #[serde(default)]                 // old state files load as Manual
    pub layout: TabLayout,
}

pub struct TabLayout {
    pub kind: LayoutKind,             // Manual | Tile | Monocle | ThreeColumn
    pub previous: Option<LayoutKind>, // for toggle-back, as Bismuth's layout_store
    pub order: Vec<TileId>,           // authoritative for dynamic kinds
    pub weights: BTreeMap<TileId, f64>, // per-tile stack weight, default 1.0
    pub params: LayoutParams,         // every kind's parameters, remembered across switches
    pub manual: Option<Node>,         // last hand-made tree, restored when switching back
}

pub struct LayoutParams {
    pub tile: TileParams,             // masters 0..=10 (default 1), ratio 0.2..=0.8 (0.5),
                                      // orientation Left|Top|Right|Bottom, masters_axis
    pub three_column: ThreeColumnParams, // masters 1..=10 (1), ratio 0.2..=0.75 (0.6)
}
```

- **Manual:** `root` is the source of truth, exactly as today.
- **Dynamic kinds:** `order` + `params` + `weights` are the source of truth. `root` is still kept, **derived** after every op (see 4), so code that walks the tree (layout export, adoption, today's shells) keeps working while the shells move to rectangles.
- Parameters are per tab, the equivalent of Bismuth's per-surface layout store. A tab remembers Tile's master count while it's on Monocle.

### 2. Layouts are pure functions

```rust
fn arrange(kind, &params, &order, &weights) -> Vec<Placement>
struct Placement { tile: TileId, rect: Rect /* unit square */, z: u16, visible: bool }
```

- Same unit-square coordinates as `Node::rects()`. Gaps, gutters and the tab bar stay a shell concern, as today.
- `z` and `visible` exist for overlapping kinds. **Monocle:** every tile gets the full rect; the focused tile is visible and on top, the rest hidden. Switching focus in Monocle changes z-order only, so **no PTY resizes** (ADR 0004) and the 2.1 animation layer can cross-fade or slide.
- Core computes placements for **every** tab (manual ones via `root.rects()`) and sends them in `layout.changed` as `rects`. Shells render from `rects`; the animation code already animates frame → frame, so it needs no new model. `tree` stays in the event for manual tabs.

The kinds, ported from Bismuth's maths (the full set, as decided):

| Kind | Behaviour | Parameters |
| --- | --- | --- |
| `tile` | Masters on one side, stack on the other; both split by weight. Rotates through left/top/right/bottom. Masters can sit stacked or side by side ("rotate part") | masters 0–10 (1), ratio 0.2–0.8 (0.5) |
| `monocle` | All tiles full size, focused one shown; directional focus cycles the order | — |
| `three_column` | Masters centred; left and right stacks; right stack gets `floor((n−m)/2)`. Columns weigh `(1−ratio) : ratio : (1−ratio)`, as in Bismuth (amended Nov 4) | masters 1–10 (1), ratio 0.2–0.75 (0.6) |

Left out on purpose: **Spiral** and **Quarter** (dropped by the maintainer), **Spread** and **Stair** (overlapping cards hide most of each terminal), **Floating** (Azulejo has no floating tiles) and Cascade (not even wired up in Bismuth). The `z` field means they can be added later without a model change.

### 3. How existing operations behave in a dynamic tab

| Operation | Manual tab (today) | Dynamic tab |
| --- | --- | --- |
| New tile (`layout.split`, `split_auto`, `tab.create`, layout file, adopted herdr pane) | Split the focused tile / tall rule | Insert into `order` per `[layout] insert`: `end` (default; Bismuth and `tall.py`), `master`, or `after_focused`. The direction of `layout.split` is ignored and the reply says so. New tile takes focus |
| Close | Sibling takes the space | Remove from `order`; focus goes to the tile now in that position, else the previous one |
| Focus left/down/up/right | Geometric on rects | Same function over `rects`. Monocle: cycles the order (as Bismuth). Ties break by **most recently focused**, adopted from Bismuth for both modes |
| Focus next/previous (`cycle_tile_*`) | Tree order | `order` |
| Swap left/down/up/right | Tree swap with geometric neighbour | Swap the two tiles' positions in `order` (weights travel with tiles) |
| Move left/down/up/right | Tree restructure | Same as swap, as in Bismuth |
| Move to next/previous position (new) | — | Move one step in `order`; past either end, wrap round to the other end (Bismuth's `swapOrder`; amended Nov 4) |
| Resize left/down/up/right | Move nearest divider; flip to the opposite one at the edge | Same edge rule, translated into parameters: master ratio, stack weights, three-column ratio. Monocle ignores it. Matches Bismuth for tiles at an edge; see the Nov 4 note for tiles with neighbours on both sides |
| Promote (`alt+enter`) | Swap with master | Move to the front of `order` |
| Rotate (`alt+o`) | Master left ↔ top | Tile: Bismuth's clockwise quarter turn, left → top → right → bottom (amended Nov 4). Anticlockwise, "rotate part" and the old left ↔ top flip are separate actions |
| Equalize | Even out the stack | Reset all weights to 1.0 (ratio unchanged) |
| Zoom | Overlay | Unchanged; works in every kind |

### 4. Switching between manual and dynamic

- **Dynamic → manual** ("freeze"): build a split tree from the current placements with a guillotine cut (find a full-span line, split, recurse). Tile and Three Column are both guillotine layouts, so the frozen tree reproduces the rects exactly; this is also how the derived `root` is computed. Monocle freezes to the Tile arrangement of the same order. If `manual` holds a tree with the same tile set, switching back restores that instead.
- **Manual → dynamic:** `order` comes from the existing `tall()` reading (master first, then the stack in reading order). If the tree is in canonical tall shape, Tile's ratio and orientation come from it, so a tall tab converts with no visible jump.
- `layout.toggle {kind}` follows Bismuth: switch to `kind`, or back to `previous` if `kind` is already active. `layout.cycle` walks `[layout] cycle`.

### 5. herdr adoption

- Tabs adopted from herdr (ADR 0008) open as **manual**: their geometry mirrors herdr exactly, which a dynamic layout can't promise.
- Panes later split off in herdr (ADR 0009): manual tab → placed beside the sibling, as today; dynamic tab → inserted per `[layout] insert`.
- Nothing here changes herdr's own layout. Azulejo still never calls `layout.apply` on live tabs (ADR 0002).

### 6. Layout files (2.4)

- `layout = "tile" | "monocle" | "three_column"` makes a dynamic tab, with `masters`, `ratio` and `orientation` where they apply. Tile order is the order of `[[tile]]` entries.
- `layout = "tall"` becomes an alias for `tile` with one master. It already builds the same shape.
- `columns`, `rows` and an explicit `root` stay manual.
- `layout.export` writes a dynamic tab as its kind, parameters and tile order.

### 7. Default for new tabs: dynamic Tile

`[layout] default = "tile"` (or `"manual"`). Reasons:

- It's how the maintainer tiles every day (`tall.py`: one master left, even stack right, new panes appended), and Tile is Bismuth's first layout.
- Ratios become parameters, so they survive inserts and closes instead of drifting with each split.
- `alt+enter`, `alt+o`, `alt+shift+e` and the split chord keep doing what they do now; the only visible change is that a plain split no longer cuts the focused tile in two.
- Manual stays one keypress away, and adopted herdr tabs stay manual.

### 8. Protocol and config

- **Commands:** `layout.set {tab?, kind}`, `layout.toggle {tab?, kind}`, `layout.cycle {tab?, step}`, `layout.masters {tab?, delta}`, `layout.master_ratio {tab?, delta}`, `layout.move_order {tile?, step}`, and `layout.rotate {tab?, reverse?, part?, flip?}` extending today's (`flip` added Nov 4). They reach the palette and CLI through the schema, as everything else does.
- **Event:** `layout.changed` gains `kind`, `params`, `order` and `rects`.
- **Config:**

  ```toml
  [layout]
  default = "tile"            # or "manual"
  insert = "end"              # "end" | "master" | "after_focused"
  cycle = ["tile", "monocle", "three_column"]
  [layout.tile]
  masters = 1
  ratio = 0.5
  [layout.three_column]
  ratio = 0.6
  ```

- **Persistence:** additive. Old `state.json` files have no `layout` field, load as manual, and keep their exact shapes. No format version bump.

### 9. Invariants (proptest)

- For every dynamic kind, n = 1–20 tiles and random parameters, `arrange` places every tile exactly once, inside the unit square. For guillotine kinds the visible rects don't overlap and cover the square.
- `arrange` is deterministic, and parameters stay clamped after any sequence of ops.
- `order` is always a permutation of the tab's tiles; `weights` has no stray keys.
- Freezing a guillotine kind gives a tree whose `rects()` equal the placements within EPS.
- Converting a canonical tall tree to Tile and back gives the same tree.
- Swapping in direction *d* and then the opposite direction restores `order` when the neighbours are mutual.
- Random op sequences that mix kinds and switches keep every existing session invariant (tile in exactly one tab, focus and zoom valid).

## Proposed keys

These follow the maintainer's `alt` layer (herdr binds, `tall.py`) and map Bismuth's `Meta` chords where they're free. Nothing is added to the keymap files until this ADR is accepted.

| Action | Proposed | Bismuth | Notes |
| --- | --- | --- | --- |
| Next / previous layout | `alt+]` / `alt+[`, and `alt+\` / `alt+shift+\` | `Meta+\` / `Meta+\|` | `alt+]` / `alt+[` added Nov 4; `alt+\` types `«` in browser text fields |
| Toggle Tile | `alt+t` | `Meta+T` | bash/zsh lose `alt+t` (transpose words) inside tiles |
| Toggle Monocle | `alt+z` | `Meta+M` | `alt+m` belongs to Hammerspoon, `alt+shift+m` is zoom |
| Toggle manual (freeze / restore) | `alt+shift+t` | — | |
| More / fewer masters | `alt+=` / `alt+-` | `Meta+]` / `Meta+[` | `alt+]` / `alt+[` switch layouts |
| Bigger / smaller master area | `alt+shift+=` / `alt+shift+-` | unbound | Resizing the master with `ctrl+alt+shift+h/l` does the same |
| Move to next / previous position | `alt+shift+]` / `alt+shift+[` | unbound | |
| Rotate part (masters side by side) | `alt+shift+o` | `Meta+Shift+R` | |
| Push to master | `alt+enter` (existing) | `Meta+Return` | unchanged |
| Rotate | `alt+o` (four-way clockwise since Nov 4) | `Meta+R` (four-way) | Anticlockwise (`rotate_anticlockwise`) and the old left ↔ top flip (`flip_layout`) go to the palette |
| Focus / swap / resize | existing `alt+hjkl`, `alt+shift+hjkl`, `ctrl+alt+shift+hjkl` | `Meta+…` | Same behaviour, with the differences in the Nov 4 note |
| Next / previous tile | unbound (was `alt+]` / `alt+[`) | unbound | Palette and prefix mode (`prefix+tab`) |

Amended Nov 4, 2026: `alt+o` is the four-way rotate, `alt+]` / `alt+[` switch layouts, and `cycle_tile_next` / `cycle_tile_previous` lost their chords. See [the Nov 4 note](#bismuth-check-nov-4-2026).

Checked against `docs/keybindings.md`: none of these are owned by Hammerspoon (`alt+1..9`, `alt+m`, `alt+q`, `alt+,`, `alt+n`, `alt+f1..f3`) or already bound in `shared/config/keymap.default.toml`. Avoided on purpose: `alt+f`, `alt+b` and `alt+d`, which shells use for word movement. Linux uses the same chords; GNOME's `alt+space` window menu is why the layout cycle isn't on `alt+space`.

## Consequences

- One more concept per tab, but it replaces special cases: the tall ops become thin aliases in dynamic tabs, and their tree versions stay only for manual tabs.
- Shells switch from rendering the tree to rendering `rects`. That's a real change in `TileContainerView` (and the GTK shell later), and it's what makes overlapping layouts and the Monocle no-resize focus switch possible.
- A plain split behaves differently in a dynamic tab (the layout decides where the tile goes). The reply says so, and `alt+shift+t` gets the old behaviour back for that tab.
- Bismuth is archived, so there's no upstream to track; we port its maths once and own it.

## Implementation notes (2.12)

- The macOS shell rebuilds from `session.get`, which carries the tab's tree and `layout` but not `rects`. For Tile and Three Column it draws the derived tree, which reproduces the rects exactly (a model invariant checks this after every op). For Monocle it gives every tile the full tab and shows only the focused one, so a focus switch restacks without resizing a terminal. `layout.changed` still carries `rects` for API clients; an overlapping kind added later would move the shell onto them.
- Directional focus now breaks ties by most recently focused in manual tabs too, as decided above. In a tall layout, focusing right from the master lands on the stack tile used last rather than the middle one.
- `layout = "tall"` with a `ratio` outside 0.2–0.8 is now an error in layout files; older files with such ratios need `layout = "rows"`/`"columns"` or an explicit `root`.

## Decisions (Nov 3, 2026)

the maintainer answered the open questions:

1. **Default for new tabs:** dynamic Tile. `[layout] default = "tile"`.
2. **Adopted herdr tabs:** stay manual, so they match herdr's own layout exactly.
3. **Layout set:** Tile, Monocle and Three Column only. Spiral and Quarter are dropped; Spread, Stair, Floating and Cascade aren't planned. The `z` field still leaves room for more later.

## Bismuth check (Nov 4, 2026)

Every default layout chord was run end to end through the key router (`apps/macos/automation/bismuth.azs`, `just mac-bismuth`), with the order, parameters and rects traced after each key. The results were compared with Bismuth's source. The table and every difference are in [docs/keybindings.md, "Bismuth mapping"](../keybindings.md#bismuth-mapping). What changed:

- **`alt+o` is Bismuth's rotate** (`Meta+R`): a clockwise quarter turn of a Tile tab, left → top → right → bottom. `layout.rotate` with no params now turns clockwise; `reverse: true` turns back; `flip: true` is the old left ↔ top toggle, kept as the `flip_layout` action with no chord. A manual tab flips its tree either way: a split tree's tall reading always puts the master first, so a master on the right or at the bottom has no tree to go to. `rotate_clockwise` is gone, and the name still binds `rotate_layout` with a warning.
- **`alt+]` / `alt+[` switch layouts** (`cycle_layout_next` / `previous`). `alt+\` / `alt+shift+\`, Bismuth's own chords, stay as second bindings, on Linux too. `cycle_tile_next` / `previous` are unbound, as Bismuth's focus next/previous are: no `alt` chord is free that fits, since Hammerspoon owns `alt+tab` and shells use `alt+.`. They stay in quick open and prefix mode.
- **Fixed to match Bismuth:**
  - Moving past the end of the order wraps the tile round to the other end, with the rest moving up. It used to swap the two ends.
  - Three Column weighs its columns `(1−ratio) : ratio : (1−ratio)`, so the default 0.6 gives 2/7, 3/7, 2/7. It used to give 0.2, 0.6, 0.2. Edge moves change the ratio by the move × (2 − ratio), as in Bismuth's `adjust`.
  - Resizing Three Column's centre: `l` grows it and `h` shrinks it. Both used to grow it, since its two edges move together.
  - With the master area on the right or at the bottom, side-by-side masters (rotate part) are mirrored, so the first master keeps the outer edge.
- **Kept on purpose:** resizing moves 5 % (Bismuth 3 %), the same step as manual tabs. A tile with neighbours on both sides moves the edge on the pressed side (herdr's rule, as in manual tabs), where Bismuth always moves its east or south edge. Swap in Monocle trades places in the order (a no-op in Bismuth). Close picks the next focus. Master count is on `alt+=` / `alt+-`.
- The 10,000-sequence proptest gives the four-way turn more weight, and a new property checks that four turns (or one turn and one back) restore the arrangement exactly, with the masters on the expected side after each turn.
