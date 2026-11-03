# 0006: Start herdr with a clean environment and size the PTY before attach

Status: accepted · Nov 3, 2026 · Gate G0

## Context

Three findings from the spikes:

- **Env leaks.** Panes inherit the environment of whoever started the herdr server. In spike 0.3, `CLAUDE_CODE_CHILD_SESSION` leaked from the agent's shell into Claude Code inside a pane. The same goes for `HERDR_*` variables when Azulejo itself runs inside herdr: commands then hit the wrong session.
- **Zero-sized PTY.** `herdr terminal attach` errors out with "terminal reported a zero-sized grid".
- **EOF.** The attach client forwards every input byte. A stray EOF (^D) on detach ended the pane's shell.

## Decision

- When Azulejo starts a herdr server (or a named session), it spawns it with a clean environment: a login-shell environment for the user, minus `HERDR_*`, `CLAUDE_*` and anything from the app's own process that panes shouldn't see.
- Every herdr call from the core pins the socket path explicitly and never relies on an inherited `HERDR_SOCKET_PATH`.
- Shells set the PTY size from the tile's frame before spawning the attach process.
- Shells never write EOF to the attach PTY. Detach means closing the PTY master or killing the attach process.

## Consequences

- Panes created by Azulejo look like panes created from a fresh terminal.
- If the user's herdr server is already running (started from elsewhere), we use it as is and can't clean its environment. That's documented, not fixed.

## Evidence

- `spikes/macos-terminal/README.md` and `run.sh` (env scrubbing)
- [Linux renderer notes](../linux-renderer.md), "How herdr's stream behaves"
