# Architecture decision records

One file per decision: `NNNN-short-title.md`, numbered in order. Each record says what we decided, why, and what we gave up. Gate G0 adds the first ones: terminal bridge, herdr mapping and take-over behaviour.

| ADR | Decision |
| --- | --- |
| [0001](0001-terminal-bridge.md) | Terminal tiles run `herdr terminal attach` as their child process |
| [0002](0002-herdr-mapping.md) | ~~One herdr tab per terminal tile~~ (superseded by 0012); herdr workspaces 1:1 |
| [0003](0003-take-over.md) | Take-over: ask first, fall back to read-only, never retake automatically |
| [0004](0004-animation-resize-timing.md) | Resize terminals once, at the start of a layout animation |
| [0005](0005-scrollback-and-passthrough.md) | Scrollback, search, title and attention come from herdr |
| [0006](0006-herdr-process-hygiene.md) | Clean herdr environment; size the PTY before attach |
| [0007](0007-dedicated-herdr-session.md) | ~~Terminal tiles live in a dedicated herdr session, `azulejo`~~ (superseded by 0008) |
| [0008](0008-use-the-users-herdr-session.md) | Use the user's own herdr session and adopt what already runs there |
| [0009](0009-changes-made-in-herdr.md) | Exited panes end their tile, panes closed in herdr remove it; split-off panes and agent status follow herdr |
| [0010](0010-dynamic-layouts.md) | Bismuth-style dynamic layouts per tab (Tile, Monocle, Three Column); new tabs default to Tile; manual split tree stays a layout kind |
| [0011](0011-multiple-herdr-machines.md) | Several herdr machines at once, taken from herdr's saved machines; API over herdr's `remote-api-bridge`, terminals over `ssh -tt … terminal attach` |
| [0012](0012-mirror-layout-into-herdr.md) | Mirror each tab's layout into herdr, both ways: one herdr tab per Azulejo tab, its splits, zoom and Monocle; migration moves panes, never kills; only Azulejo ops change herdr's structure |
