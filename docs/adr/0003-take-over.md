# 0003: Take-over: ask first, fall back to read-only, never retake automatically

Status: accepted · Nov 3, 2026 · Gate G0 · Resolves the PRD's take-over vs mirror question

## Context

herdr allows one writable direct-attach client per terminal. A second `attach` without `--takeover` fails. With `--takeover`, the new client wins: the old one exits 1, and the pane resizes to the new client. Read-only observers (`terminal session observe`) can run next to a writer, each at its own size.

The TUI client a phone uses over SSH isn't a direct-attach writer. It doesn't fight for the lock, but while a direct client is attached it sees panes at that client's size. herdr sends no "you lost ownership" event; the only signal is the attach client's exit.

## Decision

- **First attach:** try without `--takeover`. If another client owns the pane, show the tile read-only through `observe` at the tile's size, with a **Take over** action (re-attach with `--takeover`).
- **Taken over while attached:** switch the tile to read-only observe with **Take back**. Never retake automatically, or two windows or machines will ping-pong.
- **Release to phone:** an explicit action detaches the tile so the TUI's size wins.
- **Tile states come from the attach client's exit:**

| Exit | stderr (last line) | Tile state |
| --- | --- | --- |
| 0 | none | Detached. Reattach silently unless the app asked for the detach (accidental `ctrl+b q`) |
| 1 | `… terminal attach taken over` | Taken over: observe + Take back |
| 1 | `… already has an attached client; retry with --takeover` | Owned elsewhere: observe + Take over |
| 1 | `… terminal term_… not found` | Session ended: Respawn or Close |
| 1 | `… server is shutting down` | herdr gone: reconnect with backoff |

## Consequences

- Parsing stderr is fragile; every message starts with "server shut down" even when the server is fine. The patterns live in the herdr adapter only, with tests pinned to the supported herdr version. Clearer exit reasons are on the upstream ask list.
- After the taker leaves, the pane keeps the taker's size until we reattach.
- A dedicated phone app using direct attach would hit the same take-over flow. That's fine, since it's explicit both ways.

## Evidence

- [herdr notes](../herdr-notes.md), question 3 and the exit table
- [Linux renderer notes](../linux-renderer.md), "Spike results" (takeover)
