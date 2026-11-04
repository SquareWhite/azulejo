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

## 1.9a Preview rendering and file watching (core)

Machine: macOS test environment, release build, 2026-11-03. Method: `cargo run --release -p azulejo-preview --example bench` renders each file 5 times after one warm-up render; numbers are wall time for `azulejo_preview::render` (read, classify, render, wrap in the page). Watcher latency comes from `cargo test -p azulejo-preview --test watch -- --nocapture`.

| Input | Size | Median | Range |
| --- | --- | --- | --- |
| First render (loads syntaxes and theme CSS) | tiny | 18 ms | once per process |
| Markdown, plan-sized (20 sections, tables, task lists, a Rust fence each) | 10 KB, 380 lines | 3.8 ms | 3.5–11 ms |
| Markdown, the same repeated | 1.0 MB, 53k lines | 361 ms | 359–374 ms |
| Rust source | 0.24 MB, 10k lines | 255 ms | 254–260 ms |

| Watcher event (FSEvents, 50 ms debounce) | Latency |
| --- | --- |
| Plain write | ~70 ms |
| Editor-style atomic save (write temp, rename over) | 62–72 ms |
| Write after an atomic save | 68–70 ms |
| Delete | 62–72 ms |

- **Highlighting dominates.** Most of the 1 MB markdown time is syntect on its ~3,000 code fences; the 10k-line file is all syntect (pure-Rust `fancy-regex` engine). Files over 2 MB skip highlighting, and text over 5 MB is cut with a notice.
- **The 200 ms target in step 1.9 holds for real plans:** a plan-sized file renders in ~4 ms, plus ~70 ms of watcher latency (mostly the 50 ms debounce). Large files render off the engine's thread, so they never block layout commands; they just update later.
- **Not measured:** inotify on Linux, and webview time to display the HTML (shell side, step 1.9b).

## 1.12 Session restore and wake (macOS)

Machine: macOS test environment, debug build, 2026-11-03. Method: `apps/macos/automation/restore.sh` on dev instance `restore` (herdr session `azj-dev-restore`), app launched with `--bg`, no OS input. Session: 6 terminal tiles in a tall layout, a second tab with a browser and a zoomed preview, sidebar hidden, a ticker running in the first terminal. Times are wall clock until **every** terminal tile's attach is running and still running 2 s later (no `attach ended` after its last attach). Three runs; target ≤ 2 s.

| Step | Run 1 | Run 2 | Run 3 |
| --- | --- | --- | --- |
| Quit (SIGTERM) → relaunch (launch → all attached) | 280 ms | 296 ms | 282 ms |
| kill -9 → relaunch | 291 ms | 307 ms | 286 ms |
| Every attach process SIGKILLed, then wake | 259 ms | 65 ms | 62 ms |
| Every attach process SIGKILLed, no wake (second failure in 10 s backs off 1 s) | 1108 ms | 1104 ms | 1115 ms |
| herdr server stopped and started again (server start → all reattached) | 756 ms | 752 ms | 754 ms |
| Launch with the herdr server stopped (the app starts it) | 490 ms | 484 ms | 460 ms |

- **No lost sessions.** After each relaunch and wake step the session JSON (minus window frames) was identical, herdr had the same terminal IDs with none added, and the ticker kept counting.
- **herdr restarts lose processes.** That's herdr's own behaviour (ADR 0008): it restores workspaces, tabs and panes, but with fresh shells under new terminal IDs. Tiles rebind and reattach to them; whatever ran before is gone.
- **"Attached" means the attach process runs and holds**, not first paint. The extra herdr hop is ~1.6 ms (0.4 above).
- **Wake is simulated** through the dev debug hook (`dev.azulejo.debug.terminal "wake all"`), which runs the same health check as `NSWorkspace.didWakeNotification`. A real sleep/wake cycle isn't measured.

## 1.13 NFR pass (macOS)

