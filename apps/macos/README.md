# Azulejo for macOS

A Swift 6 and AppKit shell over the Rust core (`core/crates/ffi`). Terminals
are libghostty surfaces attached to herdr panes.

More detail lives in [`docs/`](docs/):

- [architecture.md](docs/architecture.md): source layout, terminal tiles, key routing, writing a tile kind
- [interface.md](docs/interface.md): tile headers, toolbar and tabs, sidebar, menus, the mouse
- [motion.md](docs/motion.md): layout motion, workspace slides, the focus dim
- [smart-select.md](docs/smart-select.md): ⌘S, labelling and acting on visible terminal items

## Building

```sh
just mac        # target/macos/Azulejo.app
just ghostty    # rebuild GhosttyKit from vendor/ghostty (after bumping the submodule)
```

`just mac` runs `apps/macos/bundle.sh`. With full Xcode it uses XcodeGen and
`xcodebuild`; with only the Command Line Tools it uses SwiftPM and assembles
the bundle by hand. It builds GhosttyKit first when it's missing.

In a fresh Git worktree `vendor/ghostty` is empty. Link it to the main
checkout's copy before building, and never commit the link:

```sh
rmdir vendor/ghostty
ln -s "$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")/vendor/ghostty" vendor/ghostty
```

Release builds, signing and notarisation: [docs/release.md](../../docs/release.md).

## Dev profile: use this for all GUI testing

`just mac-run` launches the real thing: your config, your saved session and
your own herdr session. Don't test with it. The dev profile keeps everything
under `.dev/` in the repo and runs its own herdr session:

| | Dev profile |
| --- | --- |
| Config | `.dev/config/azulejo/config.toml` (generated, `[herdr] session = "azj-dev"`) |
| State | `.dev/state/state.json` |
| API socket | `.dev/run/azulejo.sock` |
| herdr session | `azj-dev` (`AZJ_DEV_SESSION` overrides it; `default` is refused) |
| Ghostty dim settings | `.dev/config/ghostty/config`, if it exists |
| Log | `.dev/logs/app.log` |

```sh
just mac-dev --bg       # build, then launch without activating or raising a window
just mac-dev            # the same, but the window comes to the front
just azj-dev tile list  # azj against the dev app's socket
apps/macos/dev.sh --stop
just dev-reset          # quit, stop and delete the herdr session, wipe state, run, out and logs
```

`apps/macos/dev.sh --help` lists every flag. Always use `--bg` (or
`--automation`, which implies it) when someone else is using the Mac.

### Several instances

`AZJ_DEV_NAME=<name>` moves the profile to `.dev/<name>/` and the herdr
session to `azj-dev-<name>`. Every recipe above honours it:

```sh
AZJ_DEV_NAME=term just mac-dev --bg
AZJ_DEV_NAME=term just azj-dev tile list
AZJ_DEV_NAME=term just dev-reset
```

Names use letters, digits, `-` and `_`, and can't be `config`, `state`,
`run`, `logs` or `out`. macOS caps Unix socket paths at 103 bytes, and
`dev.sh` refuses a longer `<checkout>/.dev/<name>/run/azulejo.sock`. In a
deep worktree path that leaves only a few characters for the name; keep it
short.

`--reset` leaves `.dev/<name>/config/` alone, so a hand-edited dev config
survives. Delete it to start over.

### Guards

- `dev.sh` refuses `AZJ_DEV_SESSION=default` and a dev config that points at
  herdr's `default` session.
- The app, launched with `AZULEJO_PROFILE=dev`, refuses to boot unless its
  core options name a herdr session other than `default`.
- `dev.sh` strips `HERDR_SOCKET_PATH`, `HERDR_PANE_ID`, `HERDR_TAB_ID`,
  `HERDR_WORKSPACE_ID`, `HERDR_ENV` and `HERDR_CONFIG_PATH` before calling
  herdr, since your shell may sit inside your own herdr session.
