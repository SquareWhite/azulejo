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

## Evidence

- `spikes/macos-anim/README.md`, findings table
- Deferred: frame pacing with the window visible, and Instruments once Xcode is installed (PLAN "Deferred checks")