**Date:** Nov 3, 2026
**Machine:** macOS test environment, 120 Hz display
**Builds:** debug (`just mac`, HEAD `9076e4e` + the 1.13 instrumentation) and release (`just release-mac`, ad-hoc signed). GhosttyKit is ReleaseFast in both.
**Where:** dev instance `nfr` (herdr session `azj-dev-nfr`). Apps launched in the background, no OS input, no window raised. Scripts in `scripts/nfr/`, output in `scripts/nfr/build/` (gitignored).

### Summary

| NFR | Target | Measured | Verdict |
| --- | --- | --- | --- |
| Platform | macOS 14+, Apple Silicon | Builds and runs on macOS 26 on Apple Silicon; deployment target is macOS 14 (`Package.swift`, `LSMinimumSystemVersion`), never run on 14 | Pass (on this machine); macOS 14 untested |
| Stack | Rust core, Swift/AppKit, no Electron, no webview terminal | As built | Pass |
| Cold start | < 500 ms to a usable window with restored layout | Release, 2 workspaces / 8 tiles restored: first frame **248 ms** median, 265 ms p95. Empty state: 148 ms median | **Pass** |
| Input latency | Within 1 frame of standalone Ghostty | App tile vs the 0.4 libghostty spike on the same herdr pane, same conditions: **18.8 vs 18.9 ms** median (−0.1 ms) | **Pass** against the spike; standalone Ghostty.app still deferred |
| Animation | 120 Hz, 150–250 ms transitions | Not in the app yet (F4 is P1, step 2.1). The 0.5 spike: 0 dropped frames at 120 Hz | Deferred to M2 |
| Memory | < 150 MB, one window, 4 terminal tiles | **195 MB** (debug and release alike) | **Fail**: see below |
| herdr reconnect | ≤ 2 s after restart or sleep | Re-run at HEAD: relaunch 276 ms, kill -9 + relaunch 272 ms, wake 65 ms, herdr restart 726 ms | **Pass** |
| Stability | Browser or preview crash never takes down terminals | WebContent and QuickLookUIService SIGKILLed: app, attach processes and ticker unaffected | **Pass** |
| API | Every user action via the socket API; versioned JSON | Shell actions go through core commands (`azj_core_key` → command); `schema/api.v1.json`, CLI coverage test per method; `core.version` reports v1 | Pass (by construction) |
| Security | Socket readable only by the user (the per-workspace automation switch went with F21, dropped Nov 3, 2026) | Socket 0600, dir 0700, owned by the user | Pass |

### Cold start

`scripts/nfr/coldstart.sh [runs]` (`NFR_APP=…` for the release bundle). In-app marks (`Startup.swift`, logged with `AZULEJO_TIMING_OUT`) are measured from the **kernel's process start time**, so dyld and Swift runtime start-up count. `first_frame` is the first display-link tick after the launch layout is drawn and committed. Every launch goes through `open` (LaunchServices adds another 33–45 ms before the process exists, reported separately). The herdr server is already running; the empty-state runs start from a fresh, empty herdr session each time.

Restored state: window 1 has 4 terminals in a tall layout; workspace 2 has 2 terminals, a browser and a preview. 10 runs each, ms:

| Build | State | main | core booted | layout built | first frame median | p95 | max | visible terminals attached (median / p95) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| debug | empty (1 tile) | 33 | 70 | 137 | 163 | 181 | 181 | – |
| debug | restored (8 tiles) | 32 | 72 | 228 | 267 | 299 | 299 | 216 / 246 (4 tiles) |
| release | empty (1 tile) | 27 | 55 | 121 | 148 | 563* | 563* | – |
| release | restored (8 tiles) | 28 | 60 | 215 | 248 | 265 | 265 | 202 / 221 (4 tiles) |

\* The first launch after `just release-mac` re-signs the bundle reached `main` at 427 ms: macOS checks a freshly signed binary once. Without that run, the release empty-state max is 150 ms. Expect it once after each install or update.

Most of the time between "core booted" and "layout built" goes to building tile views: terminal surfaces, a WKWebView and the preview webview. Terminals attach before the first frame is on screen.

### Memory

