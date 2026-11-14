# herdr notes (PLAN.md step 0.2)

Probed on Nov 3, 2026 against **herdr 0.9.1** (protocol 22) on macOS 26, in an isolated session (`--session azj-probe`). Reproduce with:

```sh
scripts/herdr-probe.sh                         # one JSON line per check, cleans up after itself
AZJ_PROBE_OUT=/tmp/azj-probe scripts/herdr-probe.sh   # keep raw captures
AZJ_PROBE_TUI=1 scripts/herdr-probe.sh         # also run the TUI client (it connects to your saved SSH machines)
```

Helpers in `scripts/herdr-probe/`:

- `attach_pty.py` runs `herdr terminal attach` under a PTY. It stands in for libghostty or VTE: records the bytes, types keys and resizes the PTY.
- `stream.py` captures `terminal session observe|control` streams.
- `events.py` reads the event subscription.
- `passthrough.sh` and `passthrough_probe.py` hold the escape-sequence cases.

**Version note.** Docs are at 0.9.5 (Oct 14, 2026), but **0.9.1 is installed**. Since then, 0.9.2 added `terminal.mouse` for controllers and changed how images reach a local Ghostty (temp files instead of terminal output). It also added `TERM_PROGRAM=herdr`, an `events_lost` error for slow subscribers, and logind-safe layout snapshots. 0.9.5 fixes prefix keys under macOS IMEs and Ghostty `text:` binds. **Recommendation: update to 0.9.5 before spike 0.3 (`herdr update`, outside herdr) and re-run this probe.** Nothing here needed a newer version. I didn't update herdr.

## Mechanics in one paragraph

`herdr terminal attach <terminal-id>` is a normal terminal program. On start it switches its host terminal to the alt screen and enables mouse (1000/1002/1003/1015/1006) and bracketed paste. It then streams herdr's **re-rendered** screen: synchronised-output (`?2026`) frames using absolute cursor addressing and no line feeds, starting with a full repaint. Keys typed into it go straight to the pane. The PTY size you give it becomes the pane's size. `terminal session observe|control` streams the same frames as NDJSON (`{"type":"terminal.frame","seq","full","width","height","encoding":"ansi","bytes":<base64>}`). Raw PTY bytes are never exposed.

---

## 1. Escape-sequence passthrough through `herdr terminal attach`

**Answer.** Cell content and styling pass through; most out-of-band sequences don't.

| Sequence | Through attach | Notes |
| --- | --- | --- |
| Truecolour `38;2`, 256-colour `38;5` | ✅ | |
| Underline styles (`4:3` undercurl) | ✅ | Underline colour (`58`) not seen in frames |
| OSC 8 hyperlinks | ✅ | Every frame resets with `]8;;` then re-emits links |
| Cursor shape (DECSCUSR `5 q`) | ✅ | |
| Kitty keyboard protocol push/pop (`>1u` / `<1u`) | ✅ | Forwarded to the host when the app pushes or pops |
| Mouse reporting | ✅ (differently) | The client enables all mouse modes on the host at start; herdr routes events to the app or uses them itself (scroll, selection) |
| Bracketed paste | ✅ (differently) | Enabled on the host at start; herdr wraps input only when the app enabled 2004 (seen: `pane run` text arrived as `\e[200~…\e[201~` once `/bin/sh` had 2004 on) |
| DECCKM (`?1h`, application cursor keys) | ❌ | Not forwarded, and herdr doesn't re-encode: the app got `1b 5b 41` for Up while in DECCKM mode |
| Focus reporting (`?1004h`) | ❌ | Not forwarded; apps never see focus in/out |
| OSC 52 clipboard | ❌ | Dropped |
| OSC 9 / 777 / 99 notifications, BEL | ❌ | Dropped, and no event fires either (see Q5) |
| OSC 0/2 title | ❌ in stream | Shows up as `terminal_title` in a `pane.updated` event |
| OSC 7 cwd | ❌ | `cwd` / `foreground_cwd` on the pane come from the process, not OSC 7 |
| Kitty graphics (APC `_G`) | ❌ on 0.9.1 with `TERM=xterm-256color` | 0.9.2+ sends images to a *local Ghostty* through temp files; retest with real Ghostty in 0.3 |

**Evidence.** `probe.py` checks `1_observe_passthrough` and `1_attach_host_bytes`. The attach client never queries its host terminal (no DA1, XTVERSION or kitty queries), so the fake PTY didn't hide any capability detection.

**Implication.**

