# 0013: Workspace order is Azulejo's own

Status: accepted · Nov 5, 2026 · M2.5 (dogfooding feedback) · relates to [0008](0008-use-the-users-herdr-session.md), [0012](0012-mirror-layout-into-herdr.md)

## Context

the maintainer wants to drag workspaces in the sidebar to reorder them. The order has to survive a relaunch, show in the API and CLI, and drive ⌘1…⌘8 and ⌘9 (the last workspace).

Azulejo already keeps an order: each window's `workspaces` list, saved with the session, with `workspace.move` (core, API, CLI, Python) and a `workspace.reordered` event. Nothing in the UI used it yet.

herdr has an order of its own. On herdr 0.9.1 (protocol 22) every workspace has a `number`, and the API has `workspace.move` (`workspace_id`, `insert_index`), `workspace.move_block` and the events `workspace.moved` and `workspace.reordered`. Its TUI numbers workspaces by it, and its own keys switch to a workspace by number. Adoption (ADR 0008) already takes herdr's order once: new workspaces come in sorted by herdr's number.

So the question: should a drag in Azulejo move the workspace in herdr too, and should a move in herdr's TUI reorder Azulejo?

## Decision

**Azulejo keeps its own order and doesn't mirror herdr's, either way.**

- A drag (or `workspace.move`) changes the window's list and nothing in herdr. Adoption still takes herdr's order for workspaces it brings in, appended after the ones already there.
- herdr's `workspace.moved` / `workspace.reordered` aren't subscribed to; a survey or resync never reorders Azulejo's list.
- The saved order wins on relaunch.

Why not mirror it:

- **It's the same kind of thing as a name.** Azulejo already keeps workspace names to itself: a rename here isn't sent to herdr, and a rename in herdr doesn't come back. Order is per-workspace presentation in the same way. ADR 0012 mirrors the *inside* of a tab (its panes and splits) so the TUI and the phone show the same arrangement; it doesn't cover the list of workspaces.
- **The lists don't line up one to one.** One Azulejo window interleaves workspaces from several herdr servers (ADR 0011), each with its own numbering. Two Azulejo windows share one herdr. herdr keeps workspaces that Azulejo closed (they detach, ADR 0008 amendment), and Azulejo can hold a workspace whose herdr workspace doesn't exist yet. Mirroring would need a projection onto each server's list and, coming back, rules for which slots herdr's order may permute. Both are guesses about what the user meant.
- **It would renumber the user's TUI under them.** herdr's numbers are what its own keys switch by. Dragging a row in Azulejo silently changing which workspace is 3 in the TUI is a surprise, and goes against ADR 0008's rule of touching only what Azulejo needs to.

## Consequences

- The sidebar, the rail, ⌘1…⌘8 / ⌘9, `workspace.list` (which now also gives each workspace's `number`, from 1) and `azj workspace list` all follow Azulejo's order.
- After a reorder, herdr's TUI and the phone may list the same workspaces in another order than Azulejo. That's accepted: they match at adoption and drift only when the user reorders on one side.
- A drag never moves a workspace to another machine group: its terminals run on that machine.
- If this turns out to matter in daily use, mirroring can be added later as a one-way push for single-machine windows (`workspace.move` with the index among that machine's bound workspaces) without changing the model.

## Evidence

- `core/crates/engine/src/tests.rs`: `workspace_move_reorders_the_window_and_the_numbers_follow` (order, `workspace.reordered`, numbers, clamping, unknown IDs).
- `core/crates/engine/src/driver_tests.rs`: `workspace_order_is_azulejos_own_and_survives_a_relaunch` (a fake herdr records no change, a resync doesn't reorder, a relaunch keeps the order).
- `apps/macos/automation/e2e/reorder.sh`: drags in the sidebar and the rail, the relaunch, `azj workspace list` / `move`, and herdr's own order unchanged throughout.
- herdr's API schema (`herdr api schema --json`, protocol 22): `workspace.move`, `workspace.move_block`, `workspace.moved`, `workspace.reordered`.
