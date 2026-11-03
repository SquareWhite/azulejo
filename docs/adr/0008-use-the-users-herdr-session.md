# 0008: Use the user's own herdr session and adopt what runs there

Status: accepted · Nov 3, 2026 · M1 · supersedes [0007](0007-dedicated-herdr-session.md)

## Context

ADR 0007 put Azulejo's terminals in a separate herdr session, `azulejo`, so Azulejo could start that server with a clean environment and never touch the user's hand-run herdr. The user overruled it: they want Azulejo to connect to the herdr session they already use, by default, so the TUI in kitty, the phone and Azulejo all see the same workspaces.

That means Azulejo has to deal with herdr state it didn't create: workspaces, tabs with several panes, and splits the user made by hand.

## Decision

- **Default session.** `[herdr] session` defaults to herdr's default session (`~/.config/herdr/herdr.sock`). A running server is used as it is and never restarted. If it isn't running, Azulejo starts it with a clean environment (ADR 0006). `session = "NAME"` picks a named session; `enabled = false` turns herdr off.
- **Adoption.** On connect, after herdr reports lost events (resync), and when herdr announces a new workspace or tab, the core reads herdr (`session.snapshot`, plus `layout.export` for every tab with more than one pane) and adopts what nobody here knows:
  - each herdr workspace without an Azulejo workspace becomes one, named after its herdr label, and bound to it;
  - each herdr tab whose panes are all unknown becomes one Azulejo tab, with herdr's split tree (direction and ratio) and one terminal tile per pane, bound by pane, terminal and tab ID;
  - a pane counts as known if a tile is bound to it, or if a tile showing it was closed here (`Session.detached_herdr`). That list is pruned once herdr no longer has the pane.
- **Read-only.** Adoption and restore only read herdr: `session.snapshot`, `layout.export`, `pane.list`, `workspace.get`. Never `layout.apply`, `pane.close` or anything that moves or re-lays out a pane. The only herdr calls that change anything are a terminal tile Azulejo creates (its own new tab) and an explicit kill.
- **First launch.** A fresh session starts with a placeholder workspace holding one empty terminal. If adoption brings in any of the user's workspaces, the placeholder is dropped before it ever gets a herdr pane, and herdr's focused workspace gets focus.
- **Mixed tabs.** This relaxes ADR 0002: an adopted tab can hold several tiles that share one herdr tab. Tiles Azulejo creates still get a herdr tab each. Closing an adopted tile detaches it (F14); killing stays explicit.
- **Not yet.** A pane split off inside a tab Azulejo already shows isn't adopted. That's PLAN step 2.7.

## Consequences

- **Environment.** If the user's herdr server was started from a terminal, every pane, including the ones Azulejo creates, inherits that terminal's environment. Azulejo can't clean it. Probed on herdr 0.9.1: `--env KEY=` sets the variable to an empty string, it doesn't unset it, so blanking `CLAUDE_*` would only half work. Azulejo passes its own `AZULEJO_*` variables and leaves the rest alone. Only a server Azulejo starts itself gets the clean environment.
- **Sizes.** A tile direct-attaches to its pane, and while attached it sets that pane's size (herdr notes, question 3). If the user's herdr TUI shows the same pane, it keeps the pane in its own layout slot but the content follows Azulejo's tile size: expect cropping or empty space in the TUI while the tile is open. What the TUI draws exactly is still untested (deferred list). Closing the tile hands the size back.
- **Phone.** The phone sees the same session with no extra flags. The "Release to phone" idea from the herdr notes still applies.
- **First launch.** Azulejo opens with one workspace per herdr workspace and one tab per herdr tab, splits included. Nothing changes in herdr.
- **Closed tiles.** A tile closed in Azulejo stays closed on relaunch even though its pane keeps running in herdr. Bringing one back (a palette "attach herdr pane" action) is future work.
- **herdr restarts.** A pane that survived a herdr restart as a fresh shell in the same slot is matched by pane and tab ID, so it's re-bound rather than adopted twice. A closed tile's pane that herdr restarted can come back, because its terminal ID changed and the slot may be reused.
- **Tests.** Every test still runs against throwaway named sessions (`azj-test-*`, `azj-e2e-*`); nothing in CI or `just` touches the user's default session.

## Evidence

- `core/crates/engine/src/adopt.rs` and `adopt_tests.rs`: adoption from a survey, idempotence, detached tiles, herdr restarts, bad layouts.
- `core/crates/engine/src/driver_tests.rs`: `adopting_and_restoring_never_change_herdr` (a fake herdr records every call; adopt and restore make only reads, on first launch and on relaunch).
- `core/crates/engine/tests/live.rs`: `adopts_what_herdr_already_runs_without_changing_it`. A throwaway session with a three-pane tab is adopted with the same split tree, and herdr's structure is identical before and after, including after a relaunch.
- [herdr notes](../herdr-notes.md), questions 3 and 6
