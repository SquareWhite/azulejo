# 0013: Workspace order is Azulejo's own

Status: accepted · Nov 5, 2026 · M2.5 (dogfooding feedback) · relates to [0008](0008-use-the-users-herdr-session.md), [0012](0012-mirror-layout-into-herdr.md) · amended by [0014](0014-workspace-close-and-rename-reach-herdr.md)

> Amended by ADR 0014 (Nov 9, 2026): names are no longer Azulejo's own. A rename here goes to herdr, a rename in herdr comes back, and a survey pushes Azulejo's name where they differ. Closed workspaces close in herdr too. Order still isn't mirrored: the other reasons below stand.

> Amended Nov 10, 2026: numbers follow the sidebar across machine groups. Before, a workspace's number was its place in the window's list, but the sidebar and the rail group workspaces by machine, so a remote workspace made between two Local ones took a number from the middle of Local's run: Local showed 1–5, 7, 8 and the remote showed 6. Now the core keeps each window's list grouped the way the sidebar shows it (see "Numbering across machine groups" below), so place, number, `workspace.list`, ⌘N and the rows agree.

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

## Numbering across machine groups

The sidebar and the rail show one group per machine: Local first, then herdr's machines in `machines.list` order (by label), then any machine a workspace still names but herdr no longer lists, in the order its workspaces first appear. Numbers run top to bottom across all of them, so with Local's workspaces 1–7 the first workspace in the next group is 8.

- **The core keeps the list grouped.** After every command and every herdr change, each window's list is sorted into that group order, keeping the order within each group (`Session::group_workspaces`). A workspace made or adopted on a machine joins the end of its group, not the end of the list. A move past its group's edge stops at that edge.
- **Machines coming and going regroup it.** When herdr's list gains, loses or relabels a machine, the core regroups and reports `workspace.reordered`. A machine leaving the list takes its group to the end, and the group goes back when the machine returns. A machine that's only reconnecting stays listed, so nothing moves. A saved session comes back grouped, with remote groups in first-appearance order until their machines connect.
- **The shell counts the same way.** The rows' numbers, their ⌘ tooltips (⌘1…⌘8, and ⌘9 on the last row) and ⌘1…⌘9, next and previous workspace all walk the groups top to bottom (`SidebarView.groups` / `ordered`). With the core's list grouped that's also the list's order, so the shell never shows a number `workspace.list` doesn't.

An empty group (a saved machine with no workspaces in this window, say one still connecting) shows only its header; it takes no number.

## Evidence

- `core/crates/engine/src/tests.rs`: `workspace_move_reorders_the_window_and_the_numbers_follow` (order, `workspace.reordered`, numbers, clamping, unknown IDs).
- `core/crates/engine/src/driver_tests.rs`: `workspace_order_is_azulejos_own_and_survives_a_relaunch` (a fake herdr records no change, a resync doesn't reorder, a relaunch keeps the order).
- `apps/macos/automation/e2e/reorder.sh`: drags in the sidebar and the rail, the relaunch, `azj workspace list` / `move`, and herdr's own order unchanged throughout.
- `core/crates/engine/src/tests.rs`: `workspaces_stay_grouped_by_machine_so_numbers_follow_the_sidebar` (creation order, moves at a group's edge, machines leaving and coming back, an ungrouped saved session). `driver_tests.rs`: `workspaces_follow_the_machine_groups_as_machines_come_and_go`.
- `apps/macos/automation/e2e/wsnum.sh`: two stand-in remote machines; workspaces made Local, remote, Local again; full-sidebar numbers, rail numbers, the core's order, `azj workspace list` and ⌘N through a drag, a close, adoption (Local and remote), a relaunch, a reconnect and a machine leaving herdr's list and coming back.
- herdr's API schema (`herdr api schema --json`, protocol 22): `workspace.move`, `workspace.move_block`, `workspace.moved`, `workspace.reordered`.
