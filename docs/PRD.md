# PRD: Azulejo, a tiling terminal workspace for macOS and Linux

Nov 3, 2026 · project maintainers · Updated Nov 3, 2026: added Linux; Gate G0 findings (see [ADRs](adr/README.md))

## Summary

Azulejo (working name) is a native workspace for macOS and Linux. Every tile is a terminal, a browser, a file preview or a file browser, arranged with keyboard-driven tiling and Hyprland-style motion. Terminal tiles are views onto herdr panes, so sessions persist, can be reached from a phone, and survive app restarts. Everything the user can do is also exposed through a CLI and a Python API, so coding agents can drive the workspace themselves.

A shared Rust core holds the layout, API, config and herdr adapter. A thin native shell per platform draws it: Swift and AppKit on macOS, GTK4 on Linux. No Electron, and no terminal rendered inside a webview.

Status: draft for discussion. Out of scope: an embedded code editor (Zed), tmux sync and Windows.

## Problem and context

No single tool combines rich tile types, keyboard-first tiling with motion, real session persistence and an agent-facing API. Developers running several coding agents end up juggling a terminal, a multiplexer, a browser and a previewer as separate windows.

| Tool | What it does well | What's missing for us |
| --- | --- | --- |
| [cmux](https://github.com/manaflow-ai/cmux) | Native Swift app on libghostty; terminal, browser and markdown panes; CLI and socket API | macOS only; no file browser tile; no Hyprland-style motion; does not restore live processes; AGPL |
| [Wave Terminal](https://github.com/wavetermdev/waveterm) | Widest tile range: terminal, browser, previews incl. video/PDF, directories; `wsh` CLI | Electron; mouse-first layout; no herdr integration |
| kitty | Python remote-control API, layouts, macOS and Linux | Terminal-only tiles |
| WezTerm | Lua config, built-in multiplexer, cross-platform | Terminal-only tiles |
| [herdr](https://herdr.dev/docs/) | Persistent agent-aware sessions, SSH remote, phone clients, socket API, macOS and Linux | Lives inside another terminal; terminal panes only |

The gap is a native shell that owns the layout and the non-terminal tiles, and delegates terminal processes to herdr.

## Goals and non-goals

**Goals**

1. One window holds terminals, browsers, file previews and a file browser as tiles in the same layout.
2. Every action is reachable from the keyboard, with layout changes that animate smoothly.
3. Terminal sessions persist through herdr and can be reached from mobile or another machine.
4. Agents can create, arrange, read and drive tiles through a CLI and a Python API.
5. It feels native on each platform: fast startup, low memory, platform conventions.
6. macOS and Linux share one core, so behaviour, config and API are identical on both.

**Non-goals**

- Embedding Zed or building a code editor. Editing happens in terminal editors or external apps.
- tmux sync or tmux control mode. tmux can still run inside a terminal tile.
- Windows builds.
- Electron, Tauri or any approach that renders the terminal inside a webview.
- Being a system-wide window manager. Tiling applies only inside the app's windows.
- Built-in AI chat. Agents run in terminal tiles.
- Agent-driven browser automation (accessibility snapshot, click, fill, evaluate, screenshot). Agents use their own browser tooling; dropped Nov 3, 2026 (was F21).

## Users and use cases

The primary user is a developer on macOS or Linux who runs several coding agents in parallel and lives on the keyboard. The secondary user is the agent itself, driving the workspace through the API.

- **Agent supervision.** As a developer, I run four agents in one workspace and see at a glance which one is blocked, so I can unblock it without hunting through tabs.
- **Live preview.** As a developer, I put a browser tile on my dev server next to the agent's terminal, so I see changes as the agent makes them.
- **Plan review.** As an agent, I write `plan.md` and open it in a preview tile, so the human can follow the plan as I update it.
- **Pick up anywhere.** As a developer, I close my laptop, check an agent from my phone, and find the same session when I reopen the app.
- **Scripted layouts.** As a developer, I run one command to open a project layout: editor terminal, test watcher, dev server and browser.
- **File inspection.** As a developer, I browse a project's files in a tile and preview images, video and PDFs without leaving the workspace.
- **Same tool on both machines.** As a developer with a Mac laptop and a Linux workstation, I use the same app, config and layout files on both.

## Functional requirements

P0 is the MVP, P1 is needed before it can be a daily driver, P2 can come later. Requirements apply to both platforms. The macOS build ships first; Linux reaches parity per priority later (see Milestones).

| ID | Area | Requirement | Priority |
| --- | --- | --- | --- |
| F1 | Layout | Hierarchy of windows, workspaces, tabs and tiles; each tab is a tree of horizontal and vertical splits | P0 |
| F2 | Layout | Keyboard actions to split, focus by direction, swap, move, resize, zoom and close tiles | P0 |
| F3a | Layout | Master-stack ("tall") actions on the split tree: add a tile to the end of an evenly divided stack, promote a tile to master, rotate master left ↔ top, equalize the stack ([keybindings](keybindings.md#decisions)). Extended by Bismuth-style dynamic layouts per tab: Tile (masters and ratio), Monocle and Three Column over an ordered tile list, with the split tree kept as the `manual` kind; new tabs default to Tile, adopted herdr tabs stay manual ([ADR 0010](adr/0010-dynamic-layouts.md), PLAN 2.12, decided Nov 3, 2026) | P0 |
| F3b | Layout | Niri-style scrolling columns layout mode | P2 |
| F4 | Layout | Animated open, close, move and resize; terminals scale a snapshot during motion and resize the PTY once, at the start of the motion, so the redraw finishes under the snapshot ([ADR 0004](adr/0004-animation-resize-timing.md)); can be turned off | P1 |
| F5 | Layout | Command palette for every action | P1 |
| F6 | Layout | Declarative layout files per project, loaded from the CLI or palette | P1 |
| F7 | Tiles | Terminal tile built on Ghostty, reading the user's Ghostty config (full libghostty on macOS; libghostty-vt with our own renderer on Linux, VTE as a stopgap) | P0 |
| F8 | Tiles | Browser tile on WebKit (WKWebView on macOS, WebKitGTK on Linux) with address bar, history, Web Inspector and localhost access | P0 |
| F9 | Tiles | Preview tile: live-reloading markdown, code with syntax highlighting, and images, video and PDF | P0 |
| F10 | Tiles | File browser tile: tree view, keyboard navigation, file watching, open in preview or terminal | P1 |
| F11 | Tiles | Attention signal on a tile when herdr reports a blocked or finished agent. A finished agent's mark clears once the user reads it (its tile focused and on screen in the key window for 0.75 s); a blocked agent's stays until its state changes (Nov 5, 2026). herdr doesn't pass on OSC 9/777/99 notifications, so generic "command finished" alerts wait on upstream ([ADR 0005](adr/0005-scrollback-and-passthrough.md)) | P1 |
| F12 | herdr | Every terminal tile attaches to a herdr pane; a new terminal tile creates a herdr pane | P0 |
| F13 | herdr | Each app workspace maps one-to-one to a herdr workspace | P0 |
| F14 | herdr | Closing a tile detaches by default; killing the herdr pane is an explicit choice | P0 |
| F15 | herdr | Panes created or closed elsewhere (phone, CLI, agent) appear or disappear in the app within 1 second | P1 |
| F16 | herdr | herdr agent state (blocked, working, done, idle) shown on tiles and in the sidebar | P1 |
| F17 | herdr | Several herdr servers at once, local and remote over SSH, grouped by machine in the sidebar with per-machine connection state; the machines are the ones saved in herdr (`herdr machine add`), Azulejo only presents them ([ADR 0011](adr/0011-multiple-herdr-machines.md); moved from P2 on Nov 3, 2026 after the design mockup; PLAN 2.13) | P1 |
| F18 | API | Socket API to list, create, close, focus, move and resize windows, workspaces, tabs and tiles, and to point browser and preview tiles at a URL or path | P0 |
| F19 | API | CLI covering the whole socket API, with JSON output; environment variables identify the calling tile and workspace | P0 |
| F20 | API | Terminal operations: send text and keys, read the screen and scrollback (through herdr's `pane.read`) | P0 |
| F21 | API | ~~Browser operations: accessibility snapshot, click, fill, evaluate JS, screenshot~~ Dropped Nov 3, 2026: agents use their own browser tooling. Opening a URL stays in F18 | — |
| F22 | API | Event stream: tile created or closed, focus changed, agent state changed | P1 |
| F23 | API | Python client mirroring the socket API, sync and async | P1 |
| F24 | Session | Restore layout, browser URLs and preview paths on relaunch; terminal tiles reattach to their herdr panes | P0 |
| F25a | Config | Every keybinding overridable from the TOML config, loaded at startup; per-platform defaults (`cmd` on macOS, Super on Linux) in [`keymap.default.toml`](../shared/config/keymap.default.toml) | P0 |
| F25b | Config | TOML config for keybindings, theme and animation, reloaded on save; the same file works on both platforms | P1 |
| F26 | Config | Sidebar per workspace showing git branch, working directory and agent states (moved from P2 on Nov 3, 2026 after the design mockup; PLAN 2.14) | P1 |
| F27 | Platform | Linux build (GTK4, Wayland and X11) with P0 parity, then P1 parity | P1 |
| F28 | Tiles | Header strip on every tile: kind icon, title and subtitle (terminal: pane title or running command and cwd; browser: page title and host; preview: file name and folder; files: root) (PLAN 2.15) | P1 |
| F29 | Tiles | Preview \| Source toggle on preview tiles; Source shows the file as highlighted code, saved per tile (PLAN 2.15) | P1 |

## Non-functional requirements

Azulejo must feel as fast as Ghostty for terminal work and never drop frames while animating. The targets below are starting proposals, to be tuned after the first prototype.

| Area | Target |
| --- | --- |
| Platform | macOS 14+, Apple Silicon first; Linux x86_64 and arm64 with GTK 4.14+ (Ubuntu 24.04, Fedora 40, Arch), Wayland and X11 |
| Stack | Rust core; Swift and AppKit on macOS; Rust and GTK4 on Linux; no Electron, no webview-rendered terminal |
| Cold start | Under 500 ms to a usable window with restored layout |
| Input latency | Keystroke to glyph within 1 frame of standalone Ghostty on the same machine (Ghostty GTK on Linux) |
| Animation | 120 Hz on ProMotion displays; at least 60 Hz on Linux, display refresh rate where the compositor allows; layout transitions 150 to 250 ms |
| Memory | Within 25% of Ghostty.app with the same splits (one window, 4 terminals, excluding browser tiles). Restated Nov 3, 2026: the first target, under 150 MB, sat below libghostty's own floor (~150 MB for one surface); M1 measured 195 MB with 4 terminals |
| herdr reconnect | Terminal tiles reattach within 2 s after app restart or sleep |
| Stability | A crash in a browser or preview tile never takes down terminal tiles |
| API | Every user action reachable through the socket API; JSON responses versioned |
| Security | Socket readable only by the user |

## Technical approach

Azulejo owns the layout and the non-terminal tiles; herdr owns every terminal process. That split keeps one source of truth for layout. A shared Rust core holds everything that isn't drawing; each platform shell only renders tiles and routes input.

```
┌──────────────────────── Azulejo app (one process) ────────────────────────┐
│  Native shell: AppKit (macOS)  ·  GTK4 (Linux)                             │
│    terminal · browser · preview · files tiles, key router, animation       │
│                │  JSON commands and events over a C ABI                    │
│  Rust core: object model · layout tree · config · persistence ·           │
│             API server · herdr adapter · file watching · markdown/code HTML│
└───────┬───────────────────────────────┬─────────────────────┬─────────────┘
        │ Unix socket, JSON-RPC         │ herdr socket API    │ child process per
        │                               │ (NDJSON)            │ terminal tile
   azj CLI · Python client · agents     ▼                     ▼
                                   herdr server ◄── `herdr terminal attach <id>`
                                        ▲
                               phone · SSH · herdr CLI
```

Agents reach Azulejo only through the socket API. Terminal tiles talk to herdr through `herdr terminal attach`, and the core talks to herdr's socket API, so the phone and the app see the same sessions.

- **Object model.** Window → workspace → tab → layout tree → tile. Each tile has a stable ID and a kind: `terminal`, `browser`, `preview` or `files`. A terminal tile also stores the herdr pane and terminal IDs it views.
- **Core and shells.** The core is a Rust library. The macOS shell links it as a static library through a small C ABI; the Linux shell is Rust and links it directly. Shells send the same JSON commands the public API uses, so every user action is reachable through the API by construction.
- **Terminal tiles.** The terminal surface runs `herdr terminal attach <terminal-id>` as its child process ([ADR 0001](adr/0001-terminal-bridge.md)). herdr streams a full repaint and then re-rendered ANSI frames; keys, mouse and paste go straight back; the PTY size becomes the pane size. The herdr hop adds about 1.3 ms median keystroke → glyph. On macOS the surface is full libghostty. On Linux it starts on VTE and moves to libghostty-vt with our own GPU renderer, because libghostty's embedding API (renderer and surface) only supports macOS and iOS. herdr's NDJSON controller mode (`herdr terminal session control`) is the upgrade path: no detach key and explicit close records. One adapter module owns all herdr wire names.
- **What herdr owns for terminals.** Because the stream is herdr's redraw, not the program's bytes, the local terminal never builds scrollback. Scrollback, find, title, cwd and agent state come from herdr's API and events ([ADR 0005](adr/0005-scrollback-and-passthrough.md)). herdr workspaces map 1:1 to app workspaces, each app tab is one herdr tab with a pane per terminal tile, and herdr's split tree mirrors the tab's both ways ([ADR 0012](adr/0012-mirror-layout-into-herdr.md), which replaced ADR 0002's tab per tile). Tile state (attached, taken over, owned elsewhere, ended) follows the attach client's exit ([ADR 0003](adr/0003-take-over.md)). Azulejo sizes the PTY before attaching, and starts herdr with a clean environment when the server isn't running yet ([ADR 0006](adr/0006-herdr-process-hygiene.md)).
- **The user's own herdr session.** By default Azulejo connects to herdr's default session, the one the user's herdr TUI and phone already use, and adopts what runs there: one workspace per herdr workspace, one tab per herdr tab with the same splits, one terminal tile per pane. Adoption only reads herdr, and a tile closed here stays closed on relaunch. A named session is a config option ([ADR 0008](adr/0008-use-the-users-herdr-session.md)).
- **Browser tiles.** WKWebView on macOS, WebKitGTK 6 on Linux. Agents can point a browser tile at a URL through the API (`browser.open`), but Azulejo doesn't script pages for them.
- **Preview and file tiles.** The core renders markdown and highlighted code to HTML and watches files; shells show it in a webview. Images, video and PDF use QuickLook on macOS and GTK4 media, glycin and poppler on Linux. File browser: NSOutlineView on macOS, GtkListView with a tree model on Linux, both fed by the core's file tree.
- **Animations.** Core Animation on macOS, GtkSnapshot transforms driven by frame-clock tick callbacks on Linux. During motion a terminal shows a scaled snapshot (an IOSurface copy on macOS), and its PTY is resized once when the motion starts. The spike held 120 Hz with no dropped frames.
- **API.** JSON-RPC over a Unix socket in the user's runtime directory. Types are defined once in Rust and exported as JSON Schema; the CLI shares the Rust types and the Python client is generated from the schema.
- **Persistence.** Layout and non-terminal tile state saved to disk on every change; terminal state lives in herdr.

**Build, don't fork.** Azulejo is built from scratch, with cmux and Ghostty's own macOS and GTK apps as reference. A fork would save roughly 3 to 5 weeks on macOS, but cmux's app owns terminal processes, which conflicts with herdr owning them. Most cmux features are out of scope, its layout was not designed around animation, it is macOS-only, and AGPL would constrain licensing.

## Milestones and effort

The macOS MVP takes about 4 to 7 weeks including spikes, a macOS daily driver about 3 to 5 months, and Linux parity another 3 to 4 months, for one experienced developer working with coding agents. `PLAN.md` has the step-by-step version.

| Milestone | Delivers | Effort | Gate |
| --- | --- | --- | --- |
| M0 Spikes | herdr attach in libghostty, input latency, animated resize, core FFI, Linux terminal path | 1 to 2 weeks | G0: every spike meets its pass criterion |
| M1 macOS MVP | All P0 on macOS | 3 to 5 weeks | G1: one week of dogfooding, NFRs measured |
| M2 macOS daily driver | All P1 on macOS | 6 to 10 weeks | G2: success-metric clock starts |
| M3 Linux MVP | All P0 on Linux (VTE terminal) | 4 to 6 weeks | G3: one week of dogfooding on Linux |
| M4 Linux daily driver | All P1 on Linux, libghostty-vt renderer replaces VTE | 6 to 10 weeks | G4: Linux latency parity with Ghostty GTK |
| M5 Later | P2 on both platforms | Ongoing | |

The core builds and tests on Linux CI from day one, and M0 includes a Linux terminal spike, so the core can't drift into macOS-only assumptions while the Mac ships first.

## Risks and mitigations

The biggest risk is depending on two young projects, herdr and libghostty, whose interfaces may still change. Linux adds a second risk: no libghostty renderer there yet.

| Risk | Impact | Mitigation |
| --- | --- | --- |
| herdr socket API changes | Terminal tiles break on herdr upgrades | One adapter module; pin a minimum herdr protocol version; contribute upstream |
| A herdr server restart restores layout but not processes | Workspaces, tabs and pane IDs come back, but shells restart fresh and running agents die; terminal IDs and `--env` vars change | Azulejo never stops or restarts the herdr server itself; recommend `herdr update --handoff` for upgrades; the adapter re-resolves panes after a restart and tiles whose process was respawned show it clearly instead of passing as the old session |
| herdr attach streams re-rendered frames, not raw PTY bytes | No local scrollback; OSC 52, OSC 9/777/99, BEL, kitty graphics, DECCKM and focus reporting are dropped | Scrollback, search, title and attention through herdr's API (ADR 0005); upstream asks filed |
| OSC 52 clipboard lost | Copy from vim or agents to the system clipboard doesn't work in terminal tiles | Upstream ask; local selection copy and `pane.selection.read` meanwhile |
| Detach key `ctrl+b q` can't be turned off | An accidental keypress drops the tile; tmux users lose `prefix q` | Reattach silently on an unrequested detach; upstream `--no-detach-key`; controller mode has no detach key |
| Take-over is signalled only by the attach client's exit code and stderr | Wrong tile state if herdr changes its messages | Patterns live in the adapter with tests pinned to the supported herdr version; upstream ask for clearer exit reasons |
| libghostty embedding API is unstable | Rendering breaks on Ghostty updates | Vendor a pinned Ghostty; bump it in dedicated changes; track upstream releases |
| No libghostty renderer on Linux | Linux terminal quality and latency lag macOS; our renderer is 6 to 9 weeks to beat VTE, 10 to 12 to match Ghostty GTK | Ship on VTE first; build our renderer on libghostty-vt in M4, deferring kitty graphics and sprite polish to M5; consider shimming Ghostty's own MIT font and renderer code (saves 3 to 4 weeks); watch upstream for Linux embedding |
| Two shells drift apart | Features land on one platform only | Logic lives in the core; every feature lands with core tests and API first; shared JS and HTML for webview tiles |
| GTK4 animation performance | Janky motion on Linux | Snapshot-based motion, frame-clock driven; animations can be turned off; measure on GNOME and wlroots compositors |
| Sharing the user's herdr server (ADR 0008) | Panes inherit the environment of whatever started that server (e.g. `CLAUDE_*` from an agent's shell); herdr's `--env KEY=` blanks a variable but can't unset it. A tile open in Azulejo sets its pane's size, so the user's herdr TUI shows that pane at Azulejo's size | Start the server from Azulejo when it isn't running (clean env); document the leak; adoption and restore only read herdr; closing the tile gives the size back. What the TUI draws meanwhile is on the M0 deferred list |
| Two clients on one herdr pane (app and phone) | Size fights, garbled full-screen apps | One writable owner; explicit Take over / Take back, read-only observe otherwise (ADR 0003). A phone on the TUI sees Mac-sized panes while the Mac is attached |
| Focus and key routing between terminals and webviews | Lost shortcuts, IME bugs, stuck focus | Central key router per shell, keymap in the core; a test suite of key and focus scenarios |
| Terminal flicker during animation | Animations feel worse than none | Snapshot during motion, single PTY resize at the start (ADR 0004); animations can be turned off |
| Linux packaging | Flatpak sandbox blocks spawning herdr and the socket | Ship tarball, .deb and AUR first; Flatpak later with host access |

## Success metrics

Success means Azulejo replaces the current terminal-plus-browser setup for daily agent work. Targets apply 4 weeks after the daily-driver milestone on each platform.

- **Daily use:** Azulejo is the only terminal app opened on 5 of 5 workdays.
- **Persistence:** zero lost agent sessions from app restarts, sleep or crashes.
- **Agent API use:** agents create or drive tiles through the API in most sessions, measured from local API logs.
- **Speed:** cold start and input latency meet the non-functional targets on an M-series laptop and a mid-range Linux machine.
- **Mobile:** an agent can be checked and unblocked from the phone without opening the laptop.
- **Parity:** the same config and layout files work unchanged on macOS and Linux.

## Open questions

- [x] Fork cmux or build from scratch? Decided: build from scratch, cmux as reference only.
- [x] Cross-platform approach? Decided: shared Rust core, native shells (AppKit, GTK4); no Electron or Tauri.
- [x] When the app and the phone attach to the same herdr pane, does the newer client take over or mirror? Decided: one writable owner; the app asks before taking over, falls back to read-only, and never retakes automatically ([ADR 0003](adr/0003-take-over.md)).
- [x] One herdr tab per terminal tile, or mirror the app's splits into herdr's layout? First decided one tab per tile, since `layout.apply` kills live panes ([ADR 0002](adr/0002-herdr-mapping.md)); then, on the maintainer's feedback, mirror the splits pane by pane without `layout.apply` ([ADR 0012](adr/0012-mirror-layout-into-herdr.md)).
- [ ] Does the CLI live inside the app bundle or ship as a separate Homebrew formula and Linux package?
- [ ] Open source from day one, and under which license?
- [x] Pull layout modes (F3) into the MVP? Decided: the master-stack (tall) actions are P0 and ship in M1 (F3a); niri-style scrolling columns stay P2 (F3b).
- [x] Bismuth-style dynamic layouts per tab? Decided: yes, Tile, Monocle and Three Column only; new tabs default to dynamic Tile; adopted herdr tabs stay manual ([ADR 0010](adr/0010-dynamic-layouts.md), PLAN 2.12).
- [x] "Session" (design mockup) or "workspace"? Decided: keep "workspace" everywhere, UI included; the designer relabels.
- [ ] Linux packaging: which formats at M3 (tarball, .deb, AUR, Flatpak)?
