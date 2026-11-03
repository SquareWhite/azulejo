# Spike 0.5: animated resize with libghostty + herdr

Three libghostty surfaces, each running `herdr terminal attach` against its own pane in the throwaway
`azj-anim` session, laid out as A | (B / C). Divider moves, close and split animate. During motion every moving
tile is a CALayer showing a copied IOSurface snapshot of its last frame; the live surface is hidden and
resized exactly once.

```sh
./build.sh                                   # build/AzjAnim.app (swiftc, no Xcode)
./run.sh --stress 50 --exit --resize-at start --reveal first --log build/run.jsonl
./run.sh --duration 2000 --resize-at start   # slow; trigger with kill -USR1 (divider) / -USR2 (close/split)
./run.sh --stop                              # stop the azj-anim session
```

`run.sh` creates the session with a clean env: htop in A (`-d 100`), `winch.sh` in B and C, which logs
every SIGWINCH with a timestamp to `build/winch-{B,C}.log`. The app is an accessory process. It never
activates, orders its window to the back (`--front` to override), and takes no synthetic input.

Other flags: `--mode ca|link` (CABasicAnimation vs per-frame from the display link), `--gravity resize|topLeft`,
`--reveal first|quiet`, `--quiet-ms`, `--gap MS`, `--dump DIR` (PNG of snapshot / revealed frame / +300 ms).
`probes/` holds the PTY-level latency probes (herdr alone, and no herdr).

## Findings (macOS test environment, 120 Hz display, Ghostty v1.4.0, herdr 0.9.1)

| Config, 50 back-to-back animations | Dropped frames | Interval median / max | Resizes (actual/expected) | Snapshot → live after motion end |
| --- | --- | --- | --- | --- |
| ca, resize at end, reveal quiet | 0 / ~1,250 | 8.33 / 8.33 ms | 100 / 100 | median 100 ms, max 216 ms (stale until the app redraws) |
| ca, resize at end, reveal first | 0 | 8.33 / 8.33 ms | 100 / 100 | 8 ms, but every reveal shows a stale crop |
| ca, resize at **start**, reveal first | 0 | 8.33 / 8.33 ms | 100 / 100 | 8 ms, final content, no later frames |
| link, resize at start, reveal first | 0 / 1,204 | 8.33 / 8.33 ms | 100 / 100 | same tick |

- **One resize per animation holds end to end.** The app counts one `ghostty_surface_set_size` per tile per
  animation. The SIGWINCH logs inside the herdr panes show the same number (e.g. B: `winch=51` after
  1 attach + 50 animations).
- **Setup cost** is 1–2 ms median, 7 ms max, once snapshots are IOSurface copies. A CGImage took 14–45 ms,
  because CA converts it at commit.
- **Resize → SIGWINCH in the pane takes 86–118 ms (median).** Ghostty coalesces resizes for 25 ms
  (`termio/Thread.zig`, `Coalesce.min_ms`). herdr adds about 37 ms median and up to about 80 ms at p95
  (bare PTY → pane: median 48 ms, against 11 ms with no herdr). The TUI only redraws after that. Present traces
  after a resize show frames at 8/16/33 ms (local reflow + herdr repaint), then the app's redraw at
  50–160 ms.
- So **resize at the end of the motion** means the snapshot either stays up for about 100–200 ms, or the
  reveal shows a cropped or half-empty stale frame (seen in the `--dump` PNGs). **Resizing once at the start
  of the motion**, hidden under the snapshot, lets the redraw finish during the 200 ms animation. In
  `--dump` PNGs, the revealed frame was already final every time (e.g. `winch=3 size=56x42` fully drawn), and
  with `--gap 400` there were 0 frames after the reveal across 24 animations, against 227 with resize-at-end.
  **Recommendation: resize at motion start**, still exactly once. Revisit the PRD wording ("resize the PTY once
  at the end").
- **Flicker/garbage:** after `--stress 50` with htop running, the window capture is clean (`build/shots/01-…`).
  Slowed mid-motion captures (`03`, `05`, `07`) show snapshots scaling cleanly, with no blank or torn
  tiles. Scaled text is visibly stretched mid-motion; `--gravity topLeft` (crop) is the alternative.

## Not verified (constraints: no synthetic input, no focus stealing, no Instruments)

- **Frames are measured on the main thread** (`NSView.displayLink` timestamps), not by Instruments, because
  there's no Xcode or xctrace. In `ca` mode, the render server drives the motion, so this shows the main
  thread never stalled, not that the compositor never dropped a frame. `link` mode makes every frame on the
  main thread, so its 0 drops are a stronger signal.
- **The window was behind other windows the whole time** (`window_visible_fraction` 0). The display link
  still ticked at 120 Hz and captures render, but drops under real on-screen composition aren't
  measured. To run later, with the window visible and you not in a meeting:
  `./run.sh --stress 50 --exit --front --resize-at start --reveal first --log build/visible.jsonl`
  Once Xcode is installed:
  `xcrun xctrace record --template 'Animation Hitches' --output build/anim.trace --launch -- build/AzjAnim.app/Contents/MacOS/AzjAnim <args from run.sh>`
- **Keyboard triggers** (⌘1 / ⌘2) aren't exercised; signals and `--stress` drive everything.
- **Claude Code inside a tile** during animation, and window drag-resize, aren't tested.
