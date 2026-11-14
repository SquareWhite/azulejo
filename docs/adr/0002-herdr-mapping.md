# 0002: One herdr tab per terminal tile

Status: accepted · Nov 3, 2026 · Gate G0 · amended by [0008](0008-use-the-users-herdr-session.md)

> Amended by ADR 0008: herdr tabs that already exist are adopted whole, so an adopted tab can hold several tiles mirroring herdr's splits. Tiles Azulejo creates still get one herdr tab each.

## Context

Azulejo owns the layout. herdr has its own workspaces, tabs and split tree. We had to choose between mirroring Azulejo's splits into herdr (so the phone sees the same arrangement) and keeping herdr's structure flat.

The probe found three things:

- `layout.apply` on an existing tab is destructive: it creates a new tab with fresh panes and kills the original processes, ignoring the `pane_id` passed in.
- While a direct-attach client is connected, the pane's size comes from that client, not from herdr's layout, so mirrored geometry buys nothing on the Mac.
- On a phone, herdr falls back to a single-column layout below 64 columns, where one tab per agent reads naturally.

## Decision

- Each app workspace maps 1:1 to a herdr workspace.
- Each terminal tile maps to its own herdr tab holding one pane. Create it with `tab create --label <tile title> --env AZULEJO_TILE_ID=… --env AZULEJO_WORKSPACE_ID=… --env AZULEJO_SOCKET=…`.
- Never call `layout.apply` on a tab with live panes.
- A tile stores both `pane_id` and `terminal_id`. Pane IDs change when a pane moves to another workspace, so re-resolve `pane_id` from `terminal_id` (via `pane.list`) after `pane.moved` and on relaunch.
- Panes created elsewhere (phone, CLI, an agent splitting a herdr tab) are adopted as new tiles. Azulejo doesn't copy herdr's split geometry.
- Subscribe to `pane.agent_status_changed` per pane; it needs a `pane_id`.

## Consequences

- The phone shows a strip of tabs, one per terminal tile, rather than Azulejo's layout.
- herdr's split tree stays trivial, and Azulejo's layout tree is the only source of truth.
- Whether `terminal_id` survives a move across workspaces is untested. Check it in step 1.4.

## Evidence

- [herdr notes](../herdr-notes.md), questions 6, 8 and 9