- Rendering fidelity is fine for agents and TUIs.
- Azulejo has to source **title, cwd, attention and clipboard** from herdr's API and events, not from libghostty callbacks.
- OSC 52 being lost is the one users will notice: agents and vim can't copy to the system clipboard. Track it upstream; meanwhile `pane.selection.read` covers herdr-side selections.
- DECCKM and focus reporting need checking with vim and Claude Code in spike 0.3. Most apps accept both arrow encodings.

## 2. Scrollback

**Answer.** Scrollback lives only in herdr. The attach output contains **zero line feeds** and runs on the alt screen, so the host terminal (libghostty) never builds its own scrollback. Mouse-wheel events sent into the attach client scroll herdr's buffer: 5 notches gave `offset_from_bottom: 15` (3 lines per notch, `[ui] mouse_scroll_lines`). The frames then repaint the scrolled view. The scroll offset persists after the client detaches.

**Evidence.** Check `2_scrollback`: `line_feeds_in_attach_output: 0`, `scroll: {offset_from_bottom: 15, max_offset_from_bottom: 340, viewport_rows: 30}`.

**Implication.**

- libghostty's own scrollback, search and "jump to prompt" won't work in terminal tiles.
- Scrolling has to go through herdr: wheel events through attach work today; for keyboard scrolling use `pane.scroll` or controller `terminal.scroll`.
- Find-in-scrollback has to use `pane.read --source recent` or `pane.copy_search`.
- F20 "read scrollback" maps cleanly to `pane.read`.
- Scrollback survives an app restart but not a herdr *server* restart, unless `[experimental] pane_history = true`.

## 3. Two writers and take-over

**Answer.**

- **One direct-attach writer per terminal.** A second `terminal attach` without `--takeover` fails immediately: exit 1, `terminal attach failed: terminal … already has an attached client; retry with --takeover`.
- **With `--takeover`** the new client wins. The old client exits 1 with `terminal attach taken over`, and the pane resizes to the new client's PTY (30 → 24 rows).
- **Observers coexist** with a writer. Each gets frames rendered at *its own* `--cols/--rows` (60×20 while the writer was 100×30), so a read-only fallback is cheap.
- **The TUI client** (what a phone runs over SSH) isn't a direct-attach writer. A direct attach next to an open TUI succeeded without `--takeover`. While attached, it sets the pane's PTY size (TUI 44 rows → 24 while attached → back to 44 after). The TUI keeps drawing the pane in its own layout slot. I didn't confirm what that looks like: run once with `AZJ_PROBE_TUI=1`.

**Evidence.** Checks `3_two_writers` and `3_tui_and_direct_attach` (TUI run once manually).

**Implication.**

- The size fight with a phone isn't a lock fight. Whoever direct-attaches owns the size. A phone in the TUI sees Mac-sized content while the Mac tile is attached.
- There's no "you lost ownership" event. The only signal is the client exiting with that stderr line.

## 4. Event latency

**Answer.** Well under the 1 s target for F15. From the start of a `pane.split` RPC to receipt of `pane.created` takes 46–148 ms, including spawning the shell. `pane.close` → `pane.closed` takes ~80 ms. Each change also emits `layout.updated` (with rects) and `pane.updated`.

**Evidence.** Check `4_event_latency_ms`: `[147.7, 133.0, 99.4, 86.0, 56.0, 46.5, 117.2, 107.2, 85.6, 73.5]`.

**Implication.**

- No polling needed.
- `pane.agent_status_changed` and `pane.scroll_changed` subscriptions **need a `pane_id` each**, so the adapter subscribes per terminal tile. `pane.updated` carries an `agent_status` field, but a reported status change (`pane report-agent`) doesn't fire it: only the per-pane subscription sees it (re-probed for ADR 0009). Shell exit sends only `pane_exited`; `workspace_closed` carries the whole workspace object.
- A subscription streams only what happens after herdr accepts it, and changing the pane set means a new subscription. A status that changes before acceptance never streams; the snapshot has it straight away (re-probed Nov 3, 2026: `report-agent` then `api snapshot` agree at once). So whenever an agent subscription goes live (first open, reconnect, resync, or replacing the old set) the core reads every pane's status from a survey, statuses only, with the stale-survey guard.
- 0.9.2+ reports `events_lost` when a reader falls behind; the adapter must resync with `session.snapshot`.

## 5. Notifications (F11)

**Answer.** No public event carries pane notifications. OSC 9, OSC 777, OSC 99 and BEL produce no event, appear in no snapshot field, and aren't forwarded through attach. herdr's notifications are agent-state based: toasts and sounds on `blocked` and `done` (`[ui.toast]`, `[ui.sound]`). `notification.show` sends a toast out; it doesn't report one.

