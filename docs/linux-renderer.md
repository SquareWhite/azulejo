# Linux terminal renderer (spike 0.7)

Nov 3, 2026. Input for PLAN.md steps 3.2 (VTE stopgap) and 4.2 (libghostty-vt renderer).

## TL;DR

- **VTE path works today.** A gtk4-rs window with a VTE widget running `herdr terminal attach <id>` renders herdr panes on Linux. See "Spike results" for what was verified.
- **libghostty-vt is a full terminal *state* engine, not a renderer.** It parses VT, keeps the grid and scrollback, does selection, search, key and mouse encoding, kitty graphics state and a dirty-tracked render state. It does **not** do fonts, shaping, glyph rasterisation, a glyph atlas or any GPU work.
- **Our renderer is mostly a font + GL project.** Estimate **6 to 9 weeks** for one developer to reach "better than VTE" for daily use, and **10 to 12 weeks** for Ghostty GTK parity (ligatures, box drawing sprites, kitty graphics, latency parity).

## What libghostty-vt gives us

Pinned to Ghostty `0d1d5e88` (main, Nov 3 2026). Built with `zig build -Demit-lib-vt` (Zig 0.17.0) → `libghostty-vt.so` / `libghostty-vt.a` + `include/ghostty/vt.h`. ~216 exported C functions. The API is functionally stable but signatures are still marked "in flux", so we pin a commit.

| Area | Header | What it covers |
| --- | --- | --- |
| Terminal state | `terminal.h` | new / free / resize / reset, `vt_write` (feed bytes), cursor, active screen, modes, title, pwd, colours and palette (with OSC overrides), scrollback row counts, viewport scrolling, memory usage and compression |
| Render state | `render.h` | Snapshot of the viewport for a renderer: row and cell iterators, **global and per-row dirty tracking**, two-phase update (lock → begin → unlock → end), synchronised-output holds, image iterators |
| Cells and styles | `screen.h`, `grid_ref.h`, `style.h` | Per-cell codepoints and graphemes, style (fg/bg/underline kinds/attrs), **hyperlink URI per cell (OSC 8)** |
| Selection | `selection.h` | word / line / output / all selection, adjust, contains, format selection to text, and a **gesture state machine** (click, double, triple, drag) |
| Search | `search.h` | Incremental search over screen + scrollback |
| Input encoding | `key/encoder.h`, `mouse/encoder.h`, `focus.h`, `paste.h` | Key → bytes including the kitty keyboard protocol; mouse reporting modes; focus events; bracketed paste |
| OSC | `osc.h` | OSC parser: notifications (9/99/777), clipboard (52), cwd (7), etc. |
| Graphics | `kitty_graphics.h` | Kitty image storage, placements, placement rects in grid and pixels, animation ticks |
| Formatting | `formatter.h` | Dump screen/selection as plain text, VT or HTML |
| Snapshots | `snapshot.h` | Encode/decode a terminal state snapshot (handy for restore and tests) |

## What we have to build

| Piece | Notes | Effort |
| --- | --- | --- |
| GTK widget + GL context | `GtkGLArea` (GL 3.3 core / GLES 3), frame-clock driven redraw only when dirty, HiDPI scale factor, resize → `ghostty_terminal_resize` + PTY `TIOCSWINSZ` | 0.5 wk |
| Font discovery and fallback | fontconfig: primary family from Ghostty config, fallback chain for missing glyphs and emoji; DPI and size | 1 wk |
| Shaping | HarfBuzz runs per row, split on style and font; ligatures; grapheme clusters from the cell graphemes | 1 to 1.5 wk |
| Rasterisation + atlas | FreeType rendering (grayscale + subpixel off by default), colour emoji (CBDT/COLRv1 via FreeType), texture atlas with LRU eviction | 1 to 1.5 wk |
| Cell pipeline | Instanced quads for backgrounds and glyphs, underline styles, strikethrough, cursor shapes, selection overlay, search highlights | 1 wk |
| Box drawing / powerline sprites | Ghostty draws these procedurally (`src/font/sprite`) for pixel-perfect joins; port or reimplement | 0.5 to 1 wk |
| Input | `GtkEventControllerKey` → libghostty-vt key encoder (kitty protocol); `GtkIMContext` for IME preedit/commit; mouse → mouse encoder; scroll; focus | 1 wk |
| Selection + clipboard + links | Gesture API → selection; copy/paste (primary + clipboard); OSC 8 + regex link hover and open | 0.5 to 1 wk |
| Ghostty config | Parse the subset that matters (font-family, font-size, theme, palette, cursor, padding); share the parser with the core | 0.5 wk |
| Kitty graphics | Upload placements as textures; z-ordering | 1 wk (can defer) |
| Latency + perf tuning | Dirty-row uploads, avoid full re-shape, bench vs Ghostty GTK (PLAN 4.3) | 1 wk |

**Total:** 6 to 9 weeks for a daily-drivable renderer without kitty graphics; 10 to 12 weeks for parity. Fits the M4 budget (6 to 10 weeks) only if kitty graphics and sprite polish slip to M5.

**Alternative worth considering in M4:** Ghostty's own font and renderer code (`src/font`, `src/renderer/OpenGL.zig`, `src/renderer/generic.zig`) is MIT and already solves shaping, the atlas and sprites against the same terminal state. Linking that Zig code into a small C ABI of our own (a "libghostty-render" shim) could cut 3 to 4 weeks, at the cost of tracking Ghostty internals that have no stable API. Also watch upstream: Mitchell has said more sibling libraries will follow libghostty-vt; a rendering library would replace most of the table above.

