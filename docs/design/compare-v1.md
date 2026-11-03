# Mockup comparison v1 (PLAN D.9)

Nov 3, 2026. The mockup's scene built with fixture data by `apps/macos/automation/mockup.sh` and captured at 1440×900 pt with the system window capture (dark appearance).

| Mockup (`mockup-v1.png`) | Azulejo (`capture-v1.png`) |
| --- | --- |
| `mockup-v1.png` (local capture) | `capture-v1.png` (local capture) |

**How the scene is built:** seven local workspaces are small fixture git repositories on the mockup's branches. "build-server" and "dev-server" are throwaway local herdr sessions reached through the ssh stand-in; dev-server then loses its link. The browser tile shows a static fixture page served from a temp folder, the preview tile a fixture ADR, and the "zsh" tile a stand-in `just dogfood` that only prints. Agent states come from `herdr pane report-agent`. No real machine or herdr session is touched. To re-run: `apps/macos/automation/mockup.sh`.

## What matches

- Window layout: sidebar card, toolbar row, tab-strip pill with "+", search field with the ⌘K badge, 8 pt gutters, rounded tile cards.
- Sidebar: Agents | Files control, machine groups with connection dots and amber "Reconnecting…", workspace rows with status icon, name and right-aligned branch (long branches truncate), "+" per group, footer counts.
- Status icons, including the working ring (a partial ring that turns while motion is on, static under Reduce Motion).
- Tile headers: kind icon, bold title, muted "— subtitle", centred for terminals; the preview tile's Preview | Source control.
- Browser chrome: chevrons, rounded address field with the globe icon, reload on the right.
- One Dark surfaces and text colours.

## Remaining differences

Tags: **designer question**, **quick fix**, **later**.

1. **Focused tile outline.** Azulejo draws a 1 pt blue outline on the focused tile; the mockup shows none. *Designer question* (D.1 default, still pending).
2. **Agent badge in the tile corner.** "claude · working" sits at the terminal tile's top right (2.6). The mockup has no badge. *Designer question*: keep, show only for blocked or finished, or rely on the header?
3. **Workspace status when tabs disagree.** The mockup shows azulejo as working while its "review" tab is blocked. Azulejo ranks blocked first, so the row shows blocked. *Designer question*: which state wins in the sidebar row?
4. **Terminal text size.** The terminals use Ghostty's default font size because the user's Ghostty config is empty, which is larger than the mockup's (about 11 pt), so long lines wrap. *Designer question*: should Azulejo default to a smaller terminal size when the Ghostty config sets none? (*quick fix* once decided)
5. **Preview typography.** The markdown preview's heading and body are much larger than the mockup's (heading about 2×), and the code block is styled differently. *Quick fix* in `shared/webview/preview.css`.
6. **Traffic lights missing in this capture.** They are placed correctly (`chrome.lights 29,31`) and show in other captures, but not in this scene. Renaming a workspace hid them; that is fixed: the lights are re-placed after every refresh. Something else in this scene still hides them. *Quick fix*, cause not found yet.
7. **Browser page blank.** WebKit stops drawing in a window that isn't on screen in front, so the captured page is empty; on screen it draws. *Later* (capture limitation; the preview tile already works round it for automation).
8. **"Done" states.** herdr can't report "done" from the CLI, so sample-app, config and the "release" tab show idle, and the footer reads 1 / 3 / 5 / 0 instead of the mockup's 2 / 2 / 3 / 2. *Later* (fixture limitation; the done icon itself renders).
9. **Branches on remote workspaces.** infra and notes show no branch; remote git status isn't implemented (2.14 deferred). *Later*.
10. **Search placeholder.** "Jump to workspace or tile" where the mockup says "session". Decided: the app keeps "workspace". *Designer question* only for the mockup's wording.
11. **Terminal subtitle truncation.** "zsh — just dogfood…" ends with an ellipsis although there's room. Probably the stand-in's process title; check with a real command. *Quick fix* if it reproduces.
