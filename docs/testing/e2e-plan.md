# Azulejo end-to-end test plan (macOS, v1)

> **Re-run on Nov 4, 2026** ([report](e2e-report-2026-11-04.md)) against `main` at `947bb22`, after the five E2E-fix merges. New in this copy:
> - A9 adds `polish.sh` (visual polish captures), A10 adds `e2e/stale-check.sh` (screen-on check for V1; run it only with the screen unlocked).
> - Phase B/C cases N1–N6 (section 3a) cover the features the fixes added (⌘F in each tile kind, `azj preview open` from a terminal, error wording, tab overflow, the badge in the header, the config banner row) and the places where several fixes meet: the find bar under motion, a herdr restart, ⌘K and a tab switch. Script: `apps/macos/automation/e2e/fixes.sh` (`run-all.sh` runs it as `fixes`).
> - Phase D adds `scripts/nfr/reconnect.sh` (herdr restart and launch with herdr stopped, repeated).
> - The report gains a "Since the previous run" table: each earlier finding as FIXED, STILL OPEN or REGRESSED, with evidence.

Nov 3, 2026. Covers `main` after M2.5 (D.1–D.9) is merged. The goal is to check that everything in M1, M2 and M2.5 works together in the real app, and that it looks like the mockup. This is a verification pass: it finds and documents problems. It doesn't fix them.

> **Changes in this copy (E2E run, Nov 3–4, 2026).** Factual fixes only; the report says what each case found.
> - A1: `just lint` runs `cargo fmt --check`, clippy and the design-token check. The schema freshness check runs in `just test`, the Python codegen check in `just python-test`.
> - A9: `browser-relaunch.azs` runs inside `browser.sh`, `headers-relaunch.azs` inside `headers.sh`, `palette.azs` and `attention.azs` inside `chrome.sh`; they aren't separate runs.
> - B3: since D.4 the tab strip shows the layout kind in the tab's tooltip, not as visible text.
> - B5: the app has no setting that forces Reduce Motion (`reduce_motion` is `"system"` or `"ignore"`). The working ring and layout motion share one switch (`Motion.active`), so motion off (`[animation] enabled = false`, or `anim enabled false` in a script) stands in for it.
> - B6: the command is `azj preview open --path plan.md` (no positional path).
> - B14: `azj layout apply --path docs/examples/azulejo.toml`.
> - C5: the app has no appearance override, and `-AppleInterfaceStyle Light` in the app's argument domain doesn't override a Dark system setting.
> - Phase D: `scripts/nfr/lib.sh` now honours `AZJ_DEV_NAME` (it forced `nfr`), so the NFR scripts run on an `e2e-*` instance.
> - New scripts for the gaps live in `apps/macos/automation/e2e/` (see its `lib.sh`); `e2e/hd` is a herdr wrapper that refuses any session not named `azj-dev-e2e*`.

Out of scope: the Linux shell (M3), notarisation, real SSH machines, real sleep/wake, and anything that needs real keyboard input or windows in front. Those go on the "Needs the maintainer" list at the end.

## 0. Ground rules (non-negotiable)

the maintainer uses this Mac while the tests run. Breaking any of these rules is worse than skipping a test.

- **No system-wide input.** No `osascript` keystrokes, no CGEvent, no `cliclick`. All input goes through the in-process automation (`.azs` scripts, `delivery direct`) or the `azj` CLI.
- **Never activate or raise a window.** Launch with `--bg`. Screenshots come from in-app capture or from `screencapture -l <our window id>` of Azulejo dev windows only.
- **Dev profile only.** Use `AZJ_DEV_NAME=e2e` (or one name per suite, e.g. `e2e-vis`), which gives `.dev/<name>/` and herdr session `azj-dev-<name>`. Never touch the default herdr session, `/Applications/Azulejo.app`, the real Azulejo state or prefs, or `~/.config/azulejo`.
- **No real machines.** Remote tests use `scripts/ssh-standin.sh` only. Never connect to build-host, dev-server or any `~/.ssh` host.
- **No clipboard, keychain or Downloads.** Copy/paste tests use the private pasteboard the focus suite already uses. Downloads go to a temp folder.
- **Never run `herdr update`.**
- **Clean up.** Run `apps/macos/dev.sh --reset` (with the same `AZJ_DEV_NAME`) at the end, kill any `azj-dev-e2e*` herdr sessions you started, and delete the temp fixtures.

