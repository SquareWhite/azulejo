# Dogfooding Azulejo (Gate G1)

One week of real use on macOS with the frozen build `v0.1.0-dogfood.1`, before M2 starts touching the app. Log anything odd in [dogfood-log.md](dogfood-log.md).

## Install

1. The first build is in `dist/` at the repo root: `dist/Azulejo-0.1.0-dogfood.1.dmg` (and a `.zip`). Or skip the `.dmg` entirely and run `just dogfood` (see [Updating](#updating)).
2. Open the `.dmg` and drag Azulejo to Applications.
3. The build is ad-hoc signed, not notarised, so Gatekeeper blocks the first launch. Either right-click Azulejo in Applications → **Open** → **Open**, or run once:

   ```sh
   xattr -dr com.apple.quarantine /Applications/Azulejo.app
   ```

4. In Azulejo's app menu, choose **Install Command-Line Tool…** It asks first, then links `azj` into `~/.local/bin` (or `/usr/local/bin`). Open a new terminal and try `azj help`.
5. Check the keyboard tools below.

### Keyboard tools

External keyboard remappers can intercept shortcuts before Azulejo receives them. If you use Hammerspoon or Karabiner, scope conflicting rules to exclude Azulejo's bundle ID, `dev.azulejo.Azulejo`.

In particular, a global window-movement rule can intercept `alt+shift+hjkl`, and terminal-only remaps can break `cmd+l` in browser tiles. Adjust your own rules or override the app's shortcuts under `[keys.macos]` ([keybindings](keybindings.md)).

## Updating

After the first install, update with one command from the repo:

```sh
just dogfood              # build HEAD, quit Azulejo, swap it into /Applications, relaunch
just dogfood --rollback   # back to the build before (again to go further back)
just dogfood --list       # kept builds; * marks the installed one
just dogfood --to <hash>  # reinstall a kept build by commit
```

- It builds the last commit in a separate worktree (`~/Library/Caches/Azulejo/dogfood-src`), so uncommitted changes in your checkout, including agents' work in progress, are left out. `--dirty` builds the checkout as it is; `--check` runs `just build test lint` first.
- Quitting saves the layout, and herdr keeps every terminal and agent running, so an update costs a second or two of reattaching. If Azulejo doesn't quit within 10 s the script stops and changes nothing (`--force-kill` overrides).
- The last three builds, plus the one installed, stay in `~/Library/Application Support/Azulejo/dogfood-builds/`. The `.dmg` build you started with is kept there too, so `--rollback` can get back to it.
- Each build records its commit (`AzulejoCommit` in `Info.plist`). Mention it in bug reports: `just dogfood --list` shows it.
- `azj` links into the bundle, so it updates with the app. Builds made on this Mac carry no quarantine flag, so there's no Gatekeeper prompt after the first install.

## What happens on first launch

Azulejo connects to your **default herdr session**, the one plain `herdr` uses ([ADR 0008](adr/0008-use-the-users-herdr-session.md)).

- **Adoption.** Every herdr workspace shows up as an Azulejo workspace, every herdr tab as a tab with the same splits, every pane as a terminal tile. Azulejo only reads herdr here; it moves, closes and re-lays-out nothing.
- **New tiles.** A terminal tile you create gets its own herdr tab, so you'll see new tabs appear in herdr too.
- **Closing a tile detaches.** The pane keeps running in herdr. Killing a pane is a separate, explicit action. A closed adopted tile stays closed on relaunch.
- **Pane size.** While a tile shows a pane, Azulejo sets that pane's size. If the herdr TUI (or your phone) shows the same pane, it'll look cropped or padded there until you close the tile.
- **Read-only tiles.** If another client already holds a pane, the tile shows it read-only with a **Take over** button. Azulejo never grabs it back by itself.
- **Environment.** Panes inherit the environment of whoever started the herdr server. If you started it from inside an agent, `CLAUDE_*` variables can show up in panes; restart herdr from a plain terminal (`herdr update --handoff` keeps processes alive).

## Everyday use

- Your herdr chords carry over: `alt+hjkl` focus, `alt+shift+hjkl` swap, `ctrl+alt+shift+hjkl` resize, `ctrl+cmd+t` tall split, `alt+enter` promote, `alt+o` rotate, `alt+shift+e` equalize, `alt+shift+m` zoom, `alt+w` close tile, `cmd+t` / `cmd+1..8` workspaces, `cmd+shift+t` / `ctrl+1..8` tabs. Full list and overrides: [keybindings](keybindings.md).
- `cmd+k` opens quick open: one menu for every action (with its chord) and every workspace, tab and tile (by name, cwd, URL, path or agent), fuzzy search, recent first. Start with `>` for commands only or `@` for places only. Commands that need input (rename, open URL, preview a file, open or save a layout) ask for it inline. `cmd+shift+p` and `cmd+p` are unbound now; [docs/keybindings.md](keybindings.md#quick-open-k) shows how to bring them back. In terminals `cmd+k` no longer clears (Ghostty's default): use `ctrl+l`.
- Agents show in their tile's corner (`claude · working`). Blocked is red, finished-but-unseen green; the sidebar, tab bar and Dock badge count them. Focusing the tile marks it seen. Turn the Dock badge off with `defaults write dev.azulejo.Azulejo AzulejoDockBadge -bool false`.
- Browser and preview tiles also come from `azj`:

  ```sh
  azj tile split --right --kind browser --url http://localhost:3000
  azj tile split --down --kind preview --path plan.md
  ```

- Overrides go in `~/.config/azulejo/config.toml` under `[keys]`. Saving the file applies it; problems show in a banner over the tiles (the previous config stays until the file is fixed). `reload_config` in the palette reads it again by hand.

## Where things live

| What | Where |
| --- | --- |
| Session state | `~/Library/Application Support/Azulejo/state.json` |
| Config | `~/.config/azulejo/config.toml` |
| API socket | `$TMPDIR/azulejo/azulejo.sock` |
| Crash reports | `~/Library/Logs/DiagnosticReports/Azulejo-*.ips` |
| Logs | Off by default. For a session with logs: `AZULEJO_VERBOSE=1 /Applications/Azulejo.app/Contents/MacOS/Azulejo 2>>~/Library/Logs/Azulejo.log &` |

## Reporting issues

Add an entry to [dogfood-log.md](dogfood-log.md) (the template's at the top), or just tell the manager agent in chat. Attach a crash report or log lines if you have them. Severity 1 means lost work or a session you couldn't get back; G1 passes after a week with none open.

## Known gaps (coming in M2)

- No find in terminal (`cmd+f`) yet, though the chord is reserved.
- Bismuth-style dynamic layouts (2.12), several herdr machines in the sidebar (2.13), the git branch per workspace (2.14) and tile headers with Preview | Source (2.15) are in progress.
- No IME composition in terminal tiles yet; copy/paste, mouse selection and Claude Code's full key path are untested by hand.
- OSC 52 clipboard and terminal notifications don't pass through herdr ([ADR 0005](adr/0005-scrollback-and-passthrough.md)).
- Ad-hoc signed only: notarisation waits for a Developer ID ([release](release.md)).

## Checks to run when the Mac is free

These need real input or windows in front, so the agents left them for you. Each posts keystrokes or raises windows, so don't run them during a meeting.

```sh
# Frame pacing with the window visible (0.5)
cd spikes/macos-anim && ./run.sh --stress 50 --exit --front --resize-at start --reveal first --log build/visible.jsonl

# Standalone Ghostty latency baseline (0.4); posts real keys for ~5 minutes
scripts/latency/run-os-input.sh

# NFRs with the window in front (1.13), on the release build;
# compare memory with Ghostty.app showing 4 splits (the restated memory target)
scripts/nfr/latency.sh
scripts/nfr/memory.sh

# After `herdr update`: re-run the herdr probe (and once with the TUI check)
scripts/herdr-probe.sh
AZJ_PROBE_TUI=1 scripts/herdr-probe.sh
```

By hand: close the lid for a minute with 6 terminal tiles open and check they reattach within 2 s; open the Web Inspector from a browser tile; copy/paste and mouse selection in a terminal; type into Claude Code (arrow keys included) inside a tile; open an adopted pane while the herdr TUI shows it and note what the TUI draws. The full list is under "Deferred checks" in [PLAN.md](../PLAN.md).
