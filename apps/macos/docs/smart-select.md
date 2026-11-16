# Smart select (⌘S)

In a terminal, **⌘S** (`select_visible_item`) labels the selectable items in
every visible terminal pane of the window: URLs, paths, commands, IP
addresses, domains, hashes and IDs, and branches. Every non-blank line is
an item too, labelled once you type `l`. Type a label, or move with the
arrows, to select one; then copy, open or run it. Panes hidden by zoom or
Monocle are left out.

Rebind or disable it under `[keys.macos.terminal]`. Other tile kinds keep ⌘S
for themselves.

## Where the code lives

Everything that reads the screen is in the Rust core, `core/crates/hints`, so
every shell gets the same items: detection in `src/detect`; labels, lines,
paragraphs, grow/shrink, arrows and range text in `src/select.rs`; callout
placement in `src/callout.rs`; Run's target in `src/run.rs`; the row
classifier in `src/model.rs` and `src/refine.rs`.

On the Swift side, `TerminalHintOverlay.swift` reads the rows and a styled
snapshot from Ghostty, makes one `azj_hints_detect` call per pane (JSON in and
out, UTF-16 offsets), keeps the returned pane handle for the selection keys
(`azj_hints_pane_*`) and draws. `TerminalHints.swift` is the glue and
`TerminalRun.swift` carries out Run. The styled snapshot comes from a patch
that `just ghostty` applies to libghostty; rebuild GhosttyKit after changing
it.

Path checks go through the crate's `Filesystem` trait (a fake in tests).
`/home` and `/net` are never checked, and each pass checks a path once.

## Labels

A label's first letter is the item's kind: `u` URL, `p` path, `c` command,
`i` IP address, `d` domain, `h` hash or ID (git hashes, UUIDs, other hex
IDs), `b` branch and `l` line. When only one item of a kind is visible, the
letter alone selects it; otherwise home-row letters follow (`pa`, `ps`…).
Labels of one kind share a length, so none is a prefix of another.

**Lines.** Every non-blank line in each labelled pane is an item: one row,
or one line the terminal soft-wrapped over several rows, without its indent
and trailing blanks. Boxing every row up front would bury everything else,
so lines get no box or callout at first; `l` still shows in the legend.
Typing `l` fades the other kinds and labels every line; the next letters
filter as for any kind, and the arrows walk the lines. A selected line
copies its text (a range of lines copies as a terminal selection would),
and ⌥↑ grows it to its paragraph, block and screen. Lines are copy-only.

Paragraphs have no letter of their own: grow a line (⌥↑) to reach one.

When a pane shows the same item several times (a prompt's `~/proj` on every
prompt line), only the newest copy gets a label. The others light up while it
is selected, and the arrows still reach them. Lines are the exception: two
lines that read the same are two places, and each gets its own label.

Labels are yellow callouts with a short tail pointing at the item's first
cell. `callout::place` puts each one below, above or before its item,
preferring blank space, then plain output, then paragraphs, and never over
another callout or another item's first cell. With nowhere free it counts as
a clash (`hints.callout.flagged`). Lines' callouts are placed in a pass of
their own, since they only show once the rest have faded; on a row that
starts at the pane's edge one may sit over the line's first letters.

## Choosing

- **Typing filters.** Items whose labels no longer match fade out; matching
  callouts grey the letters typed so far. Backspace takes a letter back, Esc
  or a click outside leaves, and a complete label selects.
- **Arrows** move a focus ring across the items still shown, so ⌘S, `c`, then
  ↑ walks the commands only, and ⌘S, `l`, then ↑ ↓ walk the lines. The first arrow starts from the terminal cursor;
  after that ↑ ↓ go to the nearest item above or below, and ← → to the
  previous or next in reading order, crossing panes at the ends. Return
  selects the focused item; ⇧ + arrow selects it and extends.

Terminal frames freeze while choosing, so live output can't move a label.
Scrolling, resizing, tab or focus changes cancel the mode.

## Once something is selected

| Key | Does |
| --- | --- |
| `c` (or ⌘C) | Copy |
| `o` | Open (URLs and existing local paths) |
| `r` | Run (commands; see below) |
| ← → ↑ ↓ | Move to the next item, at the selection's current level (a line: the previous or next line) |
| ⇧ + arrow, ⇧ + a label | Extend the selection (from a line, ⇧ + a line's label) |
| ⌥↑ | Grow: the item around it (a path → its command), its line, paragraph, block, the whole screen |
| ⌥↓ | Shrink back the way ⌥↑ came |
| Backspace | Back to choosing |
| Esc | Leave |

Copying one item copies its value: a command without continuation
backslashes, a quoted path without its quotes. A range copies as a terminal
selection would. Open resolves paths against the source terminal's working
directory and uses the default app. Missing or remote paths and the other
kinds are copy-only.

A popup beside the selection offers the applicable actions for the mouse. A
legend in the focused tile's header strip (through
`TileHeaderView.overrideContent`) lists the keys that apply right now; a tile
without a header strip gets a bar at the bottom instead.

