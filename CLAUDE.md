# Working on Azulejo (agents)

Rules for coding agents (Claude Code and its sub-agents). Several agents may work at once, each in its own worktree, while the user uses the same computer. Breaking a rule below is worse than skipping a test. The test-side version of these rules is [docs/testing/e2e-plan.md, section 0](docs/testing/e2e-plan.md#0-ground-rules-non-negotiable).

## Never on the user's computer

- No system-wide input: no CGEvent posting, no `osascript` keystrokes or clicks, no `cliclick`. Drive the app through in-process automation (`.azs` scripts) or `azj`.
- Never activate or raise a window. Launch dev apps with `--bg` (or `--automation`); foreground changes can disrupt the user's work.
- Screenshots: in-app captures (`shot`), or `screencapture -l <id>` of our own dev windows only. Nothing else on screen.
- Don't change system appearance, restart the Dock or clear icon caches.
- Don't read or write the general clipboard (the focus suite uses a private pasteboard), and don't touch the keychain.
- Keep personal names, real hostnames, home paths and workstation details out of fixtures and documentation. Use fictional data. Never commit app or desktop captures, terminal dumps, logs, crash reports or local credentials.
- Don't run `just release-mac`, `just release-notarize` or anything that submits to Apple without asking.

## herdr

- Never touch the user's default herdr session in tests. Use dev profiles (`azj-dev[-<name>]`) and throwaway sessions. `dev.sh` refuses `default`; so does the app with `AZULEJO_PROFILE=dev`.
- Never run `herdr update`.
- Never run `herdr machine add/remove/enable/disable`.
- Never connect to the user's real machines or any host in `~/.ssh/config`. Remote code paths go through the SSH stand-in, `scripts/ssh-standin.sh`.
- When you shell out to herdr yourself, unset `HERDR_SOCKET_PATH`, `HERDR_PANE_ID`, `HERDR_TAB_ID`, `HERDR_WORKSPACE_ID` and `HERDR_ENV` first: your shell may sit inside the user's session (see `herdr_dev` in `apps/macos/dev.sh`).

## The user's installed app and config

- Hands off `/Applications/Azulejo.app`, `~/.config/azulejo`, `~/Library/Application Support/Azulejo` and the `dev.azulejo.Azulejo` defaults. The one exception: `just dogfood` when the user asks for it (below).
- `just mac-run` launches the installed app on the user's herdr session. Don't use it for testing.
- Before editing any user dotfiles, back the file up next to itself as `<file>.bak-azulejo-<YYYYmmdd-HHMMSS>`.

## Dev instances

Details: [apps/macos/README.md, "Dev profile"](apps/macos/README.md#dev-profile-use-this-for-all-gui-testing).

- `AZJ_DEV_NAME=<name>` moves the profile to `.dev/<name>/` and the herdr session to `azj-dev-<name>`. Names: letters, digits, `-` and `_`; not `config`, `state`, `run`, `logs` or `out`.
- Every `just` dev recipe honours it: `AZJ_DEV_NAME=me just mac-dev --bg`, `AZJ_DEV_NAME=me just azj-dev tile list`.
- e2e suites need a name starting with `e2e` (`e2e/lib.sh` refuses anything else; `e2e/hd` refuses any herdr session not named `azj-dev-e2e*`).
- macOS caps Unix socket paths at 103 bytes and `dev.sh` refuses longer ones. The socket is `<checkout>/.dev/<name>/run/azulejo.sock`, so in a `.claude/worktrees/agent-<id>/` worktree only about 12 characters are left for the name.
- Clean up when you're done: `AZJ_DEV_NAME=<name> apps/macos/dev.sh --reset` quits the app, stops and deletes its herdr session and wipes its state. Leave no dev herdr session running.

## Worktrees

- `vendor/ghostty` (a submodule) is empty in a new worktree. For builds, replace it with a symlink to the main checkout's copy, which holds the built GhosttyKit. Resolve the checkout from Git rather than hard-coding a home directory:
  `rmdir vendor/ghostty && ln -s "$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")/vendor/ghostty" vendor/ghostty`
- Never commit that symlink: check `git status` before every commit. Remove it (`rm vendor/ghostty`) before you remove the worktree.

## Running tests next to other agents

- `apps/macos/automation/e2e/run-all.sh` runs every suite under a fixed dev name (each suite's default, e.g. `e2e-dyn`; or, if you exported `AZJ_DEV_NAME`, that one name for all of them). Two agents running it at once fight over the same herdr sessions. When others are testing, loop over the suites yourself and give each a distinct name: `AZJ_DEV_NAME=e2e-<you>-<n> apps/macos/automation/e2e/<suite>.sh`.
- Suites write to `.dev/e2e.out/<suite>/` in your checkout, whatever the dev name, so a re-run overwrites the last one's output.
- The same goes for `focus.sh` (`focus`) and `anim.sh` (`anim`): set your own `AZJ_DEV_NAME`.

## Before merging into main

- [ ] `just lint` (fmt, clippy, design tokens, keys doc, theme colours, icon freshness)
- [ ] `just test`
- [ ] `just mac`
- [ ] Every e2e suite in `apps/macos/automation/e2e/` (see above for parallel runs)
- [ ] `apps/macos/automation/focus/focus.sh`: 52 scenarios. Don't skip it: skipping it once let a Monocle resize bug through.
- [ ] Animation or layout changes: `apps/macos/automation/anim.sh` too.
- [ ] herdr or mirror changes: `just test-herdr` too.
- [ ] Generated files are regenerated, never hand-merged: `just design-tokens`, `just schema`, `just keys-doc`, `just icon`.
- [ ] Merge with `git merge --no-ff`.

Frame-drop numbers on a loaded machine are noisy. Before calling something a regression, run the same check on main and on your branch, alternating runs. Known flake: the `dyn` suite's "0 dropped frames" check (`anim expect-clean`) fails under load.

## Dogfood builds

Only when the user asks. `just dogfood` builds HEAD in its own worktree, quits Azulejo, swaps the build into `/Applications` and relaunches it. `scripts/release/dogfood.sh` only relaunches when stdout is a terminal, so run it as:

```sh
script -q /dev/null just dogfood < /dev/null
```

More in [docs/dogfooding.md](docs/dogfooding.md#updating).

## Where things are

- [PLAN.md](PLAN.md): milestones, status, deferred checks, standing rules
- [docs/PRD.md](docs/PRD.md): requirements (F-numbers)
- [docs/adr/](docs/adr/): decisions
- [docs/herdr-notes.md](docs/herdr-notes.md): what herdr does and doesn't do
- [docs/testing/e2e-plan.md](docs/testing/e2e-plan.md): the end-to-end plan and its reports
- [docs/keybindings.md](docs/keybindings.md): generated from `shared/config/keymap.default.toml` (`just keys-doc`)
- [docs/themes.md](docs/themes.md): themes, terminal colours, `[appearance]`
- [docs/dogfooding.md](docs/dogfooding.md): daily build and setup
- [docs/design/icon/](docs/design/icon/README.md): the app icon (`just icon`)
- [apps/macos/README.md](apps/macos/README.md): the macOS shell, dev profile, automation
