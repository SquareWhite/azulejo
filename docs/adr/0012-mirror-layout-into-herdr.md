# 0012: Mirror Azulejo's layout into herdr

Status: accepted · Nov 4, 2026 · M2 (PLAN 2.16) · supersedes [0002](0002-herdr-mapping.md)'s tab-per-tile rule · amends [0008](0008-use-the-users-herdr-session.md), [0009](0009-changes-made-in-herdr.md), [0010](0010-dynamic-layouts.md) · amended Nov 4, 2026: only Azulejo changes herdr's structure (see "Amendment")

> **Invariant (amendment, Nov 4, 2026): only Azulejo changes herdr's structure.** A push may swap panes or rebuild a herdr tab into another shape only to carry out an Azulejo op (a user op or a command in the app). A mismatch Azulejo didn't cause (adoption, splits and closes made in herdr, a reconnect, a relaunch) resolves as a pull, never a push: the tab follows herdr. Where the text below says a push "settles", "rearranges" or "wins" after a change made in herdr, the amendment at the end replaces it.

## Context

the maintainer sees a mismatch. In the herdr TUI, workspace "azulejo" has tabs 1, 2 and 3 with one pane each. In Azulejo the same three terminals are splits of one tab. ADR 0002 gave every terminal tile its own herdr tab, so herdr, the TUI and the phone never see Azulejo's layout. He decided to mirror splits: one Azulejo tab is one herdr tab, each terminal tile is a pane in it, and herdr's split tree matches Azulejo's arrangement. Changes made in the TUI or on the phone come back.

ADR 0002 rejected mirroring for three reasons. Re-probed on herdr 0.9.1 (Nov 4, 2026, throwaway sessions, `docs/herdr-notes.md` question 10):