`scripts/nfr/memory.sh` reports `footprint`'s phys_footprint, which is what Activity Monitor shows as Memory. It takes the median of 5 samples, 2 s apart, starting from a fresh herdr session. The 4 terminal tiles sit in a tall layout in an 1100×732 pt window, running idle zsh.

| Build | App, 4 terminals | Attach chains (4 × sh + herdr attach) | azj-observe | herdr server |
| --- | --- | --- | --- | --- |
| debug | 195 MB (180–195) | 17 MB | none running | 14 MB |
| release | 196 MB (180–197) | 17 MB | none running | 13 MB |

**Where it goes.** One live terminal tile costs 154 MB, and each extra one adds 10–17 MB (1/2/3/4 tiles: 154/172/179/189 MB). The app with no live terminal is about 60 MB. `footprint` puts about 140 MB of the 4-terminal total in Metal and graphics memory: 95 MB "owned physical footprint (graphics)", 25 MB IOAccelerator and 21 MB IOSurface. The rest is mostly malloc (45 MB). The 0.4 latency harness, a bare libghostty surface with no Azulejo code, is **150 MB with one surface**, including 90 MB of graphics. So libghostty's renderer, not Azulejo's shell or core, sets the floor, and a 4-tile window can't get under 150 MB without changing that.

Footprint also moves with tile sizes:
- Zooming one tile: 218 MB.
- Adding a browser tile: the app drops to 108–118 MB, while 4 new WebKit processes take 44–49 MB. Closing the browser tile brings it back to 195 MB.

These swings come from surface sizes and from how macOS attributes GPU memory between processes. We didn't dig further.

**Follow-ups.** None of these is a small, clearly scoped fix, so none landed in this step.
1. Measure Ghostty.app with 4 splits on this machine. That gives the comparable number, and the target may need to become "Ghostty + N MB".
2. With Xcode installed, compare the precompiled metallib build against our runtime-compiled shaders (no-Xcode patch), in case runtime compilation keeps extra memory.
3. Ask upstream about libghostty's per-app and per-surface GPU allocations.

### Input latency

`scripts/nfr/latency.sh [samples] [rounds]` reuses the 0.4 method inside the real app (`LatencyBench.swift`, off unless `AZULEJO_LATENCY_BENCH` is set):
- It calls `ghostty_surface_key` on the main thread for the app's terminal tile, which is attached through herdr to a pane running `/bin/cat`.
- A window-only ScreenCaptureKit stream at 120 Hz detects the glyph.
- The app gives the surface libghostty focus, as a key window would, because these runs never activate it.
- The Ghostty settings are the same as in 0.4: no cursor blink, no shell integration.

Each round then runs the 0.4 spike harness (config c) on **the same herdr pane**, so both are measured under the same WindowServer conditions. Debug build: 3 rounds × 100 samples each, no misses.

| | n | median | p95 |
| --- | --- | --- | --- |
| App tile, all samples | 300 | 20.0 | 340 |
| Spike (0.4 config c), all samples | 300 | 21.1 | 350 |
| App tile, unthrottled samples (< 100 ms) | 248 | **18.8** | 24.8 |
| Spike, unthrottled samples (< 100 ms) | 205 | **18.9** | 25.0 |

- **The app tile adds nothing measurable** over the bare libghostty surface: −0.1 ms median, against a budget of 8.3 ms (one frame).
- **The samples are bimodal, and it's the compositor, not Azulejo.** Whole rounds of both the app and the spike sat at about 320 ms: app-r1 and app-r3 tails, and spike-r3 with a median of 319 ms. At those times the background window was fully covered (Slack in front), and WindowServer updates covered windows slowly. A spike-only check earlier in the session also gave 317 ms. Unthrottled rounds match 0.4's 22–24 ms.
- **Still deferred:** comparing against standalone Ghostty.app through real OS input (`scripts/latency/run-os-input.sh`), and the release build. libghostty is ReleaseFast in both builds; only the Swift and Rust around it differ.

### herdr reconnect

`apps/macos/automation/restore.sh` re-run at HEAD (6 terminals, ticker, browser, zoomed preview):

