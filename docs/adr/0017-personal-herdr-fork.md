# 0017: A personal herdr fork fixes the trash lines; Azulejo stays fork-agnostic

Status: accepted · Nov 13, 2026 · herdr notes question 13, upstream ask 8

## Context

After a vertical resize, an Ink-style program (Claude Code among them) can show stale copies of its live area above the new one: the "trash lines". They live in herdr's own copy of the pane, not in Azulejo. When a pane grows, herdr's libghostty-vt pulls scrollback back into view, and a bottom-anchored redraw can't reach the rows that went there ([herdr notes, question 13](../herdr-notes.md#13-stale-rows-trash-lines-after-a-vertical-resize)). Upstream herdr can't turn the pull off: its vendored libghostty-vt predates Ghostty's `resize_pull_scrollback` switch, and there's no config key.

A herdr build with the switch backported, run with the pull off, fixed it: `resize.sh` 18/18 with no stale blocks, one clean block in the herdr-only repro, and no trash lines in Claude Code in an Azulejo dev instance (checked by hand by the user). The cost: a shell gets no history back on a grow, so its prompt stays mid-screen with blank rows under it.

## Decision

- **The user runs a personal herdr fork as their everyday local herdr.** It's upstream herdr plus a small patch stack:
  - the backport of Ghostty's `resize_pull_scrollback` switch into herdr's libghostty-vt;
  - a config key, `[terminal] resize_pull_scrollback`, off by default in the fork;
  - a version suffix, `+azj.N`, and an update guard, so herdr never installs upstream over the fork;
  - an update script that rebases the stack onto the newest upstream tag, then builds, tests and installs it.
- **Azulejo stays fork-agnostic.** No dependency on the fork, no fork-only protocol: Azulejo talks to it as to any herdr. Remote machines keep running stock herdr, and the trash lines remain there.
- **Azulejo doesn't bundle herdr.** The fix is server-side, and the local server is shared with the herdr TUI and the phone ([ADR 0008](0008-use-the-users-herdr-session.md)). A herdr inside the app would either fight the user's server or leave it unfixed.
- **`AZJ_HERDR_BIN` and `AZJ_HERDR_CONFIG` stay dev tools** for herdr experiments on a dev instance ([apps/macos/README.md](../apps/macos/README.md#a-custom-herdr-build)). The installed app ignores them.
- **Upstream gets a proposal, not a PR.** herdr doesn't take unsolicited PRs, so it goes in as a GitHub Discussion: the same config key, with upstream's default kept on (today's behaviour). Not posted yet.

## Consequences

- Locally, Claude Code and other Ink-style programs redraw cleanly after divider drags and window resizes. Shells pay with blank rows under the prompt after a grow; scrolling up still shows the history.
- Stale copies still pile up in scrollback, pull or no pull.
- Bug reports and tests have to say which herdr ran. Azulejo's app log names it at connect: `herdr: <bin> (config <file>)`. `just test-herdr` and the e2e suites use whatever `herdr` is on `PATH`.
- The fork lags upstream until the update script runs. A rebase that conflicts blocks the update, not Azulejo.
- If upstream ships the switch, the fork drops the backport and keeps only the config default, or goes away.
- **Rejected:** bundling a patched herdr in the app (above); fixing it on Azulejo's side (the rows are in herdr's copy, and Azulejo's surface already matches it row for row); making the fork a requirement (remotes and other users run stock herdr).

## Evidence

- [herdr notes](../herdr-notes.md), question 13 (the probe, the pull-on/pull-off table, the outcome) and upstream ask 8
- `apps/macos/automation/e2e/resize.sh` (R1): 18/18 against the patched herdr with the pull off; doubled blocks after `d3` and `d4` on stock herdr
