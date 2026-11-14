# Bismuth: how its tiling works

Research for [ADR 0010](../adr/0010-dynamic-layouts.md). the maintainer wants tiling "like in Bismuth", the KDE Plasma tiling script. This note records what Bismuth actually does, read from its source.

**Source:** [Bismuth-Forge/bismuth](https://github.com/Bismuth-Forge/bismuth) at `ef69afe` (Dec 22, 2022); last release v3.1.4 (Sep 23, 2022). The repository is **archived**, so this behaviour is frozen. Paths below are relative to `src/kwinscript/` unless stated; link base: `https://github.com/Bismuth-Forge/bismuth/blob/ef69afe/src/kwinscript/`.

## The model in one paragraph

Bismuth keeps **one ordered list of windows** per desktop and **one layout per "surface"** (screen × virtual desktop × activity). A layout is a pure arrangement rule: given the ordered list of tileable windows and the working area, it assigns each window a rectangle (`apply`). There is no split tree. Inserting, swapping and "moving" windows only reorder the list; resizing only changes the layout's parameters (master ratio, per-window stack weights). Every change re-runs `apply` on the whole list (`engine/index.ts` `arrangeScreen`).

## Layouts

Seven layouts can be enabled; the cycle order is fixed in `src/core/ts-proxy.cpp:46-52`: Tile, Monocle, Three Column, Spread, Stair, Spiral, Quarter. Floating is a separate toggle. Defaults from `src/config/bismuth_config.kcfg`: Tile, Monocle, Three Column, Spiral, Spread and Stair on; Quarter and Floating off. A Cascade layout exists in code (`engine/layout/cascade_layout.ts`) but isn't wired into the layout order.

### Tile (master-stack) — `engine/layout/tile_layout.ts`, `engine/layout/layout_part.ts`

- Built from composable parts: `Rotate(HalfSplit(Rotate(Stack), Stack))`. The half-split's primary side holds the **masters**, the secondary side the **stack**.
- **Master count** (`numMaster`, the half-split's `primarySize`): default 1, range 0–10. With 0, every window is in the stack; with `n ≤ masters`, the master area takes the whole screen.
- **Master ratio**: default 0.5, clamped to 0.2–0.8, changed in steps of 0.05 (`IncreaseLayoutMasterAreaSize` / `Decrease…`).
- Masters and stack windows are each split evenly along the stack axis, **weighted per window** (`StackLayoutPart`, `layout_utils.ts` `splitAreaWeighted`). A window's weight defaults to 1.0 and is stored per surface (`engine/window.ts:287-299`).
- **Rotate** (`Meta+R`) turns the whole layout by 90° per press: master left → top → right → bottom (`RotateLayoutPart`, angles 0/90/180/270). **Rotate part** (`Meta+Shift+R`) turns only the master area, so several masters sit side by side instead of stacked.

### Monocle — `engine/layout/monocle_layout.ts`

- Every window gets the whole area; with `monocleMaximize` (default on) they're also marked maximised.
- All focus actions, directional or not, become "focus next/previous in the list" (`focusOrder(±1)`).
- Option `monocleMinimizeRest` (default off) minimises the others.

### Three Column — `engine/layout/three_column_layout.ts`

- Masters in the **centre** column; master ratio default **0.6**, clamped 0.2–0.75; master count 1–10.
- `n ≤ masters`: one column. `n = masters + 1`: master left, the single stack window right. Otherwise left stack | masters | right stack, with the right stack holding `floor((n − masters) / 2)` windows and the left stack the rest. List order is masters, then right stack, then left stack.
- Resizing adjusts the master ratio and per-window weights within the column being resized.

### Spiral — `engine/layout/spiral_layout.ts`

- Fibonacci-style: each window takes half of the remaining area, turning clockwise (split angles cycle 0°, 90°, 180°, 270°). Each half-split's ratio can be adjusted by resizing.

### Quarter — `engine/layout/quarter_layout.ts`

- Capacity **4**. 1 window: full; 2: left | right; 3: left | (right-top / right-bottom); 4: a 2×2 grid filled clockwise from top-left (top-left, top-right, bottom-right, bottom-left).
- Three adjustable splits (vertical, left horizontal, right horizontal), clamped to 0.2–0.8.
- Windows beyond the fourth are set **afloat** (`TiledAfloat`).

### Spread — `engine/layout/spread_layout.ts`

- Overlapping full-height "cards" fanned horizontally; each card is offset by `space` (default 7% of the width, 4–10%, steps of 1% via the master-count actions). Cards never get narrower than 40% of the area; windows that would be narrower pile up at the left.

### Stair — `engine/layout/stair_layout.ts`

- Overlapping windows offset diagonally by `space` pixels (default 24, 16–160, steps of 8 via the master-count actions).

### Floating — `engine/layout/floating_layout.ts`

- Turns tiling off for the surface: every window is left floating. Per-window floating is a separate toggle (`Meta+F`).

## Engine behaviour — `engine/index.ts`, `engine/window_store.ts`

- **Insertion** (`manage`): a new window is **appended** to the end of the list, or prepended (becomes master) when `newWindowAsMaster` is on; that option defaults to **off**. Bismuth doesn't move focus itself; KWin focuses the new window.
- **Close** (`unmanage`): the window is removed from the list and the layout re-applied, so the rest close up in order. Focus goes wherever KWin sends it.
- **Focus by direction** (`focusDir`, `getNeighborByDirection`): purely geometric over the current rectangles. Candidates are windows beyond the current one in that direction whose span overlaps it; the nearest edge wins (with a 5 px tolerance), and ties go to the **most recently focused** window (`timestamp`).
- **Focus next/previous** (`focusOrder`): walks the list, wrapping.
- **Swap by direction** (`swapDirection`, bound to `Meta+Shift+H/J/K/L` "Move Window …"): finds the geometric neighbour as above and **swaps the two windows' positions in the list**. Weights travel with the windows. Floating windows are nudged instead.
- **Move to next/previous position** (`swapOrder`): moves the window before its neighbour in the list (wrapping). Unbound by default.
- **Push to master** (`setMaster`, `Meta+Return`): moves the window to the front of the list.
- **Resize** (`resizeTile`): `Meta+Ctrl+H/J/K/L` resize by 3% of the area. If the window has no neighbour on its east (south) side, the request flips to its west (north) side, so the shared edge always moves. The layout turns the edge move into parameter changes (`adjust`): master ratio, stack weights, Three Column ratios, Quarter splits. Layouts without `adjust` (Monocle, Spread, Stair) ignore resizing.
- **Layout switching**: `Meta+\` / `Meta+|` cycle through the enabled layouts; per-layout toggles (`Meta+T` Tile, `Meta+M` Monocle, `Meta+Shift+F` Floating, the rest unbound) switch to that layout, or **back to the previous one** if it's already active (`layout_store.ts` `toggleLayout`). A notification shows the layout name and its hint (e.g. the master count).
- **Per-surface state** (`layout_store.ts`): each surface keeps its own instance of every layout it has used, so a surface remembers, say, Tile's master count and ratio while it's on Monocle. With `layoutPerDesktop` / `layoutPerActivity` (both default on) a surface is screen × desktop × activity.
- **Gaps**: screen-edge gaps and an inter-tile gap (`tileLayoutGap`), all default 0 (`bismuth_config.kcfg`).
- **Other options**: `maximizeSoleTile` (default off), `limitTileWidthRatio` (cap a tile's width at 1.6 × screen height, off by default; not applied to Monocle).

## Default shortcuts — `controller/action.ts`

| Action | Default |
| --- | --- |
| Focus left / down / up / right | `Meta+H/J/K/L` |
| Focus next / previous | unbound |
| Move (swap) left / down / up / right | `Meta+Shift+H/J/K/L` |
| Move to next / previous position | unbound |
| Increase / decrease width | `Meta+Ctrl+L` / `Meta+Ctrl+H` |
| Increase / decrease height | `Meta+Ctrl+J` / `Meta+Ctrl+K` |
| Increase / decrease master count | `Meta+]` / `Meta+[` |
| Increase / decrease master area size | unbound |
| Push to master | `Meta+Return` |
| Toggle window floating | `Meta+F` |
| Next / previous layout | `Meta+\` / `Meta+|` |
| Toggle Tile / Monocle / Floating layout | `Meta+T` / `Meta+M` / `Meta+Shift+F` |
| Toggle Three Column / Spread / Stair / Spiral / Quarter | unbound |
| Rotate / rotate reverse / rotate part | `Meta+R` / unbound / `Meta+Shift+R` |

## How it compares with Azulejo today

| | Bismuth | Azulejo (Nov 3, 2026) |
| --- | --- | --- |
| Structure | Ordered list + layout rule per surface | Binary split tree per tab (`core/crates/model/src/layout.rs`) |
| Master-stack | Tile layout; any number of masters; ratio 0.2–0.8 | Tall ops read the tree geometrically (`tall()`), one master (`split_auto`, `promote`, `rotate`, `equalize`; commit `a5ef71c`) |
| New tile | Appended to the list | `split_auto` appends to the stack; plain `split` splits the focused tile |
| Swap | List swap with the geometric neighbour | Tree swap with the geometric neighbour |
| Move | Same as swap for tiled windows | Tree restructure, minimal when the neighbour is a sibling (`8542156`) |
| Resize | Edge move → layout parameters; flips to the far edge at the screen edge | Moves the nearest divider; flips to the opposite divider (`8542156`) — same rule |
| Focus by direction | Geometric, ties by most recently used | Geometric, ties by overlap then centre distance |
| Other layouts | Monocle, Three Column, Spiral, Quarter, Spread, Stair, Floating | Zoom (temporary full-tile), no other layouts |
| State scope | Per screen × desktop × activity | Per tab |
| Arbitrary shapes | Not possible | Any split tree, which herdr adoption needs (ADR 0008) |

The behaviours the maintainer already uses map one-to-one: `tall.py` is Bismuth's Tile layout with one master and `newWindowAsMaster` off, and his `alt+enter` promote is Bismuth's push-to-master. What Bismuth adds is the **layout as a rule** (any tile count, recomputed on every change), more masters, the other layouts, and cycling between them.