**Evidence.** In check `1_observe_passthrough`, the osc9, osc777, osc99 and bell cases all have empty `events`. `session.snapshot` has no notification field.

**Implication.**

- F11 has to be built on `pane.agent_status_changed` (`blocked` → attention, `done` → soft badge), which the PRD already wants for F16.
- Generic "command finished" OSC 9 notifications from non-agent programs are lost. Raise an upstream request for a `pane.notification` event, or drop that part of F11.

## 6. Mapping app tiles to herdr

**Answer.** **One herdr tab per terminal tile**, inside the herdr workspace mapped 1:1 to the app workspace.

- **Mirroring splits with `layout.apply` would kill processes.** Applying a layout to an existing tab created a *new* tab (t4 → t5) with fresh panes and closed the original. It ignored the `pane_id` I passed, so the original pane and its process were gone.
- **Pane size doesn't come from herdr's layout anyway** while a direct client is attached (Q3), so mirrored splits buy nothing on the Mac.
- **On a phone**, herdr switches to a single-column mobile layout below `mobile_width_threshold` (64 cols). One tab per terminal reads naturally there: a tab strip of agents.
- `tab create --label` gives each tab the tile's title, and `--env` passes the tile env vars (Q8).

**Evidence.** Check `6_layout_apply`: `original_still_exists: false`, `requested_tab: w1:t4`, `result_tab: w1:t5`.

**Implication.**

- Never call `layout.apply` on a tab with live panes.
- Use `pane.move` (or `tab.move` / `tab.rename`) to adopt panes created elsewhere (F15).
- If someone splits a herdr tab from the phone, the new pane shows up as a new tile; Azulejo doesn't need to mirror herdr's split geometry.

## 7. Detach key

**Answer.**

- `ctrl+b q` detaches a direct-attach client, which exits 0.
- `ctrl+b ctrl+b` sends a literal `ctrl+b`.
- `ctrl+b` followed by anything else passes both bytes through (`02 78` reached the app).
- The help string is baked into the binary: "detach from direct attach with ctrl+b q; send literal ctrl+b with ctrl+b ctrl+b".

It probably follows `[keys] prefix` and `[keys] detach` in the shared `~/.config/herdr/config.toml`. I didn't test that, because changing that file would change your main session. I found no per-session config and no flag or env var to turn the key off.

**Evidence.** Check `7_detach_key`: `app_received: 02 78 02 61 1b 5b 41 7a…`, `ctrl_b_q_exit_status: 0`.

**Implication.** tmux users (prefix `ctrl+b`) lose `prefix q` inside a tile, and an accidental `ctrl+b q` drops the tile. Mitigations:

- (a) Treat exit 0 with no app request as an accidental detach and reattach silently.
- (b) Ask upstream for `terminal attach --no-detach-key`.
- (c) Use controller mode, which has no detach key (see Recommendations).

## 8. `--env` on creation

**Answer.** Works on both `tab create --env` and `pane split --env`; the shell sees the variables. Panes also get `HERDR_ENV=1`, `HERDR_PANE_ID`, `HERDR_TAB_ID`, `HERDR_WORKSPACE_ID` and `HERDR_SOCKET_PATH`, plus `TERM=xterm-256color` (not `xterm-ghostty`). `layout.apply` pane nodes also accept `env`, `cwd` and `command`.

**Evidence.** Check `8_env`: `AZJENV=tile_tab TERM=xterm-256color` and `AZJENV=tile_split …`.

**Implication.**

- `AZULEJO_TILE_ID`, `AZULEJO_WORKSPACE_ID` and `AZULEJO_SOCKET` can be injected as planned.
- `HERDR_PANE_ID` works as the CLI fallback.
- `TERM` inside panes is herdr's choice, not Ghostty's, so terminfo-dependent features follow herdr's emulator.

## 9. Terminal ID ↔ pane ID

**Answer.** Every pane object (`pane.get`, `pane.list`, `tab.create` → `root_pane`, `pane.split` → `pane`, the `pane.created` event) includes `terminal_id` (`term_…`) next to `pane_id` (`w1:p1`). Attach takes the terminal ID; observe and control take the pane ID.

The two IDs behave differently:

- **Pane IDs** are never reused, but they **change when a pane moves to another workspace**.
- **Terminal IDs** didn't change during the probe. Persisting them across moves is untested.

**Evidence.** Check `9_terminal_id`.

