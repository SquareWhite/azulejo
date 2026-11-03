# Layout files

A layout file (`.azulejo.toml`, usually at a project's root) describes tabs, splits and tiles. One command opens it:

```sh
azj layout apply            # ./.azulejo.toml
azj layout apply ~/src/shop # a folder means its .azulejo.toml
azj layout apply ops.toml --mode new-tab
```

`azj layout export > .azulejo.toml` writes the focused workspace back out, and `--path FILE` writes the file for you, with paths made relative to it. Add `--tab ID` to export a single tab.

The example in [examples/azulejo.toml](examples/azulejo.toml) is the PRD's "scripted layouts" use case: an editor, a test watcher, a dev server and a browser on it.

## The format

```toml
name = "shop"        # workspace name (new_workspace mode); defaults to the folder's name
cwd = "app"          # terminals' default working directory, relative to the file
layout = "tall"      # how the `tile` entries are arranged

[[tile]]
command = "nvim ."
focus = true

[[tile]]
command = ["cargo", "watch", "-x", "test"]

[[tile]]
url = "http://localhost:3000"
```

### Tabs

The top-level `layout`, `tile` and `root` keys describe one tab. For several, use `[[tab]]` sections, each with the same keys plus a `name`:

```toml
[[tab]]
name = "dev"
layout = "tall"
tile = [{ command = "nvim ." }, { command = "npm run dev" }]

[[tab]]
name = "ops"
layout = "rows"
sizes = [2, 1]
tile = [{ command = "htop" }, { command = "tail -f log/app.log" }]
```

A file uses either top-level tab keys or `[[tab]]` sections, not both.

### Arrangements

`layout` arranges a tab's `tile` entries:

| `layout` | Result | Options |
| --- | --- | --- |
| `tall` (default) | The first tile is master on the left; the others stack evenly on the right. Same shape as the tall actions (`split_auto`, `promote`, `rotate`). | `ratio`: the master's share, 0.05–0.95, default 0.5 |
| `columns` | Side by side | `sizes`: one relative size per tile, default even |
| `rows` | Stacked | `sizes` |

For anything else, give an explicit tree as `root`. A split has `split` (`horizontal` puts children side by side, `vertical` stacks them), `children` (two or more) and optional `sizes`; anything else is a tile:

```toml
[root]
split = "horizontal"
sizes = [60, 40]

[[root.children]]
command = "nvim ."

[[root.children]]
split = "vertical"
children = [{ command = "cargo watch -x test" }, { url = "http://localhost:3000" }]
```

Sizes are relative: `[60, 40]`, `[3, 2]` and `[0.6, 0.4]` mean the same. Every tile needs at least 5% of the space left at its split.

### Tiles

| Key | Tiles | Meaning |
| --- | --- | --- |
| `kind` | all | `terminal`, `browser`, `preview` or `files`. Usually left out: `url` means browser, `path` preview, `root` files, anything else terminal. |
| `title` | all | The tile's name (and the herdr pane's label). |
| `focus` | all | `true` on at most one tile per tab: it gets focus. Otherwise the first tile does. |
| `command` | terminal | Typed into the shell once it starts. A string is sent as a shell line (`"npm run dev"`, pipes and all); an array is an argv, quoted for the shell. |
| `cwd` | terminal | Working directory, relative to the file. Defaults to the file's `cwd`, else the file's folder. |
| `env` | terminal | Extra environment, e.g. `env = { RUST_LOG = "debug" }`. `AZULEJO_*` names are reserved. |
| `on_exit` | terminal | `"close"` or `"keep"`: what happens when the tile's shell exits. Overrides `[terminal] on_exit` in the config (default `"close"`). |
| `url` | browser | The page to load. |
| `path` | preview | The file to show, relative to the layout file. |
| `root` | files | The folder to browse, relative to the layout file. |

Paths may start with `~/`.

### How commands run

herdr can't start a pane with something other than the user's shell, so Azulejo types `command` into the new pane's shell, followed by Enter. The shell stays when the command exits, so a dev server that crashes leaves a prompt to restart it from. Write commands your shell understands: fish users can't use `$((…))`, for example.

When the shell itself exits (ctrl+d, `exit`, or a command written as `npm run dev; exit`), the tile closes, like any terminal window. Set `on_exit = "keep"` on a tile to keep it instead, ended, with a Respawn button; respawning runs its command again.

## Modes

`layout.apply` takes a `mode`:

| Mode | What happens |
| --- | --- |
| `new_workspace` (default) | A new workspace in the focused window, named by the file's `name`. |
| `new_tab` | The file's tabs are added to the focused workspace (or `workspace`). |
| `replace_tab` | The file's tabs replace the workspace's active tab. Its terminals detach: their herdr panes keep running, and closing never kills them. |

Applying is all or nothing: a file with an error changes nothing.

## Errors

Mistakes come back as `invalid_params` errors that say where:

```
not a valid layout file: TOML parse error at line 3, column 1 … unknown field `commnd`, expected one of `split`, `sizes`, …
tab 2 (`ops`): tile 1: `url` only applies to browser tiles
tab 1: root.children[2]: a split needs at least two children
```

## API

- `layout.apply {path | toml, base_dir?, workspace?, mode?}` replies `created` with the workspace, first tab and its focused tile. `path` must be absolute (the CLI makes it so); a folder means its `.azulejo.toml`. With `toml`, relative paths resolve against `base_dir`.
- `layout.export {tab?, workspace?, path?}` replies `layout_file {toml, path?}`, writing the file when `path` is given.

The engine module is `core/crates/engine/src/layout_file.rs`; its tests cover parsing, diagnostics, every mode, and export → apply round trips.
