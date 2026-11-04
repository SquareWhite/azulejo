# 0004: Resize terminals once, at the start of a layout animation

Status: accepted · Nov 3, 2026 · Gate G0 · Changes PRD F4

## Context

During layout motion a terminal tile shows a scaled snapshot of its last frame, and its PTY is resized exactly once. The PRD said to resize at the end of the motion.

Spike 0.5 measured 86–118 ms (median) from resize to SIGWINCH in the pane: Ghostty coalesces resizes for 25 ms, and herdr adds about 37 ms median (up to about 80 ms). The TUI only redraws after that. With the resize at the end, the snapshot either stays up 100–200 ms after the motion, or the reveal shows a cropped, half-empty stale frame.

## Decision

- Resize each moving terminal exactly once, at the **start** of the motion, to its final size. The redraw happens hidden under the snapshot during the 150–250 ms animation.
- Reveal the live surface on the first frame after the motion ends.
- Take snapshots as IOSurface copies of the surface's last frame on macOS. A `CGImage` cost 14–45 ms per animation because Core Animation converts it at commit; the IOSurface copy costs 1–2 ms median.
- Linux uses the same rule with `GtkWidgetPaintable` snapshots (step 4.1).

## Consequences

- 0 dropped frames at 120 Hz across 50 back-to-back animations, exactly one resize per tile per animation, and every revealed frame was already final.
- Scaled text looks stretched mid-motion. A crop mode (`--gravity topLeft` in the spike) is the alternative if it bothers people.
- Very slow TUIs (redraw > animation length) still reveal a stale frame. Acceptable; the user can turn animations off.

## Amendment, Nov 3, 2026: crop is the default

Dogfooding: "animations stretch the content of the tile, which looks bad". `[animation] style` now picks `crop` (default), `scale` (the behaviour above) or `outline`.

