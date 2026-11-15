# 0008: Use the user's own herdr session and adopt what runs there

Status: accepted · Nov 3, 2026 · M1 · supersedes [0007](0007-dedicated-herdr-session.md) · amended by [0012](0012-mirror-layout-into-herdr.md)

> Amended by ADR 0012's amendment (Nov 4, 2026): only an Azulejo op may change the structure of a herdr tab; an adopted tab, and any tab whose arrangement came from herdr, follows herdr's tree and is never pushed into another shape.
>
> Amended by ADR 0012 (Nov 4, 2026): adoption and restore still only read, but Azulejo now changes the layout of the herdr tabs it shows (adopted ones too) to mirror its own, moving panes between tabs and setting ratios and zoom. It still never calls `layout.apply` and never closes a pane except on an explicit kill. "Tiles Azulejo creates still get a herdr tab each" no longer holds: they're panes in their tab's herdr tab.

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

## Amendment (Nov 4, 2026): closing a workspace detaches it

> Superseded by [ADR 0014](0014-workspace-close-and-rename-reach-herdr.md) (Nov 9, 2026): closing a workspace now closes its herdr workspace too. The detach bookkeeping below still applies when that close doesn't reach herdr, and such workspaces are reported, not adopted.

`cmd+w` now closes the workspace (`close_workspace`, `cmd+shift+w` closes the tab). A closed workspace follows the closed-tile rule above: every terminal's pane goes on `Session.detached_herdr`, nothing is killed, and herdr keeps the workspace, its tabs and panes. A survey skips the herdr workspace because all its panes are known, and so does a relaunch (the list is saved with the session). No separate list of closed workspaces is needed. One gap closed on the way: a pane herdr finishes making after its tile (or the tile's workspace) closed now counts as detached too, instead of being adopted as a new workspace on the next survey. A tab created later in that herdr workspace (from the phone, say) is new work and is adopted as usual. Reopening a closed workspace is future work, like reattaching a closed tile.

Evidence: `adopt_tests.rs` `a_closed_workspace_detaches_and_stays_closed`, `a_pane_made_for_a_closed_workspace_isnt_adopted`; live `closing_a_workspace_detaches_it_and_a_relaunch_keeps_it_closed` (herdr's structure identical after the close and after a relaunch).

## Amendment (Nov 9, 2026): cmd+w kills the focused pane

The user decided this one. `cmd+w` closing the whole workspace took too much with it, and a closed tile's pane running on unseen in herdr wasn't what they wanted from `cmd+w` either. So `cmd+w` now runs `close_pane`: the focused tile closes and its herdr pane is killed (`pane.close`) straight away, with no confirmation, the way `cmd+w` closes the innermost thing in any Mac app and herdr's own close pane ends the process. F14 changes with it.

- `alt+w` (`close_tile`) still detaches: the pane keeps running in herdr, as in the rule above.
- `kill_tile` (Kill Tile… in the menus, the palette, `prefix+shift+k`) does the same as `close_pane` but still asks first.
- A tab's last tile closes the tab. With `alt+w` the workspace stays even when that was its last tab. With `cmd+w` the workspace's last pane is gone, herdr deletes the workspace, and Azulejo closes it too (`close_herdr_workspace`), unless it's the window's last workspace, which stays, empty. That's how `kill_tile` already behaved.
- `close_workspace` moves to `alt+cmd+w` (macOS "Close All") and keeps its File menu item; `cmd+shift+w` still closes the tab. Since [ADR 0014](0014-workspace-close-and-rename-reach-herdr.md) (same day) it closes the herdr workspace too, and asks first when something still runs there.
- The "never closes a pane except on an explicit kill" rule stands: `cmd+w` is now an explicit kill.

Evidence: the B2 chords suite (`e2e/chords.azs.in`) checks that `cmd+w` closes the tile with no dialog and that exactly that pane leaves herdr's `pane list`, the others still running; `keys.azs` checks the pane is gone too.
