# 0005: Scrollback, search, title and attention come from herdr, not the terminal

Status: accepted · Nov 3, 2026 · Gate G0 · Changes PRD F11

## Context

`herdr terminal attach` streams herdr's re-rendered screen, not the program's bytes: synchronised-output frames with absolute cursor moves, no line feeds, on the alt screen. So the local terminal (libghostty, VTE, libghostty-vt) never builds scrollback. Many out-of-band sequences are dropped:

- **Pass through:** truecolour, underline styles, OSC 8 links, cursor shape, kitty keyboard push/pop.
- **Dropped:** OSC 52 clipboard, OSC 9/777/99 notifications, BEL, kitty graphics (0.9.1), DECCKM and focus reporting.
- **Moved to herdr events:** title and cwd. Title arrives in `pane.updated`; cwd comes from the process.

`TERM` inside panes is `xterm-256color`, set by herdr. No herdr event reports OSC notifications.

## Decision

- **Scrollback:**
  - Mouse wheel goes through attach; herdr scrolls its own buffer.
  - Keyboard scrollback uses `pane.scroll` (or controller `terminal.scroll`).
  - Find-in-scrollback uses `pane.read --source recent` / `pane.copy_search`.
  - F20 "read scrollback" maps to `pane.read`.
  - libghostty's own scrollback, search and jump-to-prompt are unused in terminal tiles.
- **Title and cwd:** from herdr's `pane.updated` and pane fields, not surface callbacks.
- **Attention (F11):** from `pane.agent_status_changed`. `blocked` raises attention and `done` shows a soft badge. Generic OSC 9 "command finished" notifications are out until herdr exposes them.
- **Clipboard:** copying text selected in the tile works locally. OSC 52 from apps (vim, agents) doesn't; `pane.selection.read` covers herdr-side selections.

## Consequences

- Users lose OSC 52 copy from inside apps. This is the most visible regression; it's at the top of the upstream asks.
- Scrollback survives an app restart, but not a herdr server restart unless `[experimental] pane_history = true`.
- Upstream asks (tracked in [herdr notes](../herdr-notes.md)): a `pane.notification` event, OSC 52 passthrough, `--no-detach-key`, DECCKM and focus forwarding, non-destructive `layout.apply`, clearer exit reasons.

## Evidence

- [herdr notes](../herdr-notes.md), questions 1, 2, 5 and 8
- [Linux renderer notes](../linux-renderer.md), "How herdr's stream behaves"