- **Crop** animates each tile layer's bounds and position instead of a transform. The frame is a clip over content at 1:1, so nothing stretches. Content sticks to the edge that moves least, so it stays still on screen while the moving edge reveals or cuts it off.
- The resize-at-start rule stands. A shrinking terminal keeps its snapshot, now at 1:1, until the motion ends. A growing one shows its 1:1 snapshot over a plain background until live content has drawn at the new size, then reveals live content, even mid-motion: live content is laid out at the final size, so it can't stretch, and the blank strip the growing edge uncovers fills in early.
- New tiles sweep in with the divider (from the parent's far edge); closing tiles shrink away from the neighbour that takes their space; the tall rotate cross-fades, since sliding every tile across the tab looked wrong. (Replaced Nov 4: rotates move too, see below.)
- **Outline** hides content during the motion, moves outlines only, and fades content in over 80 ms at the end.
- Same budget: 60-animation stress at 120 Hz, crop 0 dropped frames, outline 1, scale 3 (≤ 3 single missed ticks allowed); 114/114 PTY resizes in every style.

## Amendment, Nov 4, 2026: the reveal doesn't need the display link

The e2e pass (V1) saw terminals stuck on their motion snapshot with the screen locked. The reveal polled only on the container's display link, which doesn't tick then; libghostty kept drawing (first frame at the new size 1–11 ms after the resize). A plain timer now takes the snapshot down 50 ms after the 250 ms deadline if the clock hasn't. With the screen on the clock still reveals first. Captures report what each terminal shows and flag a stale frame (`STALE` in automation.log, `<name>.fresh.json`); `apps/macos/automation/e2e/stale-check.sh` checks the screen-on path.

## Amendment, Nov 4, 2026: every layout change moves

the maintainer: rotate and the layout switches cross-faded (a ghost of the old card faded out while the new layout appeared), so they didn't read as motion. Now every change with a geometric relation moves each tile from its old frame to its new one, position and size together, on the configured curve and length, under the same crop rules (1:1 content, one PTY resize per changed terminal at the start, reveal on a fresh frame).

- **Moves:** rotate (four-way and the manual flip), layout cycle and toggles, masters ±, master area ±, move along the order, promote, equalize, rotate part, plus swaps, splits, closes, zoom and the sidebar, which already moved. The focused (or zoomed) tile travels over the others where paths cross. Headers, corner caps, badges, the focus dim and now open find bars (and the browser's toolbar) ride the top of the moving clip.
- **Monocle in:** the focused tile grows to fill the tab over the others, which fade and shrink 3 % where they are. Monocle sizes hidden tiles to the whole tab (ADR 0010), so they freeze under their snapshot first: their one resize happens at the start too. **Monocle out:** the focused tile shrinks to its slot while the others fade and grow in from 3 % inside theirs, their last frame at the top left until they've drawn at the new size.
- **Focus inside Monocle** stays free of PTY resizes. The next tile dissolves in over the last one in 60 % of the motion's length (120 ms by default); the one underneath stays opaque until it's covered. The tiles are stacked in one place, so a slide would invent a direction that isn't there, and moving a whole terminal's text across the tab reads heavier than the switch is.
- **Cross-fades left:** tab and workspace switches (another tab's tiles, no geometric relation) and the Monocle dissolve. The rotate ghosts are gone.
- **Retargets:** a change mid-motion starts every tile from where it shows, snapshot and opacity included. A tile on its way out keeps going through refreshes, and one wanted back mid-fade (Monocle in and straight out) turns round from where it is. Snapshots that are already up stay up, so rapid repeats never resize a terminal more than once per motion that changes its size.
- **Set-up, not mid-motion:** libghostty redraws a resized surface synchronously when Core Animation next displays its layer, waiting on the GPU (2–4 ms). That used to land in a separate run-loop pass after the motion had started (8–10 ms passes coming out of Monocle). The resize now displays the layer straight away, so the cost is part of the key press. A surface coming back on screen also no longer resets its content scale and display ID when neither changed.
- Measured (`anim.sh`, 120 Hz, busy machine): 105 motions with 11 retargets (rotate ×4, the layout cycle both ways, Monocle in, focus and out, manual and back, rapid repeats), PTY resizes 345/345, 0 main-thread passes over a frame (max 4.8 ms), 3 single dropped ticks with no main-thread cause. `--slow`: 74 captures, 0 stray pixels, 0 STALE.

## Amendment, Nov 4, 2026: reflow is the default

the maintainer: "smooth pane animations (immediate terminal text update. currently we crop, then once done - update)". He wants the text to follow the tile as it moves, like drag-resizing a Ghostty or kitty window. `[animation] style = "reflow"` is the new default; `crop`, `scale` and `outline` stay as they were.

- **Frames step on the main thread.** A reflowing tile's frame moves to where the curve puts it at each display-link tick (`link.targetTimestamp`, device-pixel aligned), so its header, dim, corners and content lay out at every step. Nothing jumps to the final frame first.
- **Terminals resize live, by whole cells, throttled.** The surface takes the tile's size only when that changes it by at least one cell in either axis and the last resize was ≥ 33 ms ago (`Motion.reflowInterval`; libghostty already sends at most one PTY resize per 25 ms). In between it keeps its size, pinned top left: the terminal background shows round a smaller surface, the tile clips a bigger one. At the end, one exact resize. A 200 ms motion costs a terminal 1 to ⌈200/33⌉ + 1 = 8 resizes; measured ≈ 3 live plus the final one. `anim expect-clean` checks that range.
- **No synchronous draws.** libghostty sets `needsDisplayOnBoundsChange` on its layer, so each resize drew synchronously in the Core Animation commit, waiting on the GPU (2–4 ms per terminal; four terminals resizing on one tick blew the frame: 65 passes over a frame in the stress). While a terminal reflows that's off, and libghostty's renderer thread draws the new size a few ms later. Its layer keeps the last frame top left until then (`kCAGravityTopLeft`, no stretching) and drops frames drawn for another size, which is why the throttle also keeps the layer's size still long enough for a frame to land.
- **Tile headers** measured and redrew their text at every layout; they now cache text widths and only reset the label when what it shows changes (0.12 → 0.03 ms per layout).
- **The text lags the frame.** A resize reaches the TUI through herdr (spike 0.5: 86–118 ms resize to SIGWINCH in the pane), so the TUI's redraw lands about 50–100 ms behind the frame and the last one after the motion. Until then libghostty shows the old content at the new size (cropped or padded; the shell's primary screen rewraps at once).
- **What stays as in crop:** a closing tile and the tiles Monocle hides leave with a 1:1 snapshot (a closed tile has no surface left; Monocle sizes hidden tiles to the whole tab under it). A terminal whose pane starts mid-motion (a split) starts at its final size and keeps it, so a shell isn't born two columns wide. A Monocle focus switch still resizes nothing.
- **herdr stays quiet.** Pane sizes come from the attach client, not herdr's layout tree, so live resizes send herdr no layout change and the mirror (ADR 0012) has nothing to push or pull. Measured with `AZJ_DEBUG_MIRROR=1`, four terminals, eight sidebar toggles: 158 live resizes, 0 herdr events, 0 pushes, 0 pulls in that span; the whole run logged the same 6 pushes and 13 events as crop.
- **Ending a motion:** a transaction with no Core Animation animations completes at once, so the display-link clock ends a reflow, with a timer 100 ms after its end for when the link doesn't tick (screen locked).
- Measured (`anim.sh`, 120 Hz, busy machine, load ≈ 3, three runs each): crop dropped 5, 7 and 2 ticks, reflow 0, 2 and 0; main-thread passes p99 1.5 → 2.5 ms, one pass over a frame in three reflow runs; key-press setup 13 → 0.7 ms median; PTY resizes 345 → ≈ 1,480 (no mismatches). `--slow`: 86 captures, 0 stray pixels, 0 STALE. Details in [measurements](../measurements.md#reflow-motion-adr-0004-nov-4-reflow-amendment).

## Evidence

- `spikes/macos-anim/README.md`, findings table
- Deferred: frame pacing with the window visible, and Instruments once Xcode is installed (PLAN "Deferred checks")