- **`layout.apply` still kills.** It builds a new tab with fresh panes and closes the old one, even when every `pane_id` is named. Still never used.
- **But herdr can now be steered pane by pane**, without `layout.apply` and without killing anything:
  - `pane.split` takes `target_pane_id`, `direction` (`right` or `down`), `ratio` (the first child's share), `cwd` and `env`. The new pane is always the second child.
  - `layout.set_split_ratio` sets one split by path (`[]` is the root, `true` is the second child) and replies with the whole tree.
  - `pane.swap` swaps two panes in a tab; the ratios stay with the slots.
  - `pane.move` moves a pane into another tab (split beside a target pane, with a ratio) or into a new tab, in the same or another workspace. The process, the terminal ID and an attached `herdr terminal attach` client all survive: a ticker kept counting, the attach client kept streaming at its own size. A move inside the same tab is refused (`changed: false, reason: same_tab`), and so is a move out of a zoomed tab (`reason: zoomed_tab`). herdr closes a tab when its last pane leaves.
  - `pane.zoom` (`on`, `off`, `toggle`) zooms a pane and focuses it. While zoomed, herdr shows the focused pane; zooming another pane only moves focus. A split clears the zoom.
  - `layout.export` gives the tree; `layout_updated` fires on every split, close, move, swap, ratio change, zoom and focus change.
  - Ratios are stored as `f32` (1/3 reads back as 0.33333334). A herdr restart keeps the tree, the ratios, the zoom and the pane IDs, with fresh processes.
- **Pane size still comes from the attached client.** That stays true: mirroring gives the TUI and the phone the same arrangement, not the Mac's sizes.

## Decision

### Mapping

- **One Azulejo tab is one herdr tab per machine** it has terminals on (normally one, its workspace's machine; ADR 0011). Each terminal tile is a pane in it.
- **The tree herdr gets** is the tab's `root` with every leaf that isn't a live pane on that machine removed: browser, preview and files tiles, terminals whose pane is still being made, ended terminals. A removed leaf's split collapses, so **its space goes to its sibling** in herdr's view. A manual tab's `root` is its hand-made tree; a dynamic tab's `root` is already its frozen guillotine tree (Tile and Three Column reproduce their rects exactly, ADR 0010 §4), so that's what herdr gets.
- **Zoom:** a zoomed tile zooms its pane. **Monocle** zooms the focused tile's pane (herdr shows the focused pane while zoomed), over the Tile tree that Monocle's `root` already is, so unzooming in the TUI shows the Tile arrangement. herdr's focus is otherwise left alone: the TUI keeps its own. herdr doesn't zoom a tab's only pane (`single_pane`), so a tab with one terminal and a browser has no zoom in herdr. When Monocle's focused tile has no pane (a browser, a preview), herdr stays zoomed on the terminal focused last.
- **New terminals:** the first terminal of a tab still makes a herdr tab (`tab.create`, labelled with the tile's title, ADR 0002's env vars). Every later one is `pane.split` of the pane next to it in the target tree, with direction, ratio, cwd and the `AZULEJO_*` env, so it's born in the right place. If that pane is gone, it falls back to `tab.create` and the next push moves it in.

### Direction of truth

Azulejo drives the layout of every tab it shows; herdr's changes come back as operations on Azulejo's model.

- **Push (Azulejo → herdr).** After every change the core recomputes each tab's target tree. A target that changed goes to the machine's mirror task, which waits until that tab has been **quiet for 150 ms** and then pushes once: it reads `layout.export` and makes the smallest change that gets there:
  - same shape and panes: `layout.set_split_ratio` for each split more than 0.005 off;
  - same shape, panes in other places: `pane.swap`s, then ratios;
  - anything else (a pane from another tab, a different shape, a turn of the layout): a rebuild. Everything but the anchor (the target's first pane) moves out to one staging tab, then each pane moves back in beside its neighbour with the right direction and ratio. 2(n−1) moves, no process touched; herdr deletes the staging tab when it empties;
  - zoom last, since moves refuse a zoomed tab and splits clear zoom.
- **Pull (herdr → Azulejo).** The core subscribes to `layout_updated`. Per herdr tab, 100 ms after the last one it reads `layout.export` and compares it with **the tree it recorded after its own last push or pull**:
  - equal: it was our own write coming back; nothing happens;
  - same panes, different tree or zoom: someone changed it in herdr, so the core absorbs it (below);
  - different panes: a split or a close, which ADR 0009 already turns into tiles; the record is updated. ~~The tab's next push settles the arrangement.~~ The tab takes herdr's arrangement of those panes (amendment).
- **Absorbing a herdr change.** herdr's panes map to tiles. With the same shape as Azulejo's pruned tree, ratios carry over to the matching splits of the full tree and moved panes become swapped tiles. With a different shape, herdr's tree becomes the tab's tree and the tiles herdr doesn't have (browsers, other machines) go back beside their old neighbour. Then by kind:
  - **manual:** take it;
  - **Tile, Three Column:** fit order, ratio and weights to the new rectangles. If the layout rule reproduces them within 0.5 %, the tab absorbs it (a reorder, a master ratio, stack weights). Otherwise the tab **freezes to manual** with herdr's tree, remembering its kind for toggling back;
  - **Monocle:** a reorder is absorbed; ratio changes are dropped (Monocle has none, and the next push puts them back); unzooming in herdr leaves Monocle for the previous kind (Tile if none); zooming another pane focuses that tile;
  - **zoom** on or off in herdr zooms or unzooms the tile.
- **No echo loops.** Push and pull for a machine run in one task, which records herdr's tree after every push and pull. Our own writes come back as `layout_updated`, the pull finds the recorded tree and stops. An absorbed change makes Azulejo's target equal herdr's tree within the tolerances, so the push that follows writes nothing. If Azulejo can't express the change exactly (a clamped ratio), one push corrects herdr and the next pull is an echo. It settles in one round.
- **Changes racing our writes** (amended Nov 4, 2026, after the merge). The record is what herdr answered to our last write, never what we meant to write: a push records the reply to its zoom (a refused zoom included), and the worker tells the task about every split and every new tab it makes. A pull compares herdr with the record over the panes both have: a ratio, swap or zoom changed among them is a change, whatever panes came or went meanwhile. A change whose panes don't match the tab's yet (a split on its way) waits and is read again every 150 ms, up to 2 s, and the push for that tab waits with it. Before a push the task reads the tab; a change found there goes first.
- **Waiting.** A tab doesn't push while one of its terminals is still getting its pane. A herdr tab holding a pane nobody here knows yet (split in the TUI, adoption pending) waits and retries; after 2 s that pane is moved out to its own herdr tab, never closed (amended: only by a push carrying out an Azulejo op; otherwise the push keeps waiting for the adoption).

### Sizes

- Azulejo keeps ratios in the unit square and the shell turns them into points; herdr keeps `f32` ratios and lays out whole cells. Azulejo pushes its ratio as is when it's more than **0.005** off herdr's (half a cell at 100 columns), and herdr rounds it to cells. A pulled ratio is taken as herdr reports it, clamped to Azulejo's range (5–95 % per split; Tile's master 20–80 %). The echo check allows 1e-4 for the `f32` rounding.
- The TUI and the GUI differ in exact sizes, and that's accepted. While a tile is attached its pane has the Mac's size (ADR 0003, 0008), so the TUI shows the same arrangement with each pane cropped or padded to the Mac's tile size.

### Migration

- There's no separate migration step: the first push of each tab merges it. Its herdr tab is the one holding most of its panes (a tie goes to the tab of its first terminal). Every other pane `pane.move`s in beside its neighbour. The processes, terminal IDs and attach clients stay; herdr closes the emptied one-pane tabs. Pushes start only after the machine is connected, its bound tiles are checked and its first survey is in, so a pane that died while Azulejo was away is rebound or ended first.
- Each herdr tab belongs to at most one Azulejo tab. If two Azulejo tabs share one (an old mixed adopted tab, ADR 0008), the one with more panes there keeps it and the other's panes move out to a new herdr tab.
- A pane in a merged tab that no tile shows any more (closed here, so detached) is moved out to its own herdr tab, as ADR 0002 kept it. **No pane is ever closed**: the only herdr calls that end a process stay `pane.close` on an explicit kill (F14), and `layout.apply` is never called.
- herdr reports the `tab_closed` of an emptied tab **before** the `pane_moved` that emptied it (seen in the migration test). ADR 0009 closed the tiles of a closed tab on the spot, which would have closed every merged tile (and already did for a pane moved between tabs in the TUI). Now a closed tab only asks the core to check its tiles' panes: a pane that's gone closes its tile, a moved one rebinds it (`Effect::ConfirmClosed`, one `pane.list`).
- With `mirror_layout = false` nothing moves and new terminals get their own herdr tab, as before. Turning it off doesn't split merged tabs again.

### Adoption, closing, remote machines

- **Adopted tabs** (ADR 0008) already mirror herdr's tree, so their first push writes nothing (or only a ratio Azulejo clamped). They stay manual (ADR 0010 §5) and now follow herdr both ways.
- **Panes split off in herdr** (ADR 0009) land beside their sibling in a manual tab, which matches herdr exactly. ~~In a dynamic tab they're inserted per `[layout] insert`, and the push rearranges herdr to match.~~ Amended: the tab then takes herdr's tree from the survey, so a dynamic tab absorbs it or freezes to manual (`Session::apply_tree`), like any change made in herdr.
- **Closing a tile** still detaches it (F14). Its pane moves out of the tab to a herdr tab of its own, unless it was the tab's last terminal (then the herdr tab simply stops being mirrored). Closing in herdr closes the tile (ADR 0009); the herdr tree has collapsed already, so the push only fixes ratios a dynamic layout rearranged.
- **Remote machines** (ADR 0011) behave the same: each machine's link has its own mirror task, and a tab's target for machine M only holds M's panes.

### Config

```toml
[herdr]
mirror_layout = true   # false: one herdr tab per terminal tile, herdr's tree left alone
```

It applies live on reload (unlike the rest of `[herdr]`). Off means no pushes, no pulls, and new terminals get their own tab.

## Consequences

- The TUI and the phone show Azulejo's tabs as herdr tabs with the same splits, zoom and Monocle. Splitting, closing, swapping, resizing and zooming in the TUI change Azulejo.
- ADR 0008's "read-only" rule narrows: adoption and restore still only read, but the mirror changes the layout of herdr tabs Azulejo shows, adopted ones included. It never calls `layout.apply` and never closes a pane except on an explicit kill.
- A structural change (a layout switch, a Tile rotation, a promote that changes the shape) shows a staging tab in the TUI's tab bar for the length of the rebuild, 3–4 ms for three panes ([measurements](../measurements.md#layout-mirror-pushes-adr-0012)). Ratio changes and swaps don't.
- **Two clients resizing** at once: the last write wins. A TUI drag during an Azulejo push can be overwritten by the push; the TUI's next drag comes back as usual.
- **The phone** below `mobile_width_threshold` (64 columns) draws herdr's single-column mobile layout, whatever the tree. Mirroring changes what it shows above that width, and the tab list (one per Azulejo tab rather than per tile).
- **Animations** don't cause a storm. The model changes once per command and the shell only animates between states. A held resize key or a divider drag sends many commands, but the 150 ms quiet period collapses them into one push of the settled layout.
- herdr's tab labels no longer name each terminal: a merged tab keeps the label of the tab it was built from.
- **Changes made in herdr while Azulejo isn't running** have nothing to be compared with, so at the next start Azulejo's saved ratios win. ~~And order.~~ Amended: a different order or shape is herdr's, and the tab follows it. Panes split or closed meanwhile still come in or go (ADR 0008, 0009).
- `AZJ_DEBUG_MIRROR=1` logs every push, every pulled change and the herdr events the core reacts to.

## Evidence

- `docs/herdr-notes.md` question 10, probed with `scripts/herdr-probe/mirror.py` in a throwaway session.
- `core/crates/herdr/src/mirror.rs`: the push planner, with a simulated herdr that refuses what herdr refuses (same-tab and zoomed moves), and a property test that random trees spread over random tabs always end as the target tree without losing a pane.
- `core/crates/model`: fitting Tile and Three Column to rectangles (`dynamic::fit`), with a property test that `fit(arrange(x))` reproduces `arrange(x)`.
- `core/crates/engine/src/mirror.rs`: targets, absorbing herdr's changes, primary-tab choice; unit tests.
- `core/crates/engine/tests/live_mirror.rs`: against real herdr in throwaway sessions (`just test-herdr`).

## Amendment (Nov 4, 2026): only Azulejo changes herdr's structure

### What went wrong

Four panes split in herdr in a quick burst (herdr-grid.sh without its pauses), while the app was busy, were adopted in the wrong shape: a 2×2 grid came in as master on top over three across. The mirror then pushed that shape back into herdr and rebuilt the user's own grid. Spacing the splits 0.5 s apart hid it.

- **Adoption guessed.** Each `pane_created` asks for a survey, and surveys coalesce, so a busy core sees a burst half made: the first pane alone, then all four. ADR 0009 places every new pane beside its sibling in herdr's tree, but in a burst that sibling is often a pane that isn't here yet either (in the grid, the two right-hand panes are each other's siblings). Those fell back to the tab's stack (`split_auto`), and the tab was built in that shape. Separately, `layout.export` is read after the snapshot, so a pane split off in between made the tree disagree with the panes and adoption laid them side by side.
- **The mirror trusted the guess.** An adopted tab's first record was Azulejo's tree; the push saw a different shape in herdr and rebuilt it.

Measured with `live_mirror.rs` `bursts_of_splits_in_herdr_keep_herdrs_shape*` (throwaway `azj-test-` sessions, Apple Silicon, herdr 0.9.1, 60 bursts each of a 2×2 grid, three columns and a nested tree): 5/60 wrong idle and 15/60 under load (three threads sending commands back to back, a pane retitling itself every 5 ms) through herdr's API; 0/60 through its CLI, whose ≈ 95 ms per call let each survey keep up. Every wrong adoption was then pushed into herdr.

### Decision

- **Invariant: only Azulejo changes herdr's structure.** A push may swap panes or rebuild a herdr tab into another shape only to carry out an Azulejo op: a command (from the app, the CLI or the API), a layout preference, or a terminal Azulejo makes (herdr adds its pane second, and the push swaps it into place). For a change that started in herdr, Azulejo follows herdr. A mismatch Azulejo didn't cause resolves as a pull, never a push.
- **Intents.** The driver gives each tab an *intent*, a number it bumps whenever a command changes the tab's arrangement (its tree, ratios aside) or a terminal is made for it. Every target carries the tab's latest intent. The mirror task lets a push restructure (`MirrorTarget::restructure`) only for an intent no successful push has carried out yet, so each op authorises one push. Adoption, panes split or closed in herdr, pulls, reconnects and relaunches never bump it.
- **The planner enforces it.** Without `restructure`, a push compares herdr's tab with the target over the panes both hold. If they sit in another shape or order (`herdr::diverged`), it writes nothing and returns `MirrorError::Diverged` with herdr's tree; the core then makes the tab follow it (`Internal::MirrorFollow`, `mirror::follow`). Without `restructure` a push still sets ratios and zoom, still merges panes in from other tabs (migration) and moves out panes closed here or shown in another tab, all of which leave the panes that stay where herdr has them. It never parks a pane nobody knows yet: it waits for the adoption instead.
- **Adoption converges on herdr's tree.** Every herdr tab a survey adopts or adds panes to then takes herdr's tree from that survey (`layout.export`, read after the snapshot), through `Session::apply_tree`: whatever the sibling placement guessed, the tab ends in herdr's shape. A tree naming a pane the snapshot doesn't have yet is used without it; the next survey brings the pane in.
- **Dynamic tabs.** A pane split off in herdr into a Tile or Three Column tab no longer makes the push rearrange herdr: the tab absorbs herdr's tree when its rule can make it, and freezes to manual otherwise, as for any change made in herdr.
- **Relaunch.** Azulejo's saved ratios still win at the next start; a different order or shape found in herdr is herdr's, and the tab follows it.

### Consequences

- An Azulejo op still turns, swaps and rebuilds herdr tabs as before; adopted tabs and changes made in the TUI are never rewritten into another shape.
- A mismatch that slips past adoption costs one push that writes nothing and one pull: Azulejo shows the wrong shape for about 150 ms, herdr never does.
- If an Azulejo op and a change made in herdr land in the same tab within one quiet period, the op's push carries both, as before. The pull before every push still reads herdr first.
- With `AZJ_DEBUG_MIRROR=1` each push logs its intent and whether it may restructure.

### Evidence

- `core/crates/engine/tests/live_mirror.rs`: `bursts_of_splits_in_herdr_keep_herdrs_shape`, `…_under_load`, `cli_bursts_of_splits_in_herdr_keep_herdrs_shape_under_load` (`AZJ_BURST_ROUNDS`, default 20 rounds of three recipes): 0/60 wrong in each after the fix; also with only the mirror's guard on (adoption left guessing), herdr was never rewritten. Each round checks Azulejo's adopted tree and herdr's tree, after both have been still for 600 ms, against the tree the splits made.
- `core/crates/engine/src/adopt_tests.rs`: `a_burst_seen_half_made_still_ends_in_herdrs_tree` (every set of in-between stages a survey could catch, for the three recipes), `a_tree_newer_than_the_snapshot_is_taken_without_the_newer_pane`.
- `core/crates/herdr/src/mirror_tests.rs`: `another_shape_not_asked_for_writes_nothing_and_hands_back_herdrs_tree`, `panes_in_other_places_not_asked_for_arent_swapped`, `without_restructure_*`, and the property test `without_restructure_herdrs_arrangement_stays` (random trees over random tabs: a push either keeps herdr's arrangement of the panes already there or writes nothing).
- `core/crates/engine/src/mirror_tests.rs`: `only_an_azulejo_op_not_yet_carried_out_may_restructure_herdr` (the mirror task: no intent, no restructure; a diverged push becomes a follow; each intent restructures once), `a_tab_follows_herdrs_arrangement_and_keeps_its_browser`, `following_looks_only_at_the_panes_both_have`.
- `core/crates/engine/src/driver_tests.rs`: `herdrs_own_arrangement_is_followed_and_only_azulejo_ops_restructure_it` (a running core: an adopted grid is never restructured, a swap made behind the core's back is followed, alt+o restructures).
