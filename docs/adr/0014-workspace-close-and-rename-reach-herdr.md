# 0014: Closing and renaming a workspace reach herdr

Status: accepted · Nov 9, 2026 · dogfooding feedback · amends [0008](0008-use-the-users-herdr-session.md) (its "closing a workspace detaches it" amendment), [0009](0009-changes-made-in-herdr.md) and [0013](0013-workspace-order.md) (names only; order stays Azulejo's own)

## Context

Dogfooding: Azulejo and herdr drift apart. Workspaces closed in Azulejo stay open in herdr, so herdr lists more workspaces than Azulejo, and a workspace renamed in Azulejo keeps its old label in herdr's TUI and on the phone.

Reproduced on a dev instance (herdr 0.9.5, protocol 22): three workspaces made through `azj`, one renamed, one closed. Azulejo then listed `w1 main, w2 alpha, w3 bravo-renamed`, herdr `w1 main, w2 alpha, w3 bravo, w4 charlie`, and a relaunch kept both lists as they were. The mapping wasn't the problem: every Azulejo workspace stayed bound to the right herdr workspace, before and after the relaunch, and nothing failed or got logged. The calls were never made, on purpose:

- ADR 0008's amendment made ⌘W detach a workspace like a closed tile (F14): every pane went on `Session.detached_herdr`, herdr kept the workspace, and the survey skipped it because all its panes were known.
- ADR 0013 kept names to Azulejo: "a rename here isn't sent to herdr, and a rename in herdr doesn't come back".

herdr has everything needed: `workspace.close {workspace_id}` (closes the tabs and panes and sends `workspace_closed`), `workspace.rename {workspace_id, label}` (sends `workspace_renamed {workspace_id, label}`), and both answer `workspace_not_found` once it's gone. Checked live in `azulejo-herdr`'s `renames_and_closes_a_workspace`.

## Decision

**Azulejo drives workspace close and rename; renames made in herdr come back.**

- **Close.** `workspace.close` (⌥⌘W, the menu bar, the sidebar, the API) closes the herdr workspace with every tab and pane in it, so their processes end. `window.close` does the same for each workspace of the window. Quitting doesn't: the shell quits instead of closing the last window, and herdr keeps everything for the relaunch. Only these explicit commands close anything in herdr; a workspace herdr closed, or the first-launch placeholder that adoption drops, never does. A herdr workspace another Azulejo workspace still shows is left alone. Closing a tab, or a tile with `alt+w`, still detaches; `cmd+w` kills the focused pane (F14, ADR 0008's Nov 9 amendment).
- **Rename.** `workspace.rename` sets the herdr workspace's label. herdr's `workspace_renamed` renames the Azulejo workspace showing it. Our own renames come back as echoes; an echo of an older name while a newer one is on its way is skipped, so typing two names quickly doesn't flash the first.
- **Drift found by a survey: Azulejo's name wins.** On the first survey after each connect or resync, a bound workspace whose herdr label differs gets Azulejo's name pushed. That covers drift from before this change, renames made while herdr was away, and renames made in herdr while Azulejo wasn't running (herdr doesn't replay events, so we can't tell which side changed). Later surveys (a new tab or workspace in herdr) don't compare names: while the stream is up, herdr's renames arrive as events, and a survey racing one could otherwise put the old name back. An unlabelled herdr workspace is left alone.
- **Workspaces not yet bound.** Close and rename go through the same per-machine worker as pane creation, in order, so a workspace whose herdr workspace is still being made gets closed or renamed once it exists.
- **Orphans: left running and reported, never closed or adopted.** herdr workspaces that no Azulejo workspace shows after adoption (every tab holds a pane closed here) are mostly workspaces closed before this change. Azulejo logs their herdr IDs (no names) once per change of the list, on each survey: `herdr (Local): N workspace(s) not shown in Azulejo, left running: w3, w5. …`. The user closes them in herdr (`herdr workspace close <id>`) or leaves them. A close that fails (herdr unreachable) ends up in the same report on the next survey.
- **Failures are logged**, with the herdr workspace ID and herdr's error, never the name: `herdr: closing workspace w3: …`, `herdr: renaming workspace w3: …`.

Why leave orphans alone: adopting them would bring back, possibly in bulk, workspaces the user had closed on purpose; closing them would end processes the user may still want (a long job, an agent reached from the phone). Reporting keeps both options open and costs nothing.

## Consequences

- Closing a workspace now ends processes, so the shell asks first when something runs there (see "Amendment" below).
- herdr's TUI and the phone show the same workspaces and names as Azulejo. Order still differs after a reorder (ADR 0013).
- A rename made in herdr's TUI while Azulejo isn't running is undone by the next launch.
- `Session.detached_herdr` still matters for closed tiles and tabs, and for a workspace close that didn't reach herdr.

## Evidence

- `core/crates/engine/src/driver_tests.rs`: `closing_a_workspace_closes_it_in_herdr` (also one closed before its herdr workspace is bound), `closing_a_window_closes_its_workspaces_in_herdr`, `renames_go_to_herdr_and_herdr_renames_come_back` (stale echo skipped, a rename in herdr not pushed back), `a_survey_puts_azulejos_names_back_into_herdr`.
- `core/crates/engine/src/adopt_tests.rs`: `a_closed_workspace_detaches_and_stays_closed` (the close effect, and the orphan report when it never reached herdr).
- `core/crates/engine/tests/live.rs`: `closing_and_renaming_a_workspace_reach_herdr_and_survive_a_relaunch`; `core/crates/herdr/tests/live.rs`: `renames_and_closes_a_workspace`.
- `apps/macos/automation/e2e/wssync.sh`: the app on a dev instance, compared with `herdr workspace list` after each step, a rename in herdr, a relaunch, and an orphan reported but left running.

## Amendment (Nov 9, 2026): ask before ending running processes

The user asked for a Ghostty-style confirmation. Closing a workspace (any route through the shell: the keymap, the menu bar, the sidebar menu, ⌘K) or a window that isn't the last now shows a sheet on that window, "Close “name”? A process is still running: … Closing ends it, and herdr closes the workspace too", with Close and Cancel, when something besides an idle shell runs in one of its terminals. A sheet never activates the app or raises the window. An idle workspace closes at once.

"Running" comes from what the core already knows per terminal tile, no extra herdr call:

- the foreground command, which the core polls from herdr's `pane.process_info` every second (`tile.pane.command`, set only when the foreground process isn't the shell; F28), or
- a working or blocked agent (`tile.agent.status`, from herdr's agent events), for an agent whose process the poll hasn't seen.

Ended tiles and tiles without a pane don't count. A pane not polled yet (the first second after it's made) counts as idle rather than prompting on every quick ⌘T ⌥⌘W. herdr panes the workspace holds but no tile shows (detached, or a tab not adopted yet) aren't polled, so they don't count either. `workspace.close` and `window.close` over the API and CLI never ask: scripts decide for themselves.

Evidence: `apps/macos/automation/e2e/wssync.sh` (`confirm.azs`): `sleep 600` in one workspace, Cancel keeps it, an idle workspace closes without a prompt, Close ends the busy one and herdr closes its workspace. Automation answers with `menu answer`, and `prompt.asks` / `prompt.last` count and show the prompts.
