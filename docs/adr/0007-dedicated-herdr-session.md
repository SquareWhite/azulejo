# 0007: Terminal tiles live in a dedicated herdr session, `azulejo`

Status: accepted · Nov 3, 2026 · M1 integration

## Context

herdr can run several servers side by side, one per named session (`herdr --session NAME`). The user already runs herdr's default session by hand, in kitty. Azulejo could share that server or run its own.

- **Clean environment.** ADR 0006 needs Azulejo to start the herdr server itself, or panes inherit whatever environment the server was started with. A default session the user started from a terminal (often from inside an agent's shell) can't be cleaned.
- **Workspace sync.** Azulejo maps herdr workspaces 1:1, adopts external panes as tiles (step 2.7) and marks tiles ended when their tab closes (ADR 0002). Pointed at the user's hand-run session, it would adopt every pane there and treat the user's own layout changes as its own.
- **Restarts.** A herdr server restart loses processes (PRD risks). Azulejo never restarts herdr, but a separate server also keeps a crash or `herdr update` in one session from taking the other with it.
- **Phone access.** A phone reaches a named session with `herdr session attach azulejo` (or `--session azulejo`) instead of plain `herdr`.

## Decision

- By default, Azulejo uses the named herdr session `azulejo`, started by Azulejo with a clean environment when it isn't running.
- `[herdr] session = "NAME"` picks another named session; `session = "default"` opts into herdr's default session, with the clean-environment caveat from ADR 0006; `enabled = false` turns herdr off (terminal tiles stay unbound).
- `core.version` reports `herdr_session`, `herdr_socket` and `herdr_bin`, so `azj` can tell whether a `HERDR_PANE_ID` belongs to Azulejo's session (it compares `HERDR_SOCKET_PATH`).
- `azulejod --herdr-session NAME` and `azj_core_new_with_options` override the config, for tests and development.

## Consequences

- Panes Azulejo creates don't show up in the user's existing herdr TUI, and the user's hand-run panes don't show up in Azulejo. Someone who wants one view sets `session = "default"`.
- The phone attaches to `azulejo` explicitly. The onboarding and docs must say so.
- Two herdr servers run while the user keeps using herdr by hand; each is a ~10 MB process.

## Evidence

- `core/crates/daemon` (boot path) and its end-to-end test, `just e2e`
- [herdr notes](../herdr-notes.md), questions 3 and 6
