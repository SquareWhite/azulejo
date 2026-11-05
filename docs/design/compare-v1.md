# Mockup comparison v1 (PLAN D.9)

Nov 3, 2026. The mockup's scene built with fixture data by `apps/macos/automation/mockup.sh` and captured at 1440×900 pt (dark appearance). The capture is the app's own render of its layer tree (`01-scene-app.png`), with the title bar's traffic lights drawn in; `screencapture` needs an unlocked screen. Recaptured on Nov 3, 2026 after the fixes to items 5 and 6, and on Nov 4, 2026 after the e2e polish fixes (items 2 and 12).

Screen captures are kept locally, not published: they can expose usernames, project names and terminal content. Recreate the scene with `apps/macos/automation/mockup.sh`; captures land under `.dev/<name>.out/mockup/`.

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

1. **Focused tile outline.** *Decided* (the maintainer, Nov 4): no outline, as in the mockup. Unfocused tiles dim instead: 25 % of the window colour over each, which turns the mockup's focused body and header colours into its unfocused ones (details in [README.md](README.md#focus-indicator-unfocused-tiles-dim-decided-nov-4-2026)). The capture shows it: the claude tile is focused, the other three are dimmed.
2. **Agent badge.** "claude · working" (2.6) used to sit over the terminal's top-right corner and covered its text (e2e V8). Since Nov 4 it sits at the right end of the header strip, the header's title keeps clear of it, and a narrow tile shows the dot alone. The mockup still has no badge. *Designer question*: keep, show only for blocked or finished, or rely on the header?
3. **Workspace status when tabs disagree.** The mockup shows azulejo as working while its "review" tab is blocked. Azulejo ranks blocked first, so the row shows blocked. *Designer question*: which state wins in the sidebar row?
4. **Terminal text size.** The terminals use Ghostty's default font size because the user's Ghostty config is empty, which is larger than the mockup's (about 11 pt), so long lines wrap. *Designer question*: should Azulejo default to a smaller terminal size when the Ghostty config sets none? (*quick fix* once decided)
5. **Preview typography.** *Fixed.* The heading was about twice the mockup's size. `shared/webview/preview.css` now uses the mockup's scale: body 13 px (`font.body`) at 1.5 line height, heading semibold 18 px with no rule under it, code blocks 11 px on the sunken fill, inline code in the error red with no fill, body text `textSecondary` and headings `textPrimary`. Still different: the mockup's code block has syntax colours and its eyebrow line is muted.
6. **Traffic lights missing in this capture.** *Fixed.* The scene never hid them: the capture was the app's render of its content view, and the lights live in the window's title bar outside it. Both in-app renders (`capture` and `shot`) now draw the title bar's buttons where they sit, and `chrome.lights` reads "hidden" if they ever are. They're grey because the automation window is never the key window; the mockup shows an active window's colours.
7. **Browser page blank.** WebKit stops drawing in a window that isn't on screen in front, so the captured page is empty; on screen it draws. *Later* (capture limitation; the preview tile already works round it for automation).
8. **"Done" states.** herdr can't report "done" from the CLI, so sample-app, config and the "release" tab show idle, and the footer reads 1 / 3 / 5 / 0 instead of the mockup's 2 / 2 / 3 / 2. *Later* (fixture limitation; the done icon itself renders). Update from the Nov 3 e2e pass: herdr does turn a state change into "done" by itself. In one B5 run, working → idle on an unfocused tile came through as `done!`, so the fixture could stage done by reporting working, then idle, on an unfocused tile. The run that showed it was timing-dependent (attention.azs expects plain idle there), so treat it as a lead, not a recipe yet.
9. **Branches on remote workspaces.** infra and notes show no branch; remote git status isn't implemented (2.14 deferred). *Later*.
10. **Search placeholder.** "Jump to workspace or tile" where the mockup says "session". Decided: the app keeps "workspace". *Designer question* only for the mockup's wording.
11. **Terminal subtitle truncation.** "zsh — just dogfood…" ends with an ellipsis although there's room. Probably the stand-in's process title; check with a real command. *Quick fix* if it reproduces. (Path subtitles now keep their last folder and lose leading ones, "…/projects/azulejo"; a command still loses its end.)
12. **Sidebar details.** *Fixed* (Nov 4, e2e V12), all through tokens: the Agents | Files control sat 8 pt higher than the mockup's (`space.sidebarSegmentTop`), group rows were a little tight (`sidebarRowHeight` 31, `sidebarGroupHeight`, `sidebarAddHeight`, `sidebarListTop`), workspace names were heavier (`font.sidebarRow` is now light: ink within about 10 % of the mockup's, against 28 % heavier before), footer numbers are tinted by state, and "Reconnecting…" has the mockup's orange arc instead of a grey system spinner. Rows now line up with the mockup's to within 1–2 pt. Side by side: `../testing/e2e-v1/fix-sidebar-mockup-vs-app.png` (local capture).
