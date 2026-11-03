# Design: mockup v1

`mockup-v1.png` (local capture)

`mockup-v1.png` is the designer's mockup the maintainer shared on Nov 3, 2026: 1440 × 900 at @1x, 297 KB. the maintainer's direction: "that's how I want the app to look like. Let's get the basic functionality working first, then we'll polish the GUI." The visual work is planned as **M2.5** in [PLAN.md](../../PLAN.md); the functional items it implies are listed at the end of this file and in PLAN under "Proposed from mockup".

Colours below were sampled from the PNG with PIL. Anti-aliased text gives approximate values; flat fills are exact. The palette matches Atom's One Dark closely, so where a sample is fuzzy the One Dark value is given.

## Layout at a glance

Everything sits on a dark window background (`#1b1d23`) as rounded cards with even gutters: a sidebar card on the left, a toolbar row along the top, and the tile area below it. There's no visible title bar; the traffic lights live inside the sidebar card.

| Region | Position (px, @1x) |
| --- | --- |
| Sidebar card | x 10–240, full height, with a 10 px margin |
| Toolbar row (tabs + search) | y 12–48, from x 255 to the right edge |
| Tile area | x 248–1432, y 62–890 |
| Gutter between cards | ~8 px |
| Corner radius | ~10 px on tiles, ~12 px on the sidebar card |

## Window chrome

- **No title bar.** Full-size content view; traffic lights (red, yellow, green) at the top-left of the sidebar card, with a sidebar-toggle icon at the card's top-right.
- The toolbar row shares the title-bar height: tab strip on the left, search field on the right.

## Sidebar (card, `#21252b`)

- **Segmented control** at the top: **Agents** (selected, list icon) | **Files** (folder icon). Track `#2c3036`, selected segment lighter.
- **Machine groups**, each with a small-caps-style label in muted grey (`#7f848e`) and a status on the right:
  - **Local**: green dot (`#98c379`), connected.
  - **build-server**: green dot, connected.
  - **dev-server**: small orange spinner + "Reconnecting…" in orange (One Dark `#d19a66`; sampled `#b4875c` through anti-aliasing). Its row is dimmed.
- **Workspace rows** (the mockup calls them sessions): status icon, name in primary text (`#d7dae0`), git branch right-aligned in muted text, truncated with an ellipsis ("audit/critical-…"). The selected row (`azulejo`) has a rounded highlight (`#35393e`).
  - Local: azulejo · main (working), sample-service · development (blocked), sample-app · main (done), sample-api · main (working), sample-project · audit/critical-… (blocked), sample-dashboard · dev (idle), config · main (done).
  - build-server: infra · main (idle).
  - dev-server: notes · main (idle, dimmed while the machine reconnects).
- **"+"** under each group to add a workspace on that machine.
- **Footer**: rollup counts by status, icon + number: working 2, blocked 2, idle 3, done 2.

### Status icons (used in sidebar, tabs and footer)

| State | Icon | Colour |
| --- | --- | --- |
| working | open spinner arc | blue `#61afef` |
| blocked | filled circle with "!" | yellow `#e5c07b` |
| done | circle with a check | green `#98c379` |
| idle | dashed circle | grey `#393d44`–`#7f848e` |
| reconnecting (machine) | small spinner + label | orange `#d19a66` |

## Toolbar row

- **Tab strip** in a rounded container (`#24262c`): `agents` (selected, `#3a3f4b`, working spinner), `review` (blocked), `release` (done), then **"+"**. Tab labels in secondary text (`#9da5b4`), the selected one in primary.
- **Search field** on the right, "Jump to session or tile" with a magnifier and a **⌘K** key badge (field `#27292e`, badge `#34363a`).

## Tiles

Four tiles in the mockup: a terminal as the master on the left, a browser top-right, and a preview and a terminal side by side bottom-right. Each is a rounded card; no tile shows a coloured focus ring, so the mockup doesn't say how focus is drawn.

- **Tile header** (terminal and preview): a strip (`#21252b`) with a kind icon, the title in bold primary text and a muted subtitle after an em dash, centred: "claude — ~/src/azulejo", "zsh — just dogfood". A hairline below separates it from the content.
- **Terminal tiles**: body `#282c34` (master) or `#252930`. Terminal colours look like One Dark too (yellow `#e5c07b`, green `#98c379`, blue links), so the terminal theme matches the chrome.
- **Browser tile**: no header strip; the browser chrome is the header. Back and forward chevrons, a rounded URL field (`#272b33`) with a globe icon and "localhost:5173/patterns/14", reload at the right. Chrome background `#202329`.
- **Preview tile**: header with a document icon, the file name truncated ("0009-pane-exit.…") and a small segmented **Preview | Source** toggle (Preview selected, `#333842`). Body `#202329`–`#252930`; rendered markdown with a muted eyebrow ("ADR 0009 · Accepted"), a semibold heading, body text, inline code in red/orange, and a code block on a darker fill (`#202329`).

## Typography (impressions)