## 1. Setup

1. Work in your own worktree, branched from the latest `main`. Record the commit hash in the report.
2. `just build`, `just mac`. If either fails, stop and report: nothing else is meaningful.
3. Fixtures go in a temp folder (`mktemp -d`):
   - git repos `azulejo` (main), `sample-service` (development), `sample-project` (`audit/critical-path-review`, for truncation), plus one folder that isn't a repo
   - `plan.md`, a `.rs` file, a PNG, a short MP4 and a PDF
   - a 50k-file tree (generate it) for the files tile
   - the local site in `apps/macos/automation/browser-site/`, served on a free localhost port
4. Reuse the existing automation where it already covers a case. Write new `.azs` or shell scripts only for the gaps, and keep them under `apps/macos/automation/e2e/` so they can be re-run.

## 2. Phase A: existing automated suites

Run each one and record pass/fail, duration and any flake. Re-run a failure up to three times on its own before calling it a fail, and note the pass rate.

| ID | Command |
| --- | --- |
| A1 | `just lint` (fmt, clippy, design-token freshness; the schema check is in `just test`, the Python codegen check in `just python-test`) |
| A2 | `just test` |
| A3 | `just test-herdr` |
| A4 | `just e2e` |
| A5 | `just python-test` |
| A6 | `just mac-automation` (keys.azs) |
| A7 | `just mac-layouts` |
| A8 | `just mac-focus` (38+ scenarios) |
| A9 | `apps/macos/automation/{browser,preview,files,restore,headers,machines,sidebar,menus,toolbar,chrome,polish}.sh`, `focus/focus.sh` (47 scenarios since ⌘F: S39–S47), and the `.azs` scripts not covered above (attention and palette run inside `chrome.sh`; browser-relaunch and headers-relaunch inside `browser.sh` and `headers.sh`). Run `polish.sh` with an `e2e-*` `AZJ_DEV_NAME` (its default is `polish-g`) |
| A10 | `apps/macos/automation/anim.sh` (crop stress): 0 dropped ticks expected, exact PTY resize counts, no `STALE` capture. With the screen unlocked only: `apps/macos/automation/e2e/stale-check.sh` (every reveal from the display link, a first frame at every new size, no stale capture) |
| A11 | `just test-dogfood`, `just test-cli-installer` (only if they stay inside temp dirs; check the recipes before running) |

## 3. Phase B: user journeys

Each journey runs on a fresh dev profile, driven in-process and through `azj`. Every step needs evidence: an `azj ... --json` output, an automation log line or a window capture. Timings are measured, not eyeballed.

### B1. First launch and adoption
- Start a dev herdr session with two workspaces and three panes **before** launching the app.
- Launch: adopted workspaces and tabs show up, adopted tabs are `manual`, and there's no stray placeholder workspace.
- Cold start to first frame within 500 ms (`scripts/nfr/coldstart.sh` style).

### B2. Keyboard tiling (every default chord)
- Walk every `[keys.macos]` binding in `shared/config/keymap.default.toml` through the key router. Each must produce the expected command (`key.last`) and the expected layout.
- `cmd+a` toggles the sidebar only when a terminal has focus. In browser and preview tiles it's select-all.
- `ctrl+b` typed in a terminal doesn't detach the attach client.
- `alt+w` detaches: the herdr pane still exists (`herdr pane list` on the dev session).
- `ctrl+d` in a shell closes the tile with **no confirmation dialog**.
- `resize_mode`: hjkl is sticky until Esc.