| Step | 1.12 (3 runs) | 1.13 re-run |
| --- | --- | --- |
| Quit → relaunch, all attached and holding | 280–296 ms | 276 ms |
| kill -9 → relaunch | 286–307 ms | 272 ms |
| Attach processes SIGKILLed, then wake | 62–259 ms | 65 ms |
| Same, no wake (backs off 1 s) | ~1.1 s | 1114 ms |
| herdr server restart | ~755 ms | 726 ms |
| Launch with herdr stopped | 460–490 ms | 463 ms |

Each step left the session identical, kept the same herdr terminals and the ticker kept counting (except across the herdr restart, which loses processes by herdr's design).

### Stability and security

`scripts/nfr/stability.sh`:
- **Setup:** two terminals (one running a ticker), a browser tile on a local page and a preview tile on a PDF, which spawned a `QuickLookUIService` XPC process.
- **Test:** SIGKILLed the browser tile's `com.apple.WebKit.WebContent` process, then the new QuickLookUIService process. Only processes that appeared for these tiles were killed.
- **Result:** the app kept running, the attach processes were the same PIDs, and the ticker went from 23 to 42.
- **Socket:** the API socket is 0600 and its directory 0700, both owned by the user.

## Animation frame drops after 2.12–2.15: cause and fix

macOS test environment, 120 Hz display, Nov 3–4 2026, dev instance `perf`, `anim.sh` crop stress (55 animations: resize, swap, zoom, rotate, promote, focus, equalize, close, split, tab switch), load average 0.7–1.8.

### What the drops were

The crop stress test's missed ticks rose from 0–3 per run (before 2.12) to 1–9 (after 2.14/2.15). Every drop was a single missed tick, and PTY resize counts always matched. The display-link counter only sees main-thread stalls, so `Perf` (new, `Perf.swift`) times the main thread's work in named spans, plus each run-loop pass and AppKit's display cycle. `anim stats` now reports which spans overlapped each dropped tick (`drops`, `drop_causes_ms`) and per-phase timings (`perf.phases`).

The screen was locked during this work, so the display link didn't fire (`ticks: 0`). `perf.motion_passes_*` stands in for it: main run-loop passes while tiles move, excluding the pass that starts the motion. Any pass over half a frame (4.2 ms) risks a tick and one over a frame (8.3 ms) misses one.

- **The cause: a second, full UI refresh mid-motion, about once per motion.** The command that starts a motion refreshes the UI at once. Its events (`layout.changed`, `focus.changed`, `tile.updated`) then arrive a few ms later and triggered another `session.get` fetch, decode and apply of every window, inside the motion's first frames. 2.12 and 2.14/2.15 made that refresh dearer (bigger tiles: rects, headers, pane metadata, agent state) and more frequent (header and process events):
  - typically 3–5 ms
  - 10–13 ms when a `tile.updated` (agent state or header) reached the terminal view, which re-ran its attach bookkeeping and a layout pass
- **Smaller costs:**
  - a `machines.list` fetch on every refresh
  - a `JSONSerialization` parse and re-serialise of every reply before `JSONDecoder`

### Fixes

1. **Events don't refresh mid-motion.** An event-driven refresh waits until the motion ends (+30 ms). A user command still refreshes at once, as before.
2. **Unchanged sessions skip the apply.** A refresh whose `session.get` reply is byte-identical to the last one skips decoding and every window's apply. That's 71 of 130 refreshes in the stress.
3. **Direct decode.** The reply decodes straight from the core's JSON (`Core.fetchRaw`).
4. **`machines.list` on demand,** only after `machine.*` or `config.reloaded` events.
5. **Content views ignore header and agent changes.** `TileFrame` stops forwarding them; only the header strip and the attention overlay use them.

### Before and after (proxy: mid-motion run-loop passes)

| | runs | passes > 4.2 ms | passes > 8.3 ms | worst pass | refreshes while moving |
| --- | --- | --- | --- | --- | --- |
| Before (instrumented, no fixes) | 4 | 4, 4, 7, 5 | 1, 1, 0, 2 | 10.7, 12.0, 7.5, 13.4 ms | 58–60 per run |
| After | 7 | 3, 0, 1, 2, 4, 1, 1 | 0 in every run | 3.6–7.9 ms | 0 |

The passes over 4.2 ms that remain after the fixes contain no app span and no display-cycle time. They're most likely the main thread losing the CPU under load.

### Phase timings (after, typical run)

| Span | Count | Mean | Max | When |
| --- | --- | --- | --- | --- |
| `refresh` | 129 | 3.5 ms | 22.7 ms | 58 decode + apply; the rest stop at the unchanged check (~0.5 ms) |
| `fetch.exec:session.get` | 129 | 0.53 ms | | core round trip |
| `fetch.decode:session.get` | 58 | 0.25 ms | | |
| `apply.tiles` | 58 | 5.7 ms | 19.9 ms | mostly the motion's own start: `motion.layout`, 53 × 4.5 ms mean, the one PTY resize plus headers |
| `apply.sidebar`, `apply.tabbar`, `apply.attention` | | < 0.2 ms | | |
| `key` | 74 | 0.9 ms | | |
| `event.route` | 80 | 0.4 ms | | |
| `ghostty.tick` | 307 | 0.006 ms | | |

Work before the first frame (`setup_median_ms`) is about 5 ms. It runs before the display link restarts, so it doesn't count as a drop.

### Deferred

Re-run `apps/macos/automation/anim.sh` with the screen unlocked on a quiet machine, so the display link fires. Target: ≤ 3 missed ticks in ≥ 5 of 6 runs. Any drop that remains now lists its cause in `anim-stats.json` (`drops`).

## Tile cards (D.5): crop stress with the screen on

Nov 3, 2026, Apple Silicon, 120 Hz built-in display, screen unlocked (display link firing), other agents' builds running. `apps/macos/automation/anim.sh`, crop style, one run, on the D.5–D.7 branch: cards with corner caps (window-colour nine-slice over the corners, no rounded `masksToBounds`), header strips with a hairline, 8 pt terminal margin.

| Measure | Result |
| --- | --- |
| Animations | 55 |
| Display-link ticks | 469 |
| Dropped ticks | 0 (median, p99 and max interval 8.33 ms) |
| PTY resizes expected / actual | 115 / 115 |
| Main-thread passes while moving | 2,858; 3 over half a frame, 0 over a frame; max 6.25 ms, p99 1.1 ms |
| Refreshes while moving | 0 |
| Setup before the first frame | 5.3 ms median, 20.3 ms max |

One run, not six, so the ≥ 5-of-6 target above stays open. The inner tile views no longer round and mask their own corners (terminal radius 4, tile radius 6 before), so no layer in a tile is masked with a radius now; crop motion's clip is rectangular. GPU-side offscreen passes can't be counted without Instruments (full Xcode).

## Sidebar motion and the focus dim: crop stress

Nov 4, 2026, macOS test environment, 120 Hz display, screen unlocked, load average about 2. `apps/macos/automation/anim.sh`, crop style, one run, on the feedback branch: the focus ring is gone, unfocused tiles carry a dim layer (one more sublayer per card, cross-fading on focus moves), and the stress adds the sidebar closing and opening plus a toggle turned round 150 ms in (a retarget, so each terminal resizes twice in that chain).

| Measure | Result |
| --- | --- |
| Animations | 48 (4 sidebar, 1 retarget); focus moves no longer count, the ring that moved is gone |
| Display-link ticks | 1,156 |
| Dropped ticks | 1, a single missed tick in a sidebar motion (median and p99 interval 8.33 ms, max 16.67 ms) |
| PTY resizes expected / actual | 131 / 131 |
| Main-thread passes while moving | 4,112; 6 over half a frame, 0 over a frame; max 5.45 ms, p99 1.05 ms |
| Setup before the first frame | 6.4 ms median, 20.3 ms max |

A chain of motions that retarget each other now expects one resize per motion that changes a terminal's size, rather than one per chain; motions under 50 ms apart may merge (the terminal's resize coalescing), so such a chain only has to stay between one (when it ends at a new size) and that count.

