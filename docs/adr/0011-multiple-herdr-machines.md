# 0011: Several herdr machines at once, taken from herdr

Status: accepted · Nov 3, 2026 · M2 (PLAN 2.13, PRD F17) · amended Nov 4, 2026: `connected` means the bound panes are checked (see "Amendment")

## Context

The design mockup groups the sidebar by machine: "Local", "build-server", "dev-server", each with a connection dot, "Reconnecting…" when its link drops, its own workspaces and a "+" to create one there. The user decided to build this before the GUI polish, and then added: "remote machines are a feature of herdr, we should be getting those from herdr".

herdr already has machines. `herdr machine add` saves an SSH profile (an opaque 32-hex ID, a label, an SSH target, an explicit remote session, enabled or not) in herdr's own catalogue; the herdr TUI shows them in its sidebar; `herdr --machine <label-or-id> …` forwards API-backed CLI commands to them. Under the hood (herdr v0.9.5 source, `src/remote/saved.rs` and `src/remote/attach.rs`) forwarding is an SSH stdio bridge: herdr runs `ssh … <target> -- <remote herdr> --session <S> remote-api-bridge` and speaks the normal NDJSON API over the child's stdin and stdout. The remote side checks the capability with `remote-api-bridge --check`, which prints `herdr-api-bridge-v1` (present in the user's local 0.9.1 too). One bridge carries one API connection: one request and its reply, or one long `events.subscribe` stream (probed on 0.9.1: the bridge exits after the first reply; a subscription stays open and delivers events within ~100 ms).

What herdr does **not** forward: interactive attachment. `herdr --machine X terminal attach` is rejected ("--machine does not run local management commands or attach a TUI"), and `herdr --remote <target>` attaches the whole TUI, not one terminal. A terminal tile needs one terminal.

Only `herdr machine list --json` reads the catalogue without connecting (`src/cli/machine.rs`: it loads the catalogue and prints `{id, label, target, session, enabled, selected}`); `herdr machine status` and `reconnect` do connect.

## Decision

- **herdr is the source of truth for machines.** Azulejo reads the list with `herdr machine list --json` at start, when the config reloads, and every 60 s (it's a local file read), and diffs it: added machines connect, removed or disabled ones disconnect and their workspaces stay as they are, unbound. Adding, renaming and removing machines happens in herdr (`herdr machine add` etc.); Azulejo never edits the catalogue. Local is always there: the session from `[herdr] session` (ADR 0008).
- **Azulejo keeps presentation only**, keyed by herdr's machine ID or label:

  ```toml
  [herdr]
  machines = true                  # default: true with herdr's default session, false with a named one

  [herdr.machine."dev-server"]          # label or ID
  hidden = true                    # not shown, not connected
  connect = false                  # shown as "Off", not connected
  ```

  No targets, hosts, users, ports or keys. A named `[herdr] session` (tests, dev profiles) turns machines off unless `machines = true` says otherwise, so an isolated setup never reaches the user's real machines by accident.
- **API: herdr's own bridge.** For each enabled remote machine the herdr adapter opens connections by spawning `ssh <options> <target> -- <script>` where the script finds the remote herdr and `exec`s `herdr --session <S> remote-api-bridge`. It looks where herdr's own saved-machine lookup does (`remote_binary_candidates` in `src/remote/attach.rs`), in the same order: `command -v herdr`, then `~/.local/bin/herdr` (where herdr installs itself on a remote), Homebrew (`/opt/homebrew`, `/usr/local`, linuxbrew), mise (`latest`), Nix, `/usr/bin`. Like herdr, it runs `remote-api-bridge --check` on each and takes the first that prints `herdr-api-bridge-v1`, so a stale copy early in the list can't hide a newer one. Terminal attach and observe use the same lookup, so all three always run the same binary. The adapter's `Client` gets a transport: a Unix socket (Local) or this bridge command; everything above it — requests, `events.subscribe`, reconnect with backoff, the version check, the survey and adoption — is unchanged. herdr itself does exactly this for `--machine`; we spawn the bridge directly instead of the CLI because the CLI has no raw API passthrough and no event stream.
- **SSH options** follow herdr's non-interactive set: `BatchMode=yes`, `NumberOfPasswordPrompts=0`, `StrictHostKeyChecking=yes`, `ConnectTimeout=10`, `ConnectionAttempts=1`, `ServerAliveInterval=15`, `ServerAliveCountMax=4`, `-C`, plus our own multiplexing: `ControlMaster=auto`, `ControlPersist=600`, `ControlPath=/tmp/azj-<uid>-ssh/%C` (a 0700 directory; macOS's `$TMPDIR` is too long for a Unix socket path once ssh adds its temporary suffix). Every request, the event stream and every terminal tile reuse one SSH connection per machine; a new bridge costs a channel, not a handshake. The user's `~/.ssh/config`, agent and keys do the authentication. Azulejo stores no credentials and never answers a prompt: a machine that needs a password or an unknown host key shows "Needs attention: run `herdr machine reconnect <label>`".
- **Terminal tiles on a remote machine** run `ssh -tt <same options> <target> -- <script>` with the script `exec`ing `herdr --session <S> terminal attach <terminal-id> [--takeover]`. Exit codes pass through ssh; ssh's own failure is 255. With `-tt` the remote stderr shares the terminal, so the exit reason comes from asking herdr afterwards (`diagnose_attach_exit`, through the machine's API) instead of the stderr file; 255 means "machine unreachable" and the tile waits for its machine to reconnect. Observe (read-only view) uses `ssh -T`.
- **Model.** `Workspace.machine` and `HerdrRef.machine` (`None` = Local, else herdr's machine ID; serialised only when set, so old state files load unchanged). herdr IDs (`w1:p1`, `term_…`) are only unique per server, so every match between a tile and herdr — reconcile, adoption, agent status, closing a herdr workspace — compares the machine first.
- **Adoption per machine** (ADR 0008 applies to each server): a survey of machine M only adopts, prunes and matches M's workspaces and tiles; new workspaces get `machine = M`. Only Local's survey may drop the first-launch placeholder.
- **Connection state** per machine: `connecting` (until the first event stream is up), `connected`, `reconnecting` (stream dropped; the adapter retries with backoff up to 30 s), `off` (`connect = false` or disabled in herdr), `error` (needs the user). It's the event stream's state, so one machine dropping never touches another: each machine has its own worker, event stream and agent subscription on the core's herdr runtime. Shells get `machine.status {machine}` events and `machines.list`; `herdr.status` stays for Local.
- **Creating**: `workspace.create {machine}`; the sidebar's "+" on a group passes the group's machine. Terminals in a workspace go to its machine.
- **herdr doesn't start remote servers through forwarding, and neither do we.** A remote machine without a running server shows `error: herdr isn't running on <label>`; the user starts it there (or attaches once with the herdr TUI, which does).

## Consequences

- **What the user sees:** with machines saved in herdr (`build-host`, `dev-server`), Azulejo shows Local plus one group per enabled machine. Nothing new to configure.
- **Latency:** a request to a remote machine costs one SSH channel open plus a round trip plus starting the remote `herdr remote-api-bridge` (~10–30 ms on a LAN with the master up). Terminal I/O doesn't go through the API; it's the attach stream over the same SSH connection.
- **Version skew:** the remote herdr needs `remote-api-bridge` v1 and protocol ≥ 22 (the adapter's version check). An older remote shows `error: herdr on <label> is too old`. `remote-api-bridge` came with `herdr --machine` in 0.9.1; when no herdr on the machine has it, the machine needs attention and its tooltip lists what the lookup found, e.g. `herdr on this machine is too old for Azulejo: ~/.local/bin/herdr 0.8.2, /usr/bin/herdr 0.9.0; needs herdr 0.9.1+ (remote-api-bridge)`.
- **Tile moves across machines:** a terminal tile keeps its own `HerdrRef.machine`, so moving it into a Local workspace keeps it attached to its remote pane. New terminals follow the workspace.
- **Not verified live:** the SSH path against a real remote. Tests run the same bridge and attach commands through a stand-in `ssh` that executes the remote script locally against throwaway herdr sessions; the real-SSH checks are in PLAN's deferred list.
- **Rejected:** an Azulejo-owned `[[machines]]` list with hosts (duplicates herdr; user said no); `herdr --machine X <cli>` per call (no event stream, one process per request); OpenSSH Unix-socket forwarding (`-L local.sock:remote.sock`) — works with any herdr but needs the remote socket path first and bypasses herdr's own bridge and its capability check.

## Amendment (Nov 4, 2026): `connected` means the bound panes are checked

The e2e pass measured a herdr restart taking 1.7–1.8 s to reattach. The core said `connected` the moment the event stream came back, so shells reattached to terminals the restart had replaced, failed, and switched only when the rebinds landed (docs/measurements.md, "herdr reconnect after the e2e pass").

- A machine reads `connected` once the core has reached it **and** re-checked every bound tile's pane (one `pane.list`), on add and after every connect or resync. The check runs beside the worker, so a slow survey doesn't hold it up. Until then it reads `connecting` or `reconnecting`.
- `machine.status` and Local's `herdr.status` go out only when the state changes.
- Terminal tiles hold their attach until their machine is `connected` (2 s at most).
- Local starts `connected` when its herdr was already running at launch: its panes are most likely the saved ones, and a normal relaunch shouldn't wait. A herdr the core had to start reads `connecting` until the check is in.
- The local event stream retries at least every 250 ms while herdr is down; remote machines keep their 1–30 s back-off.