## Run

Run picks a shell (`run::plan`):

1. the current pane, if it's an idle shell;
2. else the nearest idle shell in the tab on the same machine, by distance in
   the layout, then the shell Run used most recently;
3. else a new pane beside the source, in the source's directory.

Idle means the pane's polled foreground process (`pane.process_info`) is the
shell itself. A pane not polled yet, ended, running an agent, or running any
command is busy. A command started less than a poll ago can look idle; most
shells retitle themselves when a command starts, which polls at once.

In an existing shell, Run clears half-typed input (⌃E, ⌃U), types the line and
Return. When the shell sits in another directory, the line starts with
`cd '<source dir>' &&`. Multi-line commands (heredocs, `\` continuations) run
through `/bin/sh -c 'eval "$(printf %s <base64> | base64 -d)"'`, one line that
POSIX shells and fish both parse. The pane Run used takes focus.

`[hints] run_target = "new"` always makes a new pane; the default is
`"reuse"`. Automation keys: `hints.run.target` (`current`, `reuse:<N>`,
`new`) and `hints.run.idle`.

## What counts as an item

Recognition reads visible text and foreground colours. It isn't a shell
parser or a safety check: Run passes the displayed command to the source
machine's shell as it is.

- **Paths** include quotes, escaped spaces, `:line:column` suffixes and
  prefixes such as `key=/path`. A bare existing name like `docs` counts only
  inside a command.
- **Domains** need a known top-level domain from a built-in list that leaves
  out TLDs doubling as file extensions, so `README.md` isn't a host.
- **Hashes and IDs** are 7–40 lowercase hex characters with at least one
  letter and one digit, or UUIDs.
- **Lines** are the non-blank rows, a soft-wrapped row joined to the rows it
  wraps into. The overlay takes a row as wrapped when its last cell holds
  text and the next row starts with text (the `joins` it sends the core).
- **Paragraphs**, what a line grows to, are runs of rows that read as prose
  (three or more words, mostly plain words, no column gaps). Command rows and
  their output never are.
- When items overlap, the longest wins (a URL over its host); a command keeps
  its own label beside the items inside it.

**Commands** come from three places:

- **After a prompt**: a marker (`$`, `%`, `❯`, `➜`, or `#` / `>` attached to a
  prompt) after text that looks like a prompt (user@host, a path, `(venv)`,
  starship or powerlevel10k glyphs). REPL prompts (`>>>`, `mysql>`) and
  numbered menus don't count. A coding agent's `!cmd` works the same way.
- **Coding-agent screens**: Claude Code's `● Bash(…)` tool calls, permission
  prompts and `⎿  $ cmd` rows; Codex's `• Ran …` lines. Box borders and
  output rows are ignored.
- **Without a prompt**: a built-in vocabulary of Unix, macOS, Windows and
  PowerShell command names, starting a line, a backtick span, a coloured run,
  a shell code fence, or following a cue in a sentence ("run it with npx
  vitest"). Names that are also English words (`find`, `test`, `open`) need a
  flag, path or operator. Prose, tables, log lines, usage synopses, diffs and
  single words are rejected.

Items can span rows: wrapped full-width text, lines ending in `\`, `|` or
`&&`, open quotes and heredocs, and indented continuations an application
wrapped itself. A multi-row item gets one label; Copy and Run rejoin it as
the shell would see it.

A second pass, the row classifier (`[hints] row_classifier`, on by default),
labels each row as prompt, continuation, code, output, prose, chrome or
blank with a small perceptron trained on synthetic screens. It drops
commands from rows it's sure are output (an upgrade list, a diff) and can add
ones the detector missed on code rows. `just hints-model` regenerates
`model/rows.json`; `just lint` checks it's current.

## Tests

- `core/crates/hints/data/hints-corpus.txt` holds fictional screens with the
  commands each should yield. `cargo test -p azulejo-hints --test corpus`
  scores recall and precision on it.
- `apps/macos/automation/hints.sh` runs `cargo test -p azulejo-hints`, then
  drives the GUI on a dev instance in a dark and a light theme. Copy uses a
  private pasteboard, Open records its target without launching anything,
  and Run executes a fixture command in idle fish, zsh and bash shells. Its
  lines scenario checks that ⌘S draws no line, that `l` labels each one (a
  wrapped one once, a repeated one twice) and that the arrows walk them.