**Implication.** A terminal tile stores both IDs. It re-resolves `pane_id` from `terminal_id` (via `pane.list`) after any `pane.moved` event and on relaunch.

## Attach client exit reasons (bonus, needed for 1.7 and 1.12)

| Situation | Exit status | Last stderr line |
| --- | --- | --- |
| User pressed `ctrl+b q` | 0 | none |
| Another client took over | 1 | `herdr: server shut down: terminal attach taken over` |
| Second client without `--takeover` | 1 | `… already has an attached client; retry with --takeover` |
| Pane closed or process exited | 1 | `… terminal attach ended: terminal term_… not found` |
| herdr server stopping | 1 | `herdr: server shut down: server is shutting down` |

The client always restores the host terminal (leaves the alt screen, turns mouse and paste off, resets the cursor) before printing the message. Note that every message says "server shut down", even when the server is still running.

---

## Recommendations (ADR candidates)

### ADR: terminal bridge

**Decision: run `herdr terminal attach <terminal-id>` as the libghostty or VTE surface's child process (no custom bridge) for M0 and M1. Keep controller mode as the planned upgrade path.**

- **Why attach.** It's zero code. libghostty runs it like any program: PTY resize becomes pane resize, and keys, mouse and paste work. Rendering fidelity is good, with synchronised output so there's no tearing.
- **What it costs.** No host-side scrollback (Q2); OSC 52, notifications and DECCKM/focus are lost (Q1); a detach key that can't be turned off (Q7); exit-code-based state (exit table).
- **Controller mode** (`herdr terminal session control --takeover`, NDJSON `terminal.input` / `resize` / `scroll` / `mouse` / `release`) has none of the detach-key or exit-parsing problems, gives explicit `terminal.closed` records, and works for observers too. It needs a small adapter: decode frames and feed them to the surface, encode keys out. On macOS that means feeding bytes into libghostty without a PTY, which the embedding API may not support. Decide in spike 0.3. On Linux, with libghostty-vt in M4, controller mode is the natural fit.
- **Measure both** in spike 0.4 (latency). If attach is within budget, ship it first.

### ADR: tile ↔ herdr mapping

**Decision: app workspace ↔ herdr workspace (1:1); each terminal tile ↔ its own herdr tab with one pane, labelled with the tile title and created with `--env` tile IDs.**

- Never use `layout.apply` on live tabs (it kills panes).
- Adopt externally created panes as new tiles (F15).
- Moving a pane between tabs or workspaces keeps its process; re-resolve the pane ID from the terminal ID after `pane.moved`.
- Subscribe per pane for `agent_status_changed`.

### ADR: take-over behaviour

**Decision: the newest *direct-attach* client wins; losers fall back to read-only.**

- **On first attach**, try without `--takeover`. If it says "already has an attached client", show the tile read-only through `terminal session observe` at the tile's size, with a **Take over** action that re-attaches with `--takeover`.
- **When the app loses a tile** (exit 1 "taken over"), switch that tile to observe mode with **Take back**. Don't auto-retake, or two Azulejo windows or machines will ping-pong.
- **Phones using the TUI client** don't fight for the lock. The Mac tile keeps input and size while attached. Note that this makes the phone render Mac-sized panes.
- **Make the switch explicit in the UI**, so the user can hand a pane to the phone: "Release to phone" detaches the tile and lets the TUI's size take over.
- **Watch for** a dedicated phone app that uses direct attach (herdr-phone wasn't available to test). It would hit this same take-over flow.

### Upstream asks (herdr)

1. A `pane.notification` event for OSC 9/777/99 and BEL (F11).
2. OSC 52 passthrough or a clipboard event for attach and controller clients.
3. `terminal attach --no-detach-key` (or a configurable detach key per attach).
4. Forward DECCKM and focus reporting (1004), or re-encode keys server-side.
5. `layout.apply` that reuses live `pane_id`s instead of replacing the tab.
6. Clearer exit messages than "server shut down" for take-over and pane-ended cases (or a JSON exit reason).

### Not determined

- What the TUI (phone) draws while a direct-attach client forces a smaller pane size. Run `AZJ_PROBE_TUI=1` and look, ideally on 0.9.5.
- Kitty graphics through attach when the host is a real Ghostty (0.9.2+ path). Retest in spike 0.3.
- Whether `[keys] prefix` / `detach` change the direct-attach detach key (not tested, to keep your config untouched).
- Whether `terminal_id` survives `pane.move` across workspaces.
- How herdr-phone connects (direct attach, TUI over SSH, or the private client-shell surface protocol behind `client_shell.surface.set`, which isn't documented for third parties).
