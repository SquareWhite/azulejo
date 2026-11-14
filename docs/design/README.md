# Design: mockup v1

`mockup-v1.png` (local capture)

`mockup-v1.png` is the designer's mockup the maintainer shared on Nov 3, 2026: 1440 × 900 at @1x, 297 KB. the maintainer's direction: "that's how I want the app to look like. Let's get the basic functionality working first, then we'll polish the GUI." The visual work is planned as **M2.5** in [PLAN.md](../../PLAN.md); the functional items it implies were decided on Nov 3, 2026 (end of this file, and PLAN under "Decided from the mockup").

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

## Tokens in code (D.1)

The values above live in [`shared/design/tokens.json`](../../shared/design/tokens.json), the single source for both shells. The macOS shell reads them through `Design.swift`, which `scripts/design-tokens` (or `just design-tokens`) generates; `just lint` fails while it's stale. No view sets a colour, font or corner radius directly any more.

- **Colours** use the palette above under camelCase names (`Design.Color.window`, `.sidebar`, `.tile`, `.textPrimary`, `.textMuted`, `.accent`, `.statusBlocked` …). Each has a dark and a light value; the light ones are provisional.
- **Tile radii and spacing** follow the mockup since D.5: gutter and outer margin 8 pt, tile radius 10 pt, header strip 28 pt, browser toolbar 36 pt, 8 pt margin round a terminal's text. The sidebar width (168 → 230 pt) moves with D.3.
- **Tile cards** (D.5): the card's corners are the window colour drawn over the tile (`CornerCapLayer`, a nine-slice image), not a rounded mask, so the terminal's Metal surface and web views aren't masked (a rounded mask costs an offscreen pass). During crop motion the caps follow the moving clip.
- **Header strips** (D.5): unfocused tiles `header` (#202329), the focused tile `headerFocused` (#21252b), with a `headerHairline` underneath, sampled from the mockup, whose focused tile is a shade lighter. Terminal titles are centred; tiles with controls on the right (preview, files) are left-aligned. Browser tiles have no header strip; their toolbar takes its place (`browserChrome`, address pill `browserField`, D.6).
- **Preview** (D.7): Preview | Source pill (`previewToggleTrack`, `previewToggleSelected`); Open and Reveal show while the pointer is over the header. The page's dark palette (`shared/webview/preview.css`) now uses the mockup's One Dark values (body #252930).
- **Status icons** (`Design.Status`): working `progress.indicator`, blocked `exclamationmark.circle.fill`, done `checkmark.circle`, idle `circle.dashed`, off and connecting `circle.dotted`, reconnecting `arrow.triangle.2.circlepath`, error `exclamationmark.triangle.fill`, connected `circle.fill`, each with its status colour.
- **Blocked is yellow**, as in the mockup, where the shell used red before. Red is for errors only.
- **Opacities** (`Design.Alpha`, Nov 4): every alpha the shell lays over a colour token (attention border, row hover, faded footer counts, dimmed rows, banners, the unfocused-tile dim) is a token too, as are the badge's and machine's dot radii.
- **After the Nov 3 e2e pass** (Nov 4): path subtitles in tile headers drop leading folders ("…/projects/azulejo") so the last one stays; the agent badge sits at the right end of the header strip, never over terminal text; a tile's header controls (Preview | Source, the files buttons) hide whole when they'd leave the title less than `headerTitleMin`, rather than clip; the config banner takes its own row under the toolbar and the tiles move down for it; a new terminal's "starting the herdr pane…" placeholder uses the terminal's background.
- **Tab overflow** (provisional, pending the designer): tabs shrink to `tabFitMinWidth` and no further; past that the strip scrolls sideways, fades (`tabFadeWidth`) the edges that hide tabs, and scrolls the selected tab into view. When not even one tab fits at that width (the minimum window size), the strip shows the selected tab alone, shrunk down to its icon, and the search field gives way before the strip goes below that.
- **After the Nov 4 e2e pass**: SF Symbol point sizes are tokens too (`Design.Symbol`, by where the symbol sits). The find bar drops its controls whole in narrow tiles (the count, then previous / next, then close) so the query keeps `findFieldMin`; the browser's back and forward give way the same way. The agent badge is a subview of the header strip, so it rides with the strip mid-motion. The browser's notice banner takes its own row over the page, and the config banner is capped at a fifth of the tile area, its text cut with the whole message in the tooltip.

### Focus indicator (pending designer confirmation)

The mockup doesn't show focus. Until the designer decides, the focused tile gets:

- a 1 pt outline in the accent blue at 45 % opacity (`Design.Color.focus`, `Design.Border.focus`), drawn just outside the tile and sliding between tiles as before;
- its header title at full brightness (`textPrimary`); other tiles' header titles use `textSecondary`;
- its header strip a shade lighter (`headerFocused` vs `header`, D.5), as on the mockup's focused tile. The mockup's focused tile body is lighter too (#282c34 vs #252930); terminals draw their own body from the Ghostty theme, so that part isn't copied;
- an orange outline instead while the tile is zoomed (`Design.Color.zoom`).

Changing any of these is a one-line edit in `tokens.json`.

## What the mockup needs, against the plan

Status as of Nov 3, 2026, after the maintainer's decisions. "Visual" means M2.5 only; "functional" means something the core or the shell can't do yet.

| Mockup element | Plan / PRD item | Status | Kind |
| --- | --- | --- | --- |
| Hidden title bar, traffic lights in the sidebar card | — | M2.5 | visual |
| Sidebar card with workspace list | 1.6 (basic sidebar) | done, needs restyle | visual |
| Sidebar toggle button | `toggle_sidebar` (1.6) | done, needs restyle | visual |
| **Agents \| Files** sidebar toggle | F10 tile (2.5, `c7121ce`); sidebar panel in M2.5 D.3 | decided: M2.5 | functional |
| Machine groups (Local, build-server, dev-server) | F17 (now P1), PLAN 2.13 | decided: M2, before polish | functional |
| Per-machine connection state, "Reconnecting…" | `herdr.status` (1.12); per machine in 2.13 | decided: M2 (2.13) | functional |
| "+" per group | `workspace.create` (1.11); "on machine X" in 2.13 | decided: M2 (2.13) | functional |
| Workspace row status icon | F16 / 2.6 (core `05a8c53`, shell `5cf85f5`) | done, needs restyle | functional + visual |
| Git branch per workspace | F26 (now P1), PLAN 2.14 | decided: M2, before polish | functional |
| Dimmed rows while a machine reconnects | 2.13, styled in M2.5 | follows 2.13 | visual |
| Footer rollup counts | 2.6 workspace rollups | core done; footer UI new | visual |
| Tab status icons | 2.6 tab indicator (`5cf85f5`) | done, needs restyle | visual |
| "+" new tab | `tab.create` | done, needs restyle | visual |
| **"Jump to session or tile ⌘K"** search field | 2.2 picker (`cc6f717`); ⌘K and the field in M2.5 D.8 | decided: M2.5 | functional (keymap) + visual |
| Tile header: icon, title, "— cwd or command" | F28, PLAN 2.15 | decided: M2, before polish | functional |
| Browser chrome (back, forward, URL, reload) | 1.8 (`8638e21`) | done, needs restyle | visual |
| Preview tile header with file name | 1.9b (`72f2cb3`) | done, needs restyle | visual |
| **Preview \| Source** toggle | F29, PLAN 2.15 | decided: M2, before polish | functional |
| Rounded tile cards with gutters | — | M2.5 | visual |
| One Dark chrome palette, terminal theme to match | M2.5 D.1 (design tokens) | decided: M2.5 | functional (theme) + visual |
| Focus indicator | 1.6 focus ring, 2.1 ring animation, D.1 subtle default | done; the mockup shows none, so **ask the designer** (default described above) | visual |
| "session" wording ("Jump to session or tile") | keep "workspace" | decided: designer relabels | copy |

### Decisions on the functional items (Nov 3, 2026)

Built before GUI polish, in M2:

- **Several herdr servers at once**, grouped by machine with per-machine connection state and "+": PLAN 2.13 (F17 → P1).
- **Git branch per workspace** in the sidebar: PLAN 2.14 (F26 → P1).
- **Tile headers** (title and cwd or running command) and the **Preview | Source** toggle: PLAN 2.15 (F28, F29).

Left for M2.5 polish:

- **Files panel in the sidebar** (Agents | Files): D.3.
- **⌘K** for the picker and an always-visible search field: D.8.
- **Chrome theming** and a matching default terminal theme: D.1.

Naming: keep "workspace" everywhere; the designer relabels "session".
