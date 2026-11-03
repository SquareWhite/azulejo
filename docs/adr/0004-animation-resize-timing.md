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
- New tiles sweep in with the divider (from the parent's far edge); closing tiles shrink away from the neighbour that takes their space; the tall rotate cross-fades, since sliding every tile across the tab looked wrong.
- **Outline** hides content during the motion, moves outlines only, and fades content in over 80 ms at the end.
- Same budget: 60-animation stress at 120 Hz, crop 0 dropped frames, outline 1, scale 3 (≤ 3 single missed ticks allowed); 114/114 PTY resizes in every style.

## Evidence

- `spikes/macos-anim/README.md`, findings table
- Deferred: frame pacing with the window visible, and Instruments once Xcode is installed (PLAN "Deferred checks")