## herdr reconnect after the e2e pass: cause and fix

**Date:** Nov 4, 2026
**Machine:** macOS test environment, herdr 0.9.1, debug builds (`just mac`). Screen locked, other agents' builds running: the load average is next to every number.
**Builds:** 1.13 (`3a0755b`), before (`5c35b22`, the e2e pass) and after (this branch), each built from its own tree and run on its own `perf…` dev instance.
**Method:**
- `scripts/nfr/reconnect.sh [runs]` (new): restore.sh's session (6 terminals, a browser and a zoomed preview), then per run: stop herdr for 1 s and start it (server start → every terminal reattached and holding), and quit, stop herdr and launch (launch → every terminal attached and holding).
- A scratch harness doing the same through `apps/macos/dev.sh` (rounds 1 and 2 below), so launches include `dev.sh`.
- `apps/macos/automation/restore.sh`, unchanged, for the other restore steps.
- `scripts/nfr/coldstart.sh 10` for boot.

### What was slow

A timestamped trace of the core and the shell (not committed) showed where the time went. herdr's machine list, git and header polling weren't on the path. Machines are off in dev profiles and run on their own thread anyway; git runs on its own threads; process polling runs beside the worker.

1. **Shells reattached to terminals herdr had replaced.** A herdr restart gives every pane a fresh terminal. The core sent `herdr.status connected` as soon as its event stream came back, and checked the tiles afterwards: one `pane.list` per tile, queued on the worker behind a survey. So every tile first reattached to its old terminal ("terminal not found"), then switched when the rebind landed. Since 2.15, header and agent updates also restarted attaches to the dead terminal while a tile waited out "terminal gone".
2. **That churn sometimes killed the new attach.** An attach started right after the tile stopped its previous one sometimes died within 10–30 ms, before writing a status or stderr. The trace shows it; the libghostty mechanism wasn't pinned down. The tile read it as a failure. It was the second in 10 s (herdr going away was the first), so the tile backed off for 1 s: the 1.7–1.8 s restarts. At launch, the retry hit "already attached" and waited 250 ms: the ~800 ms launches.
3. **The event stream retried up to 1 s apart.** After restore.sh's 1 s outage, the next try came 0.5–0.6 s after herdr was back. That's the ~750 ms floor, in 1.13 too.

