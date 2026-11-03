# 0001: Terminal tiles run `herdr terminal attach` as their child process

Status: accepted · Nov 3, 2026 · Gate G0

## Context

herdr owns every terminal process (PRD). A terminal tile needs live output from a herdr pane, needs to send input back, and needs to resize the pane. herdr offers two ways in:

- `herdr terminal attach <terminal-id>`: an ordinary terminal program that streams herdr's re-rendered screen as ANSI and forwards keys, mouse and paste. Its PTY size becomes the pane size.
- `herdr terminal session control|observe <pane-id>`: NDJSON over stdio (`terminal.frame` records out; `terminal.input`, `terminal.resize`, `terminal.scroll`, `terminal.mouse`, `terminal.release` in).

Neither exposes raw PTY bytes.

## Decision

On both platforms the terminal surface (libghostty on macOS, VTE and later our libghostty-vt renderer on Linux) runs `herdr terminal attach <terminal-id>` as its child command. There's no custom bridge in M1. The flag order is `attach <id> --takeover`; the other order fails.

Controller mode stays the upgrade path. It has no detach key, gives explicit `terminal.closed` records instead of exit codes, and suits the Linux libghostty-vt renderer in M4. Switching needs a way to feed bytes into libghostty without a PTY.

## Consequences

- Almost no code: PTY resize becomes pane resize, and keys, mouse and bracketed paste work.
- The herdr hop costs +1.3 ms median keystroke → glyph (+1.6 ms at byte level), against an 8.3 ms budget.
- We inherit herdr's limits: no local scrollback ([0005](0005-scrollback-and-passthrough.md)), lost OSC 52, notifications, DECCKM and focus reporting, and a detach key (`ctrl+b q`) that can't be turned off. An exit 0 the app didn't ask for is treated as an accidental detach and reattached silently.
- Tile state comes from the attach client's exit code and stderr ([0003](0003-take-over.md)).
- The surface must give the PTY a non-zero size before spawning attach, and must never write EOF to it on detach ([0006](0006-herdr-process-hygiene.md)).

## Evidence

- [herdr notes](../herdr-notes.md), questions 1, 2 and 7, plus the exit table
- [measurements](../measurements.md), section 0.4
- `spikes/macos-terminal/` (libghostty) and `spikes/linux-terminal/` (VTE)