- UI text: a neo-grotesque sans (SF Pro or Inter), about 13 px for rows, tabs and headers; group labels about 11 px in muted grey.
- Tile header titles: semibold 12–13 px, subtitle regular in muted grey.
- Preview heading: semibold ~19 px; body ~13 px with generous line height.
- Terminals: a monospace at ~12–13 px (SF Mono or JetBrains Mono style).

## Palette summary

| Token | Value | Used for |
| --- | --- | --- |
| `bg.window` | `#1b1d23` | window background, gutters, toolbar row |
| `bg.sidebar` | `#21252b` | sidebar card, tile headers |
| `bg.tile` | `#282c34` | terminal tile body (One Dark background) |
| `bg.tile.alt` | `#252930` | secondary tile bodies |
| `bg.sunken` | `#202329` | browser chrome, code blocks |
| `bg.control` | `#24262c` / `#27292e` / `#2c3036` | tab container, search field, segmented track |
| `bg.selected` | `#35393e` / `#3a3f4b` | selected sidebar row / selected tab |
| `text.primary` | `#d7dae0` | names, titles |
| `text.secondary` | `#9da5b4` | tab labels |
| `text.muted` | `#7f848e` | group labels, branches, subtitles |
| `accent.blue` | `#61afef` | working |
| `accent.yellow` | `#e5c07b` | blocked |
| `accent.green` | `#98c379` | done, connected |
| `accent.orange` | `#d19a66` | reconnecting |
| `accent.red` | `#e06c75` (One Dark, not sampled) | errors, inline code |

## What the mockup needs, against the plan

Status as of Nov 3, 2026. "Visual" means M2.5 only; "functional" means something the core or the shell can't do yet.

| Mockup element | Plan / PRD item | Status | Kind |
| --- | --- | --- | --- |
| Hidden title bar, traffic lights in the sidebar card | — | M2.5 | visual |
| Sidebar card with workspace list | 1.6 (basic sidebar) | done, needs restyle | visual |
| Sidebar toggle button | `toggle_sidebar` (1.6) | done, needs restyle | visual |
| **Agents \| Files** sidebar toggle | F10 is a *tile* (2.5, `c7121ce`); a sidebar Files panel isn't planned | **not yet planned** | functional |
| Machine groups (Local, build-server, dev-server) | F17 remote herdr (P2, M5 step 5.2) | planned M5; the mockup implies **several herdr servers at once** in one window, which isn't planned | functional |
| Per-machine connection state, "Reconnecting…" | `herdr.status` event (1.12) for the one connection | done for one server; per machine **not yet planned** | functional |
| "+" per group | `workspace.create` (1.11) | done for the local server; "on machine X" **not yet planned** | functional |
| Workspace row status icon | F16 / 2.6 (core `05a8c53`) | core done, shell badges in progress | functional + visual |
| Git branch per workspace | F26 (P2, M5 step 5.3) | planned M5 | functional |
| Dimmed rows while a machine reconnects | — | not yet planned (follows multi-machine) | visual |
| Footer rollup counts | 2.6 workspace rollups | core done; footer UI new | visual |
| Tab status icons | 2.6 tab indicator | in progress | visual |
| "+" new tab | `tab.create` | done, needs restyle | visual |
| **"Jump to session or tile ⌘K"** search field | 2.2 picker, bound to `cmd+p` in [keybindings.md](../keybindings.md) | picker in progress; **⌘K binding and an always-visible field are not yet planned** | functional (keymap) + visual |
| Tile header: icon, title, "— cwd or command" | — (preview and files tiles have their own header strips) | **not yet planned**; needs pane title and cwd in the model (herdr reports both in its events) | functional |
| Browser chrome (back, forward, URL, reload) | 1.8 (`8638e21`) | done, needs restyle | visual |
| Preview tile header with file name | 1.9b (`72f2cb3`) | done, needs restyle | visual |
| **Preview \| Source** toggle | — | **not yet planned** (the core can already highlight markdown as code) | functional |
| Rounded tile cards with gutters | — | M2.5 | visual |
| One Dark chrome palette, terminal theme to match | F25 mentions a theme in config | chrome theming **not yet planned**; terminals follow the Ghostty config, which is empty for the maintainer today | functional (theme) + visual |
| Focus indicator | 1.6 focus ring, 2.1 ring animation | done; the mockup shows none, so **ask the designer** | visual |
| "session" wording ("Jump to session or tile") | the code and docs say "workspace" | naming **to decide** | copy |

### Functional items the mockup implies that aren't planned yet

1. A **Files panel in the sidebar** (Agents | Files), next to the files tile.
2. **Several herdr servers at once**, grouped by machine in the sidebar, each with its own connection state and "+". Pulls F17 forward and goes beyond it (F17 is one remote server per workspace).
3. **Git branch per workspace** in the sidebar (F26, planned for M5).
4. **Tile headers** showing the pane's title and cwd or running command, which means carrying herdr's pane title and cwd into the model.
5. **Preview | Source** toggle on preview tiles.
6. **⌘K** for the picker, plus an always-visible search field in the toolbar (today `goto` is `cmd+p`).
7. **Chrome theming**: design tokens for the app, and a default terminal theme that matches (One Dark) when the Ghostty config doesn't set one.
8. **Naming**: "session" in the UI vs "workspace" in the code, docs and API.
