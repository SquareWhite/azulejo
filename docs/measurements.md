# Measurements

Every performance number for Azulejo goes here, with machine, date and method.

## 0.4 Input latency: libghostty surface, direct vs via `herdr terminal attach`

**Date:** Nov 3, 2026
**Machine:** macOS test environment, 120 Hz display
**Software:** Ghostty v1.4.0 (libghostty, runtime-compiled Metal shaders, no Xcode), herdr 0.9.1
**Reproduce:** `scripts/latency/run.sh 250 3` (safe to run while you're using the Mac)

### Verdict

**Pass.** The herdr hop adds **+1.3 ms median** keystroke → glyph (target ≤ 8.3 ms, one frame at 120 Hz). The byte-level probe puts it at **+1.6 ms median, +1.8 ms p95**, well under one frame.

### Configs

All three use the same libghostty surface (a copy of the 0.3 spike in `scripts/latency/harness/`), the same window size (960×600 pt) and the same Ghostty config: user config plus `cursor-style-blink = false`, `shell-integration = none`. The program under test is `/bin/cat`, so no shell redraw (fish autosuggestions, prompts) is included; echo comes from the PTY line discipline.

- **a. Standalone Ghostty.app: deferred.** It can't be driven in-process, and OS-level key injection isn't allowed while the Mac is in use. `scripts/latency/run-os-input.sh` is ready for a free moment. Until then, **b is the baseline**: same renderer, same config.
- **b. Direct.** The surface runs `/bin/cat`.
- **c. herdr.** The surface runs `herdr --session azj-latency terminal attach <id> --takeover`, and the herdr pane runs `/bin/cat`. The herdr server starts with a clean env and `SHELL=/bin/sh`.

### Method

1. **Injection (in-process).** On the main thread, take `mach_absolute_time()`, then call `ghostty_surface_key` with press + release of `x` (text `"x"`, keycode 7). This is the same call the NSView `keyDown` path makes. Backspace (0x7f) resets after each sample. A random settle of 120–220 ms between samples avoids phase-locking to vsync.
2. **Detection.** A ScreenCaptureKit stream with `SCContentFilter(desktopIndependentWindow:)` captures **only the harness's own window**. It runs at `minimumFrameInterval` = 1/120 s, BGRA, native scale. Latency is the `SCStreamFrameInfo.displayTime` of the first frame whose pixels changed in the calibrated glyph box, minus the injection time.
3. **No focus stealing.** The harness is an `LSUIElement` / `.accessory` app. Its window is ordered to the back, never activates, and no OS input events are posted.
4. **Byte-level hop.** `scripts/latency/herdr_rtt.py` writes `x` to a PTY and times until the echoed byte comes back. It runs once with `/bin/cat` on the PTY (direct) and once with `herdr terminal attach` on the PTY (pane running cat). There's no rendering here, so resolution is microseconds.

**Resolution.** Screen numbers are quantised to the 8.33 ms capture/display interval. Byte-level numbers are at µs resolution.

### Results: keystroke → glyph on screen (ms)

| run | n | misses | min | median | p95 | p99 | max |
| --- | --- | --- | --- | --- | --- | --- | --- |
| b-r1 | 245 | 5 | 5.7 | 21.8 | 44.1 | 136.2 | 239.6 |
| b-r2 | 250 | 0 | 12.0 | 23.8 | 45.0 | 60.9 | 65.7 |
| b-r3 | 242 | 8 | 11.0 | 22.2 | 39.3 | 203.2 | 606.8 |
| c-r1 | 245 | 5 | 11.5 | 24.7 | 181.1 | 406.1 | 448.6 |
| c-r2 | 246 | 4 | 10.0 | 23.1 | 43.8 | 179.4 | 822.4 |
| c-r3 | 250 | 0 | 13.4 | 24.1 | 38.4 | 45.3 | 53.4 |
| **b pooled** | 737 | 13 | 5.7 | **22.6** | 44.3 | 91.6 | 606.8 |
| **c pooled** | 741 | 9 | 10.0 | **23.9** | 48.2 | 300.9 | 822.4 |

**c − b: median +1.3 ms, p95 +3.9 ms.**

### Results: echo round trip, bytes only (ms)

| path | n | min | median | p95 | p99 | max |
| --- | --- | --- | --- | --- | --- | --- |
| direct (PTY + cat) | 500 | 0.021 | 0.043 | 0.068 | 0.098 | 0.201 |
| `herdr terminal attach` | 500 | 0.599 | 1.664 | 1.842 | 2.069 | 20.076 |

**attach − direct: median +1.62 ms, p95 +1.77 ms.**

### Caveats

- **The OS input path is excluded.** HID → WindowServer → NSEvent → `keyDown` is the same for every config, so it doesn't change the delta. It does make absolute numbers lower than a Typometer-style run would show.
- **Absolute screen numbers include the capture pipeline.** They cover WindowServer compositing of an occluded window and ScreenCaptureKit delivery, roughly 1–2 frames, plus frame quantisation. Use the deltas, not the absolutes, and don't compare these against other tools' published numbers.
- **The tails are capture noise, not herdr.** p99 values in the hundreds of ms, and roughly 1% misses (no change seen within 1 s), show up in **both** b and c. They don't show up in the byte-level probe, apart from one 20 ms outlier in 500. The likely cause is WindowServer throttling an occluded window plus system load: a video call was running during the measurement. Re-run with the window visible when the Mac is free to confirm.
- **`/bin/cat` measures transport and render only.** Real shells and TUIs add their own processing equally in every config.
- **herdr re-renders the screen.** The attach stream is herdr's own redraw, not raw PTY bytes. That costs the ~1.6 ms above, and could cost more on heavy full-screen redraws. Not measured here.

### Deferred

- **a vs b vs c through the real OS input path.** Run `scripts/latency/run-os-input.sh` when nobody is using the Mac. It brings windows to the front and posts real keystrokes for about 5 minutes, and asks for confirmation first. It's compiled and syntax-checked but **not run yet**. Results land in `scripts/latency/build/results-os/`.
- **Controller mode** (`herdr terminal session control`) isn't needed: attach already meets the target.
