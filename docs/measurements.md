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