## Reference implementations

- **Ghostty GTK app**: `src/apprt/gtk`, `src/renderer/OpenGL.zig`, `src/font/*`. The gold standard; Zig, not reusable as a library today.
- **forgetty** ([vikgmdev/forgetty](https://awesome.ecosyste.ms/projects/github.com%2Fvikgmdev%2Fforgetty)): GTK4 terminal on libghostty-vt with a daemon that owns PTYs. Same split as Azulejo + herdr; claims pixel parity with Ghostty text. Read first.
- **Husk**: C++ Wayland terminal on libghostty with a daemon architecture.
- **Ghostty examples**: `example/c-vt-render` (render state API), `c-vt-selection-gesture`, `c-vt-encode-key`, `c-vt-kitty-graphics`.

## Risks

| Risk | Impact | Mitigation |
| --- | --- | --- |
| libghostty-vt C API signatures change | Build breaks on Ghostty bumps | Pin commit; one Rust `-sys` crate (bindgen) + thin safe wrapper; bump deliberately |
| herdr streams re-rendered frames, not the app's raw bytes | Our scrollback holds herdr's redraws, not the program's output; OSC passthrough depends on herdr | See spike results; use herdr `pane.read` / scroll for history if local scrollback is poor |
| Font rendering quality vs Ghostty | Users notice immediately | Copy Ghostty's FreeType load flags and metrics logic; side-by-side screenshots in CI |
| IME on Wayland | CJK users blocked | `GtkIMContext` from day one; test with ibus and fcitx5 |
| GL on odd drivers / no GL | Blank tiles | Fall back to VTE (`terminal.renderer = "vte"`) |

## Spike results

Reproduce with `spikes/linux-terminal/run.sh` (Docker, Ubuntu 24.04 arm64 on Docker Desktop for Mac). Pinned: herdr v0.9.5, Ghostty `0d1d5e88`, Zig 0.17.0, gtk4-rs 0.11, vte4 0.10 (VTE 0.76, GTK 4.14). The GTK app runs under Xvfb with `GSK_RENDERER=cairo`; screenshots in `spikes/linux-terminal/out/`.

**VTE + `herdr terminal attach` (PLAN 3.2 path): works.**

- Shell with `ls --color`, vim and htop render correctly (screenshots `01`–`04`, checked by eye): ANSI and 256 colours, box/bar glyphs, htop's function-key bar, cursor.
- Truecolour SGR, OSC 8 hyperlinks, CJK and emoji survive herdr's re-render (found in the frame stream).
- `--env AZULEJO_TILE_ID=…` on `workspace create` reaches the pane's shell.
- Claude Code inside the tile: **not tested** (no account in the container).

**How herdr's stream behaves (affects both platforms):**

- Attach and observe streams are **re-rendered damage frames**, not the program's bytes: each frame is wrapped in synchronised output (`CSI ?2026 h/l`), uses absolute cursor moves, and the first frame is a full repaint (`CSI 2J`). vim and htop never switch to the alt screen on our side.
- So **local scrollback stays empty** (libghostty-vt replay reports `scrollback_rows=0`). History lives in herdr. The attach client enables mouse reporting (1000/1002/1003/1006), so the **mouse wheel scrolls herdr's history**; verified via `pane get` (`offset_from_bottom: 45` of 273) and screenshot `06`. Keyboard scrollback (Shift+PgUp) and search must go through herdr (`pane.scroll`, `pane.read`, `pane.copy_search`), not the local terminal. Same applies to libghostty on macOS.
- `pane.read --source recent` returns about 80 lines; `visible` returns the viewport.
- **Zero-sized PTY kills attach** (`terminal reported a zero-sized grid`). Shells must size the PTY before spawning the attach process.
- **Takeover:** a second `terminal attach <id> --takeover` resizes the pane to the new client's grid and the **displaced attach client exits with status 1**, silently. It does not fall back to read-only. The pane and any observers survive. Azulejo must treat "attach exited, pane still exists" as "taken over" and show the take-back overlay (and optionally a read-only view via `terminal session observe`). After the taker leaves, the pane keeps the taker's size until we reattach.
- An attach client forwards every input byte, including a stray EOF (^D): a test harness that closed stdin ended the shell. The shells must never write EOF on detach.
- `--takeover` must come after the terminal ID: `herdr terminal attach <id> --takeover`.

**libghostty-vt (PLAN 4.2 path): state engine confirmed on Linux.**

- `zig build -Demit-lib-vt -Doptimize=ReleaseFast` builds `libghostty-vt.so`/`.a` on aarch64 Linux in a few minutes, no system deps.
- `spikes/linux-terminal/vtdump` feeds a captured `terminal session observe` stream (45 frames, 35 KB, including vim and htop) into `ghostty_terminal_vt_write` and dumps the grid with the plain-text formatter. The final grid matches herdr's `pane.read --source visible` (rows `282…300` + prompt; cursor `(2,19)`).
- Observer frames are NDJSON: `{"type":"terminal.frame","seq","full","width","height","encoding":"ansi","bytes":<base64>}`, and `{"type":"terminal.closed","reason"}` at the end.