### B3. Dynamic layouts (ADR 0010)
- A new tab is dynamic Tile. the maintainer's tall flow, keystroke for keystroke: split chord ×3, `alt+enter`, `alt+o`, `alt+shift+e`, resize.
- Cycle Tile → Monocle → Three Column → manual and back. Each changed terminal gets exactly one PTY resize; Monocle focus switches get none.
- Masters `alt+=`/`alt+-`, master area `alt+shift+=`/`alt+shift+-`, move position, rotate part.
- The tab bar shows the layout name and hint (since D.4: in the tab's tooltip).

### B4. Animations
- Slow-motion captures (`anim.sh --slow`) of split, close, swap, resize, zoom and a layout switch. Content stays 1:1, never stretched. No row bleeds past a shrinking edge: check specifically for the vim `~` row artefact. Headers and card corner caps move with the tile.
- With `[animation] enabled = false`, changes are instant.
- Try `style = scale` and `style = outline` once each for sanity.

### B5. Agent supervision (F11, F16)
- Four terminal tiles. Use `herdr pane report-agent` on the dev session to set one blocked, one working and one idle.
- Blocked shows within 1 s on the tile badge, tab, sidebar row, footer count and Dock badge. Working shows the partial-ring icon rotating; with Reduce Motion (simulated by turning motion off, the same switch; not the system pref) it's static.
- `tile.seen` clears attention.
- Subscription churn (open and close tiles while statuses change) loses no status.

### B6. Live preview and plan review
- A browser tile on the localhost fixture beside a terminal: back/forward, `⌘L`, reload, `target=_blank`, a download into a temp folder, and the URL kept after relaunch.
- From inside a terminal tile, using `$AZULEJO_TILE_ID`: `azj preview open --path plan.md`, then edit the file from the terminal. The preview updates within 200 ms.
- Preview | Source toggle: Source shows highlighted markdown, and the choice survives a relaunch.
- PNG, MP4 and PDF preview (QuickLook, checked with a window capture).

### B7. Files
- Files tile on the 50k-file tree: first rows within 300 ms. External create, rename and delete show up within 1 s. Git badges, keyboard navigation, open in preview, open a terminal at a path.
- Sidebar Files panel: follows the focused workspace's root. Opening a file lands beside the focused tile, which keeps focus. Esc hands the keyboard back.

### B8. ⌘K quick open and toolbar search
- `⌘K` and a click on the toolbar field open the same menu, anchored to the field. The key badge shows the bound chord, and changes if the binding changes.
- `>` scopes to commands and `@` to places. Every command in `commands.list` is reachable. Jumping to a workspace, tab or tile works, and remote rows show their machine.
- Focus: a herdr refresh or attach while the menu is open doesn't steal typing.

### B9. Persistence and reconnect (F24)
- With 6 terminals (one running a ticker), a browser and a preview: quit and relaunch, kill -9 and relaunch, simulated wake (debug hook), and a herdr restart of the **dev** session.
- Each one reattaches within 2 s. It's the same herdr terminal (same id, ticker still counting), and URLs, preview paths and preview modes all come back.
- A pane killed while the app is closed shows "Session ended" with Respawn and Close, and both work.

### B10. Changes made outside the app (F15)
- `herdr pane split` and `herdr pane close` on the dev session: the tile appears or goes within 1 s.
- An `azj` script from another terminal builds a two-workspace layout end to end.

### B11. Multi-machine (stand-in only)
- Local, the stand-in remote and an `off` machine sit side by side in the sidebar.
- "+" on the remote group runs a shell on the stand-in.
- Cutting the link shows "Reconnecting…" (amber spinner, rows dimmed) on that group only, and tiles reattach when it comes back.

### B12. Git branch per workspace
- A checkout in a tile updates the sidebar within 1 s. Detached HEAD shows `@sha` and a dirty tree shows `•`.
- A workspace outside a repo shows no branch, and a long branch is truncated on the right.

### B13. Config hot reload
- Edit the dev profile's config to rebind a chord: it works without a restart.
- Save a broken file: a non-blocking banner appears, the app doesn't crash and the old bindings stay.
- Theme and animation keys reload too.

### B14. API, CLI and Python
- The socket is mode 0600 and its directory 0700.
- `azj events --follow` streams tile, focus and agent-state events with stable JSON, and filters work.
- Inside a tile, `AZULEJO_TILE_ID` and `AZULEJO_WORKSPACE_ID` are set, and `azj` resolves the caller.
- Tile A drives tile B with `azj terminal send-text`, and `azj terminal read` gets the text back.
- `azj layout apply --path docs/examples/azulejo.toml`, plus a `three_column` layout file, then `layout export` round-trips.
- The Python sync and async clients do one create/read/close against the dev socket.

### B15. Stability
- Kill the WebContent process and the QuickLook process. Terminal tiles keep running, and the crashed tile shows its banner and recovers.

## 3a. New features and where the fixes meet (added Nov 4)

`apps/macos/automation/e2e/fixes.sh` (dev instance `e2e-fix`) runs N1–N6 in-process, then relaunches the app in the background for the `azj` checks. Every capture goes next to the azs log; look at them, don't just count PASS lines.

### N1. ⌘F find bar
- N1a: ⌘F in a terminal, type a query, then a layout cycle and a swap in slow motion with the bar open. The bar keeps its query and the keyboard; capture mid-motion: the bar, header strip and badge ride with the card.
- N1b: stop and restart the dev session's herdr under an open bar. The tile shows `waiting`, reattaches, and the bar keeps the keyboard; typing doesn't reach the PTY.
- N1c: ⌘K over an open bar, then Esc. The keyboard goes back to the bar.
- N1d: a tab switch and back with the bar open; the bar is still open.
- N1e: ⌘F in a preview's Source mode finds in the source.
- N1f: ⌘F in a Quick Look preview (PNG) flashes that find isn't available there; no bar opens.
- N1g: ⌘F on a failed browser page: the error banner and the bar stack without overlap.
- N1h: the bar at the 480×320 minimum, and in a narrow tile: nothing overlaps.
- S39–S47 in `focus.sh` cover the basics in each kind (terminal, browser, preview, files filter).

### N2. `azj preview open` from a terminal
- From inside a terminal tile (`$AZULEJO_TILE_ID`): `azj preview open --path plan.md`, again with `main.rs`, and with a missing file. Each exits 0; record where each lands and whether focus stays in the terminal. The missing file shows "File not found" and waits for the file.
- `azj preview open --tile <id>` on a terminal, a preview and a browser: a new tile beside non-previews, the same tile for a preview.

### N3. Error wording
- HUD: focus, resize and swap with no neighbour say "No tile to the right", "No divider to the right to move", in sentence case, with no IDs.
- `azj`: unknown param (top level and nested), params on a method that takes none, no such tile and workspace, the wrong tile kind, a bad enum value, an unknown method. The message names no raw ID; the IDs follow it as data. Note whether `--json` errors are JSON.

### N4. Tab overflow
- 14 tabs: the strip scrolls, edges with hidden tabs fade (`tab.overflow`), the selected tab scrolls into view (`ctrl+4`, `ctrl+9`).
- A tab with attention scrolled out of view: is it signalled at the edge?
- At 480×320 (with the config banner up). Closing back to 4 tabs: `tab.overflow` is `none`.

### N5. Agent badge in the header
- Working, blocked, done (working → idle on an unfocused tile) and none, side by side: each badge in its header strip's right end, title clear of it.
- At 760×480: narrow tiles show the dot alone.
- A slow swap of a tile with a badge: the badge rides with the header strip.

### N6. Config banner row
- A broken config with a badge and an open find bar: the banner takes its own row under the toolbar, the tiles move down, the bar keeps the keyboard.
- With 14 tabs and at 480×320. Fixing the file removes the banner and the tile area (`chrome.tiles`) comes back to where it was.

## 4. Phase C: does it look right

Compare against `docs/design/mockup-v1.png`, the region notes in `docs/design/README.md`, and D.9's `capture-v1.png` / `compare-v1.md`.

- **C1. Mockup scene.** Run `apps/macos/automation/mockup.sh` and capture at 1440 × 900. Go region by region (chrome, sidebar, tab strip, each tile kind, footer) and list every visible difference not already in `compare-v1.md`.
- **C2. Tokens.** No hard-coded colours, radii or fonts outside `Design.swift` / `DesignSupport.swift`. Grep for `NSColor(`, `.cgColor` literals, `cornerRadius =` with a number, and `NSFont.systemFont(ofSize:` with a literal.
- **C3. State gallery.** Capture each state and check it against the tokens and the mockup:
  - sidebar rows: idle, working, blocked, done, selected, hover, and a long name or branch
  - machine groups: connected, connecting, reconnecting and off
  - tabs: selected, unselected, attention, many tabs (overflow)
  - each tile kind: focused and unfocused
  - browser chrome: http, https and editing
  - preview: Preview and Source modes
  - files tile and the Files panel
  - the config error banner, the "Session ended" overlay and the read-only take-over overlay
  - ⌘K menu: empty, filtered, `>` and `@`
- **C4. Edge layouts.** Minimum window size, 1 tile, 9+ tiles, Monocle, Three Column, an empty workspace and an empty tab. Look for clipped text, overlapping controls, headers wider than their tiles and broken corner caps.
- **C5. Appearance.** The theme is dark-only for now. Launch with the system set to light (through the app's own override or `-AppleInterfaceStyle`, never the system pref): nothing becomes unreadable.
- **C6. Motion polish.** From the slow captures in B4: the focus indicator morphs smoothly, tab switches fade, and nothing flashes white or black between frames.

Rate each finding **blocker**, **major**, **minor** or **designer question**.

## 5. Phase D: performance spot-check

Compare against `docs/measurements.md`, and flag anything more than 10% worse:
- cold start with 8 tiles restored (`scripts/nfr/coldstart.sh`)
- memory with 4 terminals (`scripts/nfr/memory.sh`)
- crop stress dropped ticks and PTY resize counts (A10)
- preview reload latency (B6)
- herdr reconnect, repeated (`scripts/nfr/reconnect.sh 5`): herdr restart → reattached, launch with herdr stopped

The machine is busy, so record the load average next to each number.

## 6. Report

Write `docs/testing/e2e-report-2026-11-03.md`:
- the commit tested, the machine and the date
- a summary table: case ID, PASS / FAIL / FLAKY / BLOCKED / SKIPPED, evidence path and one line of notes
- every failure with exact repro steps, expected vs actual, logs and a capture
- every visual finding with its severity and a capture, cropped to the region where that helps
- the performance numbers against the baseline

Put the screenshots worth keeping under `docs/testing/e2e-v1/` (PNG, kept small). Raw output stays in `.dev/e2e*.out/`.

Commit the plan, the report and any new `apps/macos/automation/e2e/` scripts on your branch. **Don't change product code.** If a test script itself is wrong, fix it and say so.

## 7. Needs the maintainer (manual checklist)

Things an agent can't check under the ground rules. Add anything else you hit along the way.

- [ ] Real typing feel and real IME (Japanese/Chinese input) in a terminal
- [ ] System clipboard copy/paste, mouse selection and scroll wheel in terminals
- [ ] Claude Code's interactive keys (shift+enter, esc-esc, ctrl+r) in a tile
- [ ] Animations with the window in front on the ProMotion display (feel, not numbers)
- [ ] Real sleep/wake reattach
- [ ] Real remote machines (build-host, dev-server) and remote git branches
- [ ] Sharing a pane with the herdr TUI
- [ ] Standalone Ghostty.app latency and memory baselines
- [ ] Notarisation, once a Developer ID exists
