# 0015: Remote files in the files panel, listed over the machine's SSH connection

Status: accepted · Nov 9, 2026 · M2.5 · PRD F10 × F17 · relates to [0011](0011-multiple-herdr-machines.md)

## Context

The sidebar's Files view (a `files.panel`, D.3) showed the focused workspace's files only when the workspace was Local. For a workspace on a remote herdr machine (ADR 0011) it said "Its files aren't shown here yet." The panel should list that machine's files for the focused terminal's working directory, browse like the local one (open folders, open files, follow `cd`), and never hang the UI on a slow or unreachable host.

What herdr offers: nothing for files. Its API (pane, tab, workspace, layout, events, process info) has no directory listing or file read, and `remote-api-bridge` only carries that API. What we already have per machine: an SSH target from herdr's catalogue and our own multiplexed SSH connection (`ControlMaster=auto`, one control socket per machine, ADR 0011) that the bridge and every terminal tile use. herdr already reports each remote pane's cwd (`pane_updated`, `pane.process_info`), and the core keeps it in the tile's `pane.cwd`.

The Mac shell calls the core synchronously on the main thread (`azj_core_execute`), so any wait for the network inside a `files.*` reply would freeze the window.

## Decision

- **Transport: one shell command per listing, over the machine's existing SSH connection.** `RemoteMachine::exec(script)` runs `ssh -T <ADR 0011 options> <target> -- <script>`, the same options and control socket as the bridge, so a listing costs one SSH channel, not a handshake. The script is POSIX `sh` wrapped in `exec /bin/sh -c '…'` (the login shell may be zsh or fish), uses only builtins per entry (globs, `test`, `printf`) and prints NUL-separated records, so any file name survives. One run can list several folders (polls do), capped at 20,000 entries per folder.
- **Async, cancellable, cached.** The core runs each listing on its own thread with a 15 s timeout (`ConnectTimeout` is 10 s) and kills it when cancelled. Listings go into a cache keyed by machine and folder. `files.list`/`files.expand` on a remote panel answer at once: from the cache (listed again in the background when older than 2 s), or empty with `loading: true`. When the listing is in, `files.changed` names the folder and the shell lists it again, the same flow as local file watching. Moving the panel to another root (the shell `cd`'d) or closing it cancels its running listings; their results are dropped.
- **No watching: polls.** A remote panel lists its open folders again every 3 s while no listing runs, in one SSH run. `files.changed` goes out only for folders whose listing or error changed.
- **Errors are states, not failures.** A failed run (ssh exit 255, timeout, a gone folder) sets `error` on the folder's listing and keeps the last entries. After a run fails as a whole, polls wait 10 s. A machine that's off or gone from herdr's list has no SSH command: its folders say "the machine isn't connected". When the daemon re-adds a machine, its panels list again at once.
- **Paths are worked out by name.** Remote paths can't be canonicalised here: `.` and `..` are resolved lexically, and `~` isn't expanded (it would be this Mac's home). A remote root must be absolute.
- **Protocol.** `files.panel` takes `machine` (herdr's machine ID; absent or `local` for Local). `FilesListing` gains `machine`, `loading` and `error`. Nothing else changes for local tiles or panels.
- **Actions on remote entries:**
  - *Open folder*: as local.
  - *Open in Preview* (Return on a file): the core copies the file (up to 8 MiB) over the same connection into a private folder under the temp dir (`azulejo-remote/<machine>/<remote path>`, 0700) and shows the copy in the tab's preview tile. The copy doesn't follow later edits: open it again for a fresh one.
  - *Open Terminal Here*: a new terminal beside the focused tile, in that folder. In a remote workspace new terminals run on its machine, so the shell starts there.
  - *Open in Default App*, *Reveal in Finder* and Quick Look need a local file: hidden from the context menu and inert for remote panels. ⌘↓ (open in default app) opens the preview copy instead.
  - *Copy Path* copies the remote path.
  - No git badges and no gitignore dimming for remote folders.
- **Which folder.** The sidebar shows a remote workspace's focused terminal's current cwd (`pane.cwd`), else another terminal's in the tab, else the workspace's cwd, with the machine's label in the title. While the machine reconnects the panel stays, showing the last listing and the error. When the machine is off or gone, a note says so.

## Consequences

- Works with any remote that has `/bin/sh`, `cd`, `printf` and `head`: no herdr version requirement, nothing to install.
- Cost while the Files view is open on a remote workspace: one SSH channel every 3 s (more while folders are opened), each a few milliseconds of remote shell work with the control master up.
- Changes on the remote show within about 3 s, not live.
- `run` kills the local `ssh` process on cancel. With the control master that closes the channel. A remote command that ignores it finishes on its own, but it's only `sh` builtins.
- **Tested** against the ssh stand-in only (engine tests `remote_files::*` and `files::tests::a_remote_panel_*`, e2e `rfiles.sh`). A real SSH host joins PLAN's deferred checks with the rest of ADR 0011.
- **Rejected:**
  - herdr: it has no file API.
  - SFTP (`ssh -s sftp`): a binary protocol to implement or a dependency to add, and it needs the sftp subsystem enabled.
  - `ls -la` parsing: names with newlines or leading spaces break, and output differs between GNU and BSD.
  - Mounting with sshfs or macFUSE: needs a kernel extension and system changes.
  - Listing synchronously inside `files.list`: it blocks the Mac's main thread.