### Fixes

- **Core:** a machine counts as `connected` only after the core has rebound its tiles to the panes herdr has now. One `pane.list` covers all tiles, run beside the worker when a machine is added and on every connect and resync. `herdr.status` and `machine.status` are sent only on change. Local starts `connected` when herdr was already running, so a normal relaunch doesn't wait. The local event stream retries at least every 250 ms. ADR 0011 has the amendment.
- **Shell:** terminal tiles wait for their machine to be connected (2 s at most). "herdr back" reattaches after the session refresh, with the new bindings. "Terminal gone" holds still until the core decides. A rebind clears the old terminal's failure history. "Already attached" retries after 0.1–0.4 s instead of 0.25 s each.

### Before and after

`scripts/nfr/reconnect.sh 5`, one build after the other:

| Build | herdr restart → reattached | Launch with herdr stopped | Load |
| --- | --- | --- | --- |
| 1.13 (`3a0755b`) | 736, 743, 862, 943, 766 ms (median 766) | 442, 449, 382, 305, 436 ms (median 436) | 1.0–1.3 |
| Before (`5c35b22`) | 844, **1845**, 733, 735, 744 ms (median 744) | 460, 808, 798, 796, 481 ms (median 796) | 1.2–1.5 |
| After | 201, 335, 315, 326, 311 ms (median 315) | 376, 325, 322, 382, 375 ms (median 375) | 0.9–1.2 |
| After, earlier run | 330, 302, 320, 331, 304 ms | 368, 353, 437, 394, 342 ms | 1.7–2.2 |

Scratch harness through `dev.sh`, two rounds of 5, builds interleaved:

| Build | herdr restart → reattached | Launch with herdr stopped | Load |
| --- | --- | --- | --- |
| 1.13 | 741–944 ms, median 849 | 316–469 ms, median 450 | 1.2–3.6 |
| Before | 773–1815 ms, median 862; **4 of 10 over 1.7 s** | 337–1315 ms, median 480; 4 of 10 over 800 ms | 1.1–2.3 |
| After | 207–333 ms, median 320 | 355–405 ms, median 398 | 1.4–4.1 |

`apps/macos/automation/restore.sh`, every step's checks passed:

| Step | Before (2 runs) | After (3 runs) |
| --- | --- | --- |
| Quit → relaunch | 281, 284 ms | 286, 289, 273 ms |
| kill -9 → relaunch | 290, 289 ms | 288, 282, 411 ms |
| Attach processes killed, then wake | 176, 62 ms | 170, 165, 64 ms |
| Same, no wake (1 s back-off) | 1112, 1102 ms | 1105, 1105, 1104 ms |
| herdr restart | 1825, 1821 ms | 198, 222, 203 ms |
| Launch with herdr stopped | 438, 824 ms | 397, 391, 392 ms |

Loads were 1.6–2.1.

- **Targets met.** Launch with herdr stopped is now 322–437 ms, under 1.13's 460–490. herdr restart reattaches in 198–335 ms across 23 runs, against the 1 s target and the 2 s NFR.
- **Relaunch and wake don't move.** With herdr already running, Local is connected from the start and tiles attach as before.

### Cold start

`scripts/nfr/coldstart.sh 10`. Medians in ms, from process start. First frame isn't measurable: the screen was locked, so it hit the 2 s fallback.

| Build | Load | Empty: core | Empty: layout | Restored: core | Restored: layout | Restored: terminals attached |
| --- | --- | --- | --- | --- | --- | --- |
| 1.13 | 3.0 | 73.6 | 138.1 | 82.8 | 229.2 | 217.6 |
| Before | 1.5–1.9 | 76.2 | 162.6 | 81.3 | 248.1 | 236.5 |
| After | 1.6–1.8 | 76.9 | 156.9 | 84.0 | 249.8 | 238.1 |

- **Core boot hasn't regressed.** It's within 4 % of 1.13. The e2e pass's +18 % was load.
- **Layout is ~20 ms slower than 1.13, and this work doesn't change that.** It's the new window chrome (sidebar, tab strip, cards, header strips, palette), built in the first refresh, with no single hotspot:
  - views ~5 ms
  - applying the window ~10 ms more
  - showing it ~7 ms more

  Opening the first `NSWindow` (~21 ms) costs the same in every build. The 500 ms cold-start NFR still has plenty of room.

### Also seen

herdr 0.9.1 sometimes takes ~100–110 ms to pick up a new API connection. With a debug-logging server, our request was "received" 105 ms after we'd connected, while other clients got answers in between. When it hits the boot `ping`, core boot grows by that much. That's herdr's side; it's intermittent and the same in every build.

## Layout mirror pushes (ADR 0012)

macOS test environment, herdr 0.9.1, Nov 4, 2026. `cargo test -p azulejo-herdr --test live mirror_merges -- --ignored --nocapture`, 3 runs: a throwaway session, three one-pane tabs (a ticker in one, an attach client on another), then `Client::mirror` timed from the call to its read-back.

| Push | Writes | Run 1 | Run 2 | Run 3 |
| --- | --- | --- | --- | --- |
| Merge three one-pane tabs into one (migration) | 2 moves | 114.5 ms | 4.4 ms | 3.1 ms |
| Rebuild three panes into another shape through a staging tab, then zoom | 4 moves + 1 zoom | 4.4 ms | 4.4 ms | 3.1 ms |
| Nothing to do (export and compare) | 0 | 1.5 ms | 2.2 ms | 2.5 ms |

- The staging tab exists for the length of a rebuild, about 3–4 ms here: one TUI frame at most.
- Run 1's merge is the first API call after the attach client started; herdr's occasional ~100 ms connection pickup (see "Also seen" above) fits.
- Pushes wait for 150 ms of quiet per tab first (`mirror::PUSH_QUIET`), so a settled change reaches herdr 150–160 ms after the last model change.
