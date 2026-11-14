# 0009: Changes made in herdr: closed, exited, split and agent status

Status: accepted · Nov 3, 2026 · M2 (PLAN 2.6, 2.7) · amends [0008](0008-use-the-users-herdr-session.md)

## Context

With Azulejo on the user's own herdr session (ADR 0008), panes change under it: the herdr TUI, the phone, the CLI and agents split, close and exit them. F15 wants those changes in the app within a second; F11 and F16 want agent state on tiles. Until now any pane that went away left its tile "ended", and panes split off inside a tab already shown here weren't picked up.

Probed on herdr 0.9.1 (throwaway sessions):

- A shell that exits (`exit`, a crash, SIGKILL) makes herdr drop the pane and send only `pane_exited`. When it was the workspace's last pane, herdr deletes the workspace silently: no `tab_closed`, no `workspace_closed`.
- `pane close` sends only `pane_closed`. `tab close` sends only `tab_closed`; closing a workspace's last tab adds `workspace_closed`, whose data carries the whole workspace object, not a bare `workspace_id`.
- `pane split` sends `pane_created` (and a `pane_updated` for its title). Nothing says which pane it was split from; `layout.export` of the tab does.
- `pane report-agent` reaches only a per-pane `pane.agent_status_changed` subscription; it fires no `pane_updated`. The CLI can report `idle`, `working`, `blocked` and `unknown`; `done` comes from herdr's own detection. `session.snapshot` carries each pane's current `agent` and `agent_status`.
- herdr doesn't replay events from before a subscription starts.

## Decision

- **Exited → ended.** `pane_exited` marks the tile ended; the shell offers Respawn or Close, as before.
- **Closed elsewhere → removed.** `pane_closed` and `tab_closed` close the tiles bound there, as if closed in Azulejo but without remembering them in `detached_herdr` (the panes are gone). `workspace_closed` does the same for the workspace's remaining tiles, drops the Azulejo workspace's herdr binding (so a later workspace reusing the ID isn't mistaken for it), and closes the Azulejo workspace if that left it without tabs and it isn't its window's last.
- **Unknown cause → ended.** A pane that turns out to be gone after a reconnect or at startup (re-resolve says `Gone`) is marked ended, never removed: we can't tell whether it exited or was closed.
- **Split-off panes.** `pane_created` for a pane no tile shows asks for a survey. A pane in a herdr tab that already has a tile here joins that tile's Azulejo tab beside its herdr sibling: same side, same ratio, without moving focus or zoom. With no layout or no bound sibling it goes to the end of the tab's stack (`layout.split_auto` rules). Panes in wholly new tabs are adopted as before (ADR 0008). Panes closed here stay out.
- **Agent status.** The core keeps one `pane.agent_status_changed` subscription for every bound pane, replaced (overlapping until herdr confirms the new one) whenever the set changes. Every survey also refreshes agents from the snapshot. A terminal tile carries `agent: {name, status, since_ms, attention}`.
- **Attention.** Becoming blocked or done lights `attention`; going back to work clears it (someone answered, maybe from the phone); done → idle keeps it (still unseen). Focusing the tile clears it, and so does `tile.seen`, which shells send when an already focused tile lights up while the user is looking. Workspaces report counts (`blocked`, `attention`) for the sidebar. Events: `agent.state_changed`, `tile.attention_changed`, plus `tile.updated` / `workspace.updated`.
- **Startup.** The first `Connected` on herdr's event stream triggers the same re-resolve and survey as a resync, so changes made while the stream was still connecting aren't lost.

## Consequences

- Closing a pane in the herdr TUI closes the tile in Azulejo; exiting a shell leaves an ended tile to respawn. The two look different on purpose: one is a choice, the other an accident.
- A pane split off a tile the user closed here (detached) isn't adopted: its herdr tab has a known pane but no tile to sit beside. Rare; the survey ignores it.
- One extra herdr connection for agent status, plus a short-lived second one while the pane set changes.
- The core's attention doesn't know whether the app is in front. Shells decide when a focused tile counts as seen.

## Evidence

- `core/crates/engine/tests/live.rs`: `agent_status_and_changes_made_in_herdr_reach_the_core_within_a_second`. Measured on an Apple Silicon, herdr 0.9.1, 5 runs, times from starting the herdr CLI command (≈ 95 ms of it is the CLI itself): blocked → `agent.state_changed` and `tile.attention_changed` 101–107 ms; `pane split` → `tile.created` 99–102 ms; `pane close` → `tile.closed` 103–104 ms. Target: under 1 s.
- `core/crates/engine/src/driver_tests.rs`: `herdr_events_end_close_or_rebind_tiles`, `a_closed_herdr_workspace_takes_its_tiles_and_binding`, `agent_status_follows_bound_panes_and_lights_attention`, `panes_split_off_elsewhere_become_tiles_and_closing_there_removes_them`.
- `core/crates/engine/src/adopt_tests.rs`: `a_pane_split_off_in_herdr_lands_beside_its_sibling`, `split_panes_without_a_tree_go_to_the_stack_unless_closed_here`.
- `core/crates/engine/src/agent.rs`: attention rules.