- Dev profiles use a named herdr session, so herdr's saved machines are off:
  a dev run never reaches a real machine. See
  [architecture.md](docs/architecture.md#herdr-machines) for testing machine
  groups against a stand-in.

### What dev.sh passes to the app

`open` doesn't pass the environment on, so `dev.sh` hands these over with
`open --env`:

| Variable | Does |
| --- | --- |
| `AZULEJO_CORE_OPTIONS` | JSON for `azj_core_new_with_options`: paths, herdr session, `api_required` |
| `AZULEJO_PROFILE=dev` | The session guard above; also turns on logging to the app log |
| `AZULEJO_PIDFILE` | Lets `--stop` send SIGTERM, so the app quits cleanly (state flushed, socket removed) |
| `AZULEJO_BACKGROUND=1` | `--bg`: never `NSApp.activate()`; windows are ordered to the back |
| `AZULEJO_AUTOMATION`, `AZULEJO_AUTOMATION_OUT` | `--automation`: run a script, then quit |
| `AZJ_DEBUG_MIRROR` | When set in `dev.sh`'s environment: log every herdr mirror push and pull |
| `AZJ_FILES_LIST_DELAY_MS` | When set: hold every folder listing back that long, standing in for a slow folder |
| `AZJ_REFLOW_MS`, `AZJ_REFLOW_LEAD_MS` | When set: reflow motion tuning (see [motion.md](docs/motion.md)) |
| `AZJ_HERDR_BIN`, `AZJ_HERDR_CONFIG` | A custom herdr build and its config (below) |

### A custom herdr build

To try a herdr branch inside Azulejo, point a dev instance at its binary with
`AZJ_HERDR_BIN`. The core then runs that file for everything local: the
server it starts, its herdr CLI calls, and every terminal tile's
`terminal attach` and `observe`. `dev.sh --reset` stops the session with it
too. Remote machines keep their own lookup.

`AZJ_HERDR_CONFIG=<file>` gives the instance's herdr a `config.toml` of its
own, through herdr's `HERDR_CONFIG_PATH`. Without it, `dev.sh` uses
`.dev/<name>/config/herdr/config.toml` when that exists. It refuses a file in
your own herdr config folder.

```sh
# A herdr checkout next to this one, built with `cargo build --release`.
fork="$(dirname "$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")")/herdr/target/release/herdr"
mkdir -p .dev/fork/config/herdr
printf '[terminal]\nresize_pull_scrollback = false\n' > .dev/fork/config/herdr/config.toml
AZJ_DEV_NAME=fork AZJ_HERDR_BIN="$fork" just mac-dev --bg
```

herdr reads most of its config only when the server starts, and the server
outlives the app. To change the config or the binary, restart the session:
`AZJ_DEV_NAME=fork just dev-reset`, edit, launch again. `dev.sh` warns when
the session is already running, and the app log names the herdr binary and
config it picked (`herdr: …`).

The core reads these variables on the dev profile only, so the installed app
ignores them. A path that is relative, missing, not a file or not executable
stops `dev.sh` before launch; the core logs it and runs without herdr rather
than falling back to the herdr on `PATH`. herdr passes its environment on to
panes, so a `herdr` run inside the dev session reads the same config file.
`automation/e2e/hd` honours both variables when they're exported.

## Driving the app

Two ways in, neither of which posts OS input or raises a window:

- **`azj`** against the dev socket: `just azj-dev <args>`. Good for anything
  the API covers (tiles, layouts, terminal text).
- **Automation scripts** (`.azs`) run inside the app. Use them for keys,
  focus, mouse, menus and captures.

### Automation scripts

```sh
AZJ_DEV_NAME=me apps/macos/dev.sh --automation path/to/script.azs --out .dev/me/out/run
```

`--automation` implies `--bg` and `--wait`. The app runs the script as an
accessory app, writes `<out>/automation.log` (every step, with the layout
after it) and `<out>/result` (`ok` or `fail: …`), then quits. `dev.sh` exits
non-zero when the result isn't `ok`.

A `key` line becomes an `NSEvent` posted to the app's own queue, so it goes
through the same local monitor and key router as a real keypress. Nothing
reaches other apps. A short example:

```text
# split, check, capture
key ctrl+cmd+t
until tiles 2 3000
log split done
shot after-split
```

The full command list (`key`, `type`, `action`, `expect`, `until`, `mouse`,
`menu`, `anim`, `core`, `tile`, `sh`, `palette` and more) is the comment at
the top of [`Sources/Azulejo/Automation.swift`](Sources/Azulejo/Automation.swift).
Feature-specific verbs and `expect` keys are named in the docs for each
feature. Scripts ending in `.azs.in` are templates; their `.sh` driver fills
in fixture paths and ports.

### Screenshots

- `shot NAME` renders the window's content to `<out>/NAME.png`. It needs no
  Screen Recording permission, but misses Metal content mid-motion.
- `capture NAME [ms]` drops a request in `<out>/.capture/`; the driving
  script (`anim.sh`, `toolbar.sh`, `e2e/wsslide.sh`…) takes it with
  `screencapture -l` on the app's own window, Metal and motion included. The
  app also renders its layer tree to `<out>/NAME-app.png`, which works with
  the screen locked.

Never capture the whole screen or another app's window. Captures stay under
`.dev/`; don't commit them.

## Tests

Before merging a change to the shell, run `just lint`, `just test`,
`just mac`, every e2e suite and `focus.sh`; add `anim.sh` for motion or
layout changes and `just test-herdr` for herdr or mirror changes.

Every GUI script runs on its own dev instance and resets it when done. Two
runs of one script at once fight over its herdr session, so give each a
distinct `AZJ_DEV_NAME`.

### e2e suites

`apps/macos/automation/e2e/` holds the end-to-end suites, one shell script
each, sharing `lib.sh`.

```sh
apps/macos/automation/e2e/run-all.sh              # every suite, one after another
apps/macos/automation/e2e/run-all.sh dyn mtabs    # some of them
AZJ_DEV_NAME=e2e-me-1 apps/macos/automation/e2e/dyn.sh
```

- Each suite defaults to its own name (`e2e-dyn`, `e2e-agents`…).
  `run-all.sh` with `AZJ_DEV_NAME` exported runs every suite under that one
  name.
- `lib.sh` refuses an `AZJ_DEV_NAME` that doesn't start with `e2e`, and
  `e2e/hd`, the suites' herdr wrapper, refuses any session not named
  `azj-dev-e2e*` and `herdr update`.
- Output goes to `.dev/e2e.out/<suite>/`, outside the instance folder that
  `--reset` wipes. `checks.txt` has one `PASS` or `FAIL` line per check;
  `check-output.txt`, each script's `automation.log` and the app log sit
  beside it. A re-run overwrites the last one.
- `E2E_KEEP=1` leaves the instance and fixtures behind for a look.
- `stale-check.sh` isn't in `run-all.sh`: it needs the screen unlocked and the
  display awake.

The `dyn` suite's "0 dropped frames" check (`anim expect-clean`) is known to
fail on a loaded machine.

### Focus suite

```sh
apps/macos/automation/focus/focus.sh [--keep]    # or: just mac-focus
```

39 scenarios: which action each chord resolves to and where first responder
ends up, across terminals, the sidebar's Files view, ⌘K, layouts, tabs,
workspaces, herdr changes, take-over and input methods. It runs on
instance `focus` and writes to `.dev/<name>/out/focus/`, which the reset at
the end wipes: pass `--keep` to read the log afterwards. Don't skip it; it
catches resize bugs nothing else does.

### Motion stress

```sh
apps/macos/automation/anim.sh                     # 50+ animations: frame pacing, PTY resizes per motion
apps/macos/automation/anim.sh --slow              # 1.5 s motions with mid-motion window captures
AZJ_ANIM_STYLE=crop apps/macos/automation/anim.sh # one style regardless of the config
AZJ_ANIM_THEME=atom-one-light apps/macos/automation/anim.sh --slow
```

It runs on instance `anim` and writes to `.dev/<name>-results/<mode>/`.
Frame-drop numbers on a busy machine are noisy: before calling something a
regression, alternate runs on `main` and on your branch.

### Feature scripts

Each runs in-process on the instance named, unless `AZJ_DEV_NAME` is set.

| Script | Instance | Checks |
| --- | --- | --- |
| `just mac-automation` | default | `keys.azs`: every tiling action from the keyboard |
| `just mac-layouts` | default | `layouts.azs`: the dynamic layouts |
| `just mac-bismuth` | default | `bismuth.azs`: every default layout chord, traced |
| `just mac-rebind` (`rebind.sh`) | `rebind` | Live rebinding of every keymap context |
| `files.sh` | `files` | The sidebar's Files view on a 50k-file repository |
| `e2e/fops.sh`, `e2e/rfops.sh` | `e2e-fops`, `e2e-rfops` | The Files view's file operations, Local and through the SSH stand-in |
| `files-slow.sh` | `fslow` | Slow folder listings never block the main thread |
| `headers.sh` | `hdr` | Tile headers and a branch switch |
| `chrome.sh` | `shell` | ⌘K, the layout picker, the config banner, attention badges |
| `toolbar.sh` | `toolbar` | Where clicks land under the title bar; window captures |
| `topbar.sh LABEL`, `rail.sh LABEL` | `topbar`, `railcap` | The toolbar row and the collapsed sidebar in all four themes, for before/after comparison |
| `sidebar.sh` | `side` | Agents and Files views with fixture repositories |
| `menus.sh` | `mnu` | Right-click menus, and that closed workspaces stay closed |
| `mouse.sh` | `e2e-mouse` | Terminal selection, ⌘-click links, mouse-aware programs |
| `hints.sh` | `e2e-hints` | ⌘S smart select: the core's checks, then the GUI in a dark and a light theme |
| `machines.sh` | `mm` | Machine groups against the SSH stand-in |
| `mirror.sh` | `mir` | Splits in the app show up in herdr, and back |
| `restore.sh` | `restore` | Session restore and wake |
| `themes.sh` | `thm` | The mockup scene through all four built-in themes |
| `mockup.sh` (`just mac-mockup`) | `mock` | The mockup scene with fixture data |
| `polish.sh` | `polish-g` | Captures of long subtitles, header status icons and many tabs |
| `reflow-lag.sh` | `lag` | How far terminal text trails its tile during reflow motion |

All live in `apps/macos/automation/`. Each script's header comment has its
options and output folder.

## Troubleshooting

- **`dev.sh: build first`.** Run `just mac`.
- **The app exits at launch.** `dev.sh` prints the end of the app log. A
  socket path just under the 103-byte cap can still fail: shorten the
  checkout path or the instance name.
- **`the dev app is already running`.** `apps/macos/dev.sh --stop` with the
  same `AZJ_DEV_NAME`. A crashed run can leave a pidfile; `dev.sh` clears it
  on the next launch.
- **herdr ignores a new binary or config.** The session's server still runs
  the old one: `just dev-reset`, then launch.
- **A named theme fails with `theme "…" not found`.** libghostty can't find
  its resources, which `bundle.sh` copies from `vendor/ghostty/zig-out/share`.
  Run `just ghostty`, then `just mac`.
- **Left-over dev sessions.** `AZJ_DEV_NAME=<name> apps/macos/dev.sh --reset`
  for each instance you started. Leave no dev herdr session running.
