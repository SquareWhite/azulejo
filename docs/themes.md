# Themes

Azulejo's chrome, its terminals and its previews follow one theme. Four ship built in; you can add your own. Switching is live: save the config (or a theme file), pick one in ⌘K or the View menu, and the running app changes in one frame, with no restart and no terminal reattaching or resizing.

## Built-in themes

| Theme | Id | Light or dark | Terminal palette | Preview syntax |
| --- | --- | --- | --- | --- |
| Atom One Dark (default) | `atom-one-dark` | dark | Atom One Dark (background `#282c34`, the tile body) | One Dark |
| Atom One Light | `atom-one-light` | light | Atom One Light (the one-light-syntax colours) | One Light |
| macOS Dark | `macos-dark` | dark | Apple's system colours (Terminal's look, `#1e1e1e`) | GitHub dark |
| macOS Light | `macos-light` | light | Apple's system colours, light | GitHub light |

- **Atom One Dark** is the mockup's look (docs/design/mockup-v1.png) and what Azulejo looked like before themes; nothing in it changed.
- **Atom One Light** uses the official Atom One Light palette (one-light-ui and one-light-syntax): `#fafafa` bodies, `#eaeaeb` cards and strips, `#383a42` text, `#526fff` accent.
- **macOS Dark** and **macOS Light** use the system's own colours (`NSColor.labelColor`, `secondaryLabelColor`, `separatorColor`, `textBackgroundColor`, `windowBackgroundColor`, …), resolved live, so they follow your accent colour (`controlAccentColor` for the accent and selected rows). The sidebar card is a vibrancy view (`NSVisualEffectView`, `.sidebar` material, behind the window), as in Finder. Changing the accent colour in System Settings re-colours the app at once. Where macOS has no system colour that fits, the theme uses a fixed one: macOS Light's window (`#ececec`, the classic light window grey; macOS 26's `windowBackgroundColor` is white, which would leave the cards no edge) and its sidebar, header and browser strips (`#f5f5f5`), the focus dim (black in macOS Dark, the window grey in macOS Light), and the HUD, badge and overlay fills. The terminal palettes are fixed too (Ghostty takes hex colours).

Screenshots: `design/theme-atom-one-dark.png` (local capture), `design/theme-atom-one-light.png` (local capture), `design/theme-macos-dark.png` (local capture), `design/theme-macos-light.png` (local capture).

## Picking a theme

```toml
# ~/.config/azulejo/config.toml
[appearance]
theme = "atom-one-light"     # a theme id, or "auto"
```

- **⌘K → "Set theme…"** lists Auto and every theme. Moving through the list previews each one live; Enter keeps it (Azulejo writes `theme = "…"` into `[appearance]`, changing nothing else in the file), Esc goes back to the one you had. The action is `set_theme`; it has no default chord, bind one under `[keys]` if you like.
- **View → Theme** lists the same, with a tick on the current one.
- **Auto** follows macOS's light or dark appearance as it changes:

  ```toml
  [appearance]
  theme = "auto"
  theme_light = "atom-one-light"   # the default
  theme_dark = "atom-one-dark"     # the default
  ```

The theme sets the app's appearance (Aqua or Dark Aqua), so menus, scrollers, text fields and system controls match it whatever the Mac's own setting. An unknown theme name is an error in the config banner: at start the default applies, on a reload the previous config stays.

## Terminal colours

Each theme has a terminal palette: background, foreground, cursor, cursor text, selection, and the 16 ANSI colours. Azulejo loads it on top of your Ghostty config (libghostty's config API, pushed to every terminal live), so fonts and everything else stay Ghostty's.

Your own Ghostty colours win by default:

```toml
[appearance]
terminal_colors = "theme"     # always the theme's palette
terminal_colors = "ghostty"   # always your Ghostty config's colours
```

| `terminal_colors` | Your Ghostty config sets `theme`, `background`, `foreground` or `palette` | It sets none of them |
| --- | --- | --- |
| unset (default) | Ghostty's colours | the theme's palette |
| `"theme"` | the theme's palette | the theme's palette |
| `"ghostty"` | Ghostty's colours | Ghostty's defaults |

With Ghostty's colours, a Ghostty `theme = light:…,dark:…` follows Azulejo's theme (Azulejo tells libghostty whether it's light or dark). The margin round a terminal's text always takes the terminal's own background.

**Unfocused tiles** dim towards the theme's `dimFill` at `alpha.unfocusedDim` (25 %), unless your Ghostty config sets `unfocused-split-opacity` or `unfocused-split-fill` (those win, as before). Each theme picks a fill that suits it: the window colour for the Atom themes and macOS Light, black for macOS Dark (its window colour is lighter than its tiles).

## Previews

Rendered markdown, Source mode and code previews follow the theme: Azulejo sets the page's CSS variables (`--azj-bg`, `--azj-fg`, …, mapped from colour tokens in tokens.json `webview`) and its syntax colours (`html[data-azj-syntax]`: One Dark, One Light, GitHub dark or GitHub light, built in the core from scope tables) before the page draws, and again on a theme change. Browser tiles show web pages as the site draws them; only their chrome (toolbar, address field, notices) follows the theme.

## Your own themes

A theme is a TOML file in a `themes` folder next to the config: `~/.config/azulejo/themes/<id>.toml`. The file name is the id (letters, digits, `-`, `_`, `.`; it can't reuse a built-in's id). Saving it applies it live, like the config.

```toml
# ~/.config/azulejo/themes/dusk.toml
name = "Dusk"                # shown in ⌘K and the menu; the id if missing
inherits = "atom-one-dark"   # a built-in to start from
appearance = "dark"          # light or dark; defaults to the inherited theme's
syntax = "one-dark"          # one-dark, one-light, github-dark or github-light

[color]                      # any colour token, camelCase or snake_case
window = "#14161b"
text_muted = "#8a8f99"
accent = "#ff8800"

[terminal]
background = "#1d2026"
foreground = "#c8ccd4"
cursor = "#ff8800"
cursor_text = "#1d2026"
selection_background = "#3a3f4b"
selection_foreground = ""    # "": selected text keeps its colours
palette = { 1 = "#ff6c6b", 9 = "#ff8080" }   # or a list of 16 colours
```

```toml
[appearance]
theme = "dusk"
```

- **Without `inherits`**, a theme starts from Atom One Dark, or Atom One Light with `appearance = "light"`.
- **Colours** are `#rgb`, `#rrggbb` or `#rrggbbaa`. Tokens you don't set keep the inherited theme's values, system colours included (so a theme inheriting `macos-dark` still follows your accent where you didn't override it).
- **Problems never stop the app.** An unknown key or token, a bad colour, a bad `syntax` or `appearance`: each is a warning in the config banner, pointing at the line, and that value keeps the inherited one. A file that isn't valid TOML is reported and the theme doesn't exist until it's fixed (a config naming it then keeps its previous theme on reload).

## Colour tokens

Every theme sets every token below. In the macOS columns, a name is an `NSColor` (`@ 0.45`: with that alpha); its default value is in tokens.json (`hex`) for other platforms, previews and the contrast tests. Tokens that aren't colours (radii, spacing, type, opacities like `alpha.unfocusedDim`, SF Symbol sizes, motion) are shared by all themes.

| Token | For | atom-one-dark | atom-one-light | macos-dark | macos-light |
| --- | --- | --- | --- | --- | --- |
| `window` | window background, gutters between tiles, toolbar row | `#1b1d23` | `#e1e1e3` | underPageBackgroundColor | `#ececec` |
| `sidebar` | sidebar card, tab bar, tile header strip | `#21252b` | `#eaeaeb` | windowBackgroundColor | `#f5f5f5` |
| `tile` | tile body: terminal backdrop, browser, preview, files | `#282c34` | `#fafafa` | textBackgroundColor | textBackgroundColor |
| `tileAlt` | secondary tile bodies | `#252930` | `#f5f5f6` | textBackgroundColor | textBackgroundColor |
| `sunken` | browser chrome, code blocks, inset strips | `#202329` | `#ededee` | underPageBackgroundColor | `#f5f5f5` |
| `control` | segmented tracks, search field, tab container | `#2c3036` | `#dfdfe1` | secondarySystemFill | secondarySystemFill |
| `selected` | selected sidebar row, selected palette row | `#35393e` | `#dbdbdc` | controlAccentColor @ 0.35 | controlAccentColor @ 0.2 |
| `selectedStrong` | strong selection grey: preview borders and table rules | `#3a3f4b` | `#d0d0d2` | unemphasizedSelectedContentBackgroundColor | unemphasizedSelectedContentBackgroundColor |
| `border` | hairlines around cards, fields and toasts | `#ffffff14` | `#0000001a` | separatorColor | separatorColor |
| `textPrimary` | names, titles, tile header titles | `#d7dae0` | `#383a42` | labelColor | labelColor |
| `textSecondary` | tab labels, details, tile header icons | `#9da5b4` | `#5c5f69` | secondaryLabelColor | labelColor @ 0.62 |
| `textMuted` | group labels, branches, subtitles, hidden files | `#7f848e` | `#74767f` | labelColor @ 0.45 | labelColor @ 0.45 |
| `textOnAccent` | text on HUDs, badges and coloured fills | `#ffffff` | `#ffffff` | alternateSelectedControlTextColor | alternateSelectedControlTextColor |
| `accent` | progress bars, matched characters, links | `#61afef` | `#526fff` | controlAccentColor | controlAccentColor |
| `dimFill` | what unfocused tiles fade towards (alpha.unfocusedDim): the window colour, so they recede into the gutters. Ghostty's unfocused-split-fill overrides it | `#1b1d23` | `#e1e1e3` | `#000000` | `#ececec` |
| `statusWorking` | working agents (azulejo.ring) | `#61afef` | `#4078f2` | systemBlue | systemBlue |
| `statusBlocked` | agents waiting for input | `#e5c07b` | `#c18401` | systemYellow | systemOrange |
| `statusDone` | agents that finished | `#98c379` | `#50a14f` | systemGreen | systemGreen |
| `statusIdle` | idle agents | `#7f848e` | `#74767f` | labelColor @ 0.45 | labelColor @ 0.45 |
| `statusOff` | machines that are off | `#5c6370` | `#a0a1a7` | tertiaryLabelColor | tertiaryLabelColor |
| `statusConnected` | connected machines | `#98c379` | `#50a14f` | systemGreen | systemGreen |
| `statusConnecting` | machines connecting | `#e5c07b` | `#c18401` | systemYellow | systemOrange |
| `statusReconnecting` | machines reconnecting | `#d19a66` | `#986801` | systemOrange | systemOrange |
| `statusError` | machines or agents in error | `#e06c75` | `#e45649` | systemRed | systemRed |
| `error` | config errors, failed loads | `#e06c75` | `#e45649` | systemRed | systemRed |
| `warning` | config warnings | `#e5c07b` | `#c18401` | systemYellow | systemOrange |
| `gitModified` | git: modified | `#d19a66` | `#986801` | systemOrange | systemOrange |
| `gitAdded` | git: added | `#98c379` | `#50a14f` | systemGreen | systemGreen |
| `gitDeleted` | git: deleted | `#e06c75` | `#e45649` | systemRed | systemRed |
| `gitRenamed` | git: renamed | `#61afef` | `#4078f2` | systemBlue | systemBlue |
| `gitConflicted` | git: conflicted | `#c678dd` | `#a626a4` | systemPurple | systemPurple |
| `hud` | status HUD in the window corner | `#000000bf` | `#000000bf` | `#000000bf` | `#000000bf` |
| `badge` | agent badge pill on tiles | `#0000008c` | `#0000008c` | `#0000008c` | `#0000008c` |
| `overlay` | terminal state overlays (reconnecting, ended) | `#1b1d23eb` | `#fafafaeb` | `#1e1e1eeb` | `#ffffffeb` |
| `overlayText` | text on terminal state overlays | `#bfbfbf` | `#5c5f69` | secondaryLabelColor | labelColor @ 0.62 |
| `overlayTitle` | titles on terminal state overlays (Session ended, reconnecting) | `#ffffff` | `#383a42` | labelColor | labelColor |
| `takeover` | read-only / taken-over banner on terminals | `#8c610df2` | `#8c610df2` | `#8c610df2` | `#8c610df2` |
| `shadow` | drop shadows (⌘K) | `#000000` | `#000000` | shadowColor | shadowColor |
| `ghostFill` | tile outlines during the outline animation style | `#8080801a` | `#8080801a` | `#8080801a` | `#8080801a` |
| `header` | tile header strip (mockup: the focused claude tile; unfocused strips get there through the dim, #202329) | `#21252b` | `#eaeaeb` | windowBackgroundColor | `#f5f5f5` |
| `headerHairline` | line under a tile's header strip | `#181a1f` | `#dbdbdc` | separatorColor | separatorColor |
| `browserChrome` | browser tile toolbar (back, forward, address, reload) | `#202329` | `#ededee` | windowBackgroundColor | `#f5f5f5` |
| `browserField` | browser address pill | `#252930` | `#fafafa` | secondarySystemFill | textBackgroundColor |
| `previewToggleTrack` | Preview | Source track | `#2c3036` | `#dfdfe1` | secondarySystemFill | secondarySystemFill |
| `previewToggleSelected` | Preview | Source selected segment | `#3a3f4b` | `#ffffff` | controlColor | controlColor |
| `kindTerminal` | placeholder tile bodies before a tile kind's view exists | `#29303d` | `#29303d` | `#29303d` | `#29303d` |
| `kindBrowser` | placeholder body, browser | `#21527a` | `#21527a` | `#21527a` | `#21527a` |
| `kindPreview` | placeholder body, preview | `#5c4078` | `#5c4078` | `#5c4078` | `#5c4078` |
| `kindFiles` | placeholder body, files | `#336145` | `#336145` | `#336145` | `#336145` |
| `tabStrip` | the tab strip's pill container in the toolbar | `#24262c` | `#e8e8e9` | tertiarySystemFill | tertiarySystemFill |
| `tabStripBorder` | hairline around the tab strip | `#2f3036` | `#d4d4d6` | separatorColor | separatorColor |
| `tabSelected` | the selected tab's fill | `#3a3f4b` | `#ffffff` | tertiaryLabelColor | controlColor |
| `tabText` | unselected tab names | `#9da5b4` | `#5c5f69` | secondaryLabelColor | labelColor @ 0.62 |
| `tabTextSelected` | the selected tab's name | `#d7dae0` | `#383a42` | labelColor | labelColor |
| `tabAdd` | the tab strip's + | `#6c717a` | `#74767f` | secondaryLabelColor | secondaryLabelColor |
| `searchField` | toolbar search field | `#27292e` | `#fafafa` | secondarySystemFill | textBackgroundColor |
| `searchBorder` | toolbar search field hairline | `#33353a` | `#d4d4d6` | separatorColor | separatorColor |
| `searchBorderActive` | search field hairline while quick open is open | `#61afef80` | `#526fff80` | controlAccentColor @ 0.5 | controlAccentColor @ 0.5 |
| `searchText` | search placeholder and magnifier | `#7f848e` | `#74767f` | labelColor @ 0.45 | labelColor @ 0.45 |
| `searchBadge` | the search field's key badge (⌘K) | `#34363a` | `#eaeaeb` | secondarySystemFill | secondarySystemFill |
| `searchBadgeText` | the search field's key badge text | `#abb2bf` | `#5c5f69` | secondaryLabelColor | labelColor @ 0.62 |
| `titlebarIcon` | sidebar toggle next to the traffic lights | `#7f848e` | `#74767f` | secondaryLabelColor | secondaryLabelColor |
| `noticeFill` | preview notices (a truncated file): fill | `#2e2a1a` | `#fff8c5` | systemYellow @ 0.14 | systemYellow @ 0.14 |
| `noticeBorder` | preview notices: border | `#9e6a03` | `#d4a72c` | systemYellow @ 0.6 | systemOrange @ 0.7 |

## Where it lives

- `shared/design/tokens.json`: `colorTokens` (the list), `themes.<id>` (each theme's `appearance`, `syntax`, `sidebarMaterial`, `color` and `terminal`), `webview` (CSS variables). `scripts/design-tokens` checks that every theme sets every token with a valid value and generates the macOS shell's `Design.swift` (the per-theme tables included); the Linux shell (M3) reads the same file through `azulejo-config`.
- `azulejo-config` (`theme.rs`): built-ins, user themes, `[appearance]`, the terminal colour precedence and the one-line config edit behind "Set theme…". The FFI serves it to the macOS shell (`azj_core_theme`, `azj_core_set_theme`).
- macOS shell (`Theme.swift`): every `Design.Color` token is a dynamic `NSColor` that reads the current theme. Views that keep a colour as a `CGColor` or a baked image adopt `ThemeAware`: they set those in `applyTheme()` and register with `Theme.track(self)`; a theme change calls every `applyTheme()`, sets the app's and windows' appearance and redraws everything in one Core Animation transaction, then swaps the terminals' config and updates the previews. `scripts/check-theme-colours` (in `just lint`) fails on any `.cgColor` outside `applyTheme()`, `updateLayer()` or `draw(_:)` that doesn't say how it stays current.

## Tests

- `cargo test -p azulejo-config --test themes`: the built-ins (all four define every token; atom-one-dark keeps the mockup's values), user theme parsing, inheritance and validation, `[appearance]`, the terminal colour precedence, and WCAG AA contrast for every built-in (body text 4.5:1 and muted or secondary text 3:1 against each background they're drawn on, terminal text 4.5:1; `-- --nocapture` prints the table).
- `python3 scripts/test_design_tokens.py` (in `just lint`): the generator's validation.
- `apps/macos/automation/themes.sh`: the mockup's scene switched through all four themes live (config file and ⌘K, with the preview and Esc), auto, a user theme and the Ghostty precedence, checking that no PTY resized and no terminal reattached, and sampling the sidebar, rail, tab strip, a header, a terminal, the preview, ⌘K and the find bar in each capture. `--docs` writes the captures to `docs/design/theme-<id>.png`.
- `AZJ_ANIM_THEME=atom-one-light apps/macos/automation/anim.sh --slow`: layout motion in another theme (`outside.py` measures stray pixels against the window colour, so light themes count the same way).
