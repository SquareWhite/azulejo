# Smart select (⌘S)

In a terminal, **⌘S** (`select_visible_item`) finds the selectable items in
every visible terminal pane of the window: URLs, paths, commands, IP
addresses, domains, hashes and IDs, branches, and every non-blank line. It
shows no labels at first: a key panel at the bottom of the window lists
the categories. Press one to label its items, type a label (or move with
the arrows) to select one, then copy, open or run it. Panes hidden by zoom
or Monocle are left out.

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

## The key panel

A which-key style HUD floats centred above the window's bottom edge for
the whole session and lists the keys that apply right now. It's a
translucent system material (`material.hintPanel`: a HUD in dark themes, a
popover in light ones) under a tint (`color.hintPanel`, which keeps it
legible where the material can't blur, such as the app's own captures),
with a thin edge and a top highlight. Category keys are raised keycaps;
the moving-about keys are flat ones.

- **After ⌘S**, the categories: `u` URLs, `p` paths, `c` commands, `l`
  lines, `h` hashes and IDs (git hashes, UUIDs, other hex IDs), `i` IP
  addresses, `d` domains and `b` branches, each with how many it found.
  Categories with nothing on screen stay listed, dimmed, and their keys do
  nothing. No labels show yet. `esc` closes. A category with one item (a
  count of 1) skips its labels: its key selects that item at once, lines
  included, and `⌫` from it goes straight back to the categories.
- **In a category**, its name and count, then `a–z` pick, the arrows,
  `⏎` select (once the ring is on an item), `⌫` back to the categories and
  `esc`.
- **With an item selected**, what it is and its actions (`c` copy, `o` open,
  `r` run), then the arrows, `⇧` to extend, `⌥↑↓` grow and shrink, `⌫` back
  and `esc`.

It fades and rises in (`motion.paletteOpen`) and fades out with the overlay
(`motion.paletteClose`); with motion off or Reduce Motion it just shows.
When it would hide the labelled items or the selection, it moves to the
bottom right, top right, bottom left or top, the first spot that hides
nothing (else the one hiding least). The category keys are fixed: they
are the core's `Kind::letter`.

## Labels

A category of one item selects it without showing a label. A category
of several shows on its own, so labels only need to be unique within it:
the shortest letters from `asdfghjklqwertyuiopzxcvbnm`, so `a`, `s`, `d`…
up to 26 items. Past 26 the alphabet's last letters start two-letter
labels (`ma`, `ms`…) and those go to the first items in reading order; the
lowest items, the newest output, keep single letters. No label is a
prefix of another (`select::short_labels`). The panel's own keys there
(Esc, Backspace, Return, the arrows) aren't letters, so every letter is
free for labels.

**Lines.** Every non-blank line in each labelled pane is an item: one row,
or one line the terminal soft-wrapped over several rows, without its indent
and trailing blanks. `l` labels every one. A selected line copies its text
(a range of lines copies as a terminal selection would), and ⌥↑ grows it to
its paragraph, block and screen. Lines are copy-only.

Paragraphs have no category of their own: grow a line (⌥↑) to reach one.

When a pane shows the same item several times (a prompt's `~/proj` on every
prompt line), only the newest copy gets a label. The others light up while it
is selected, and the arrows still reach them. Lines are the exception: two
lines that read the same are two places, and each gets its own label.

Labels are neutral ink chips (near-white in dark themes, near-black in
light ones) with a short tail pointing at the item's first cell and a
soft shadow. The category's items take the terminal's own selection colour
under their text, and the rest of the text fades to `alpha.hintOthers`.
`callout::place` puts each one below, above or before its item,
preferring blank space, then plain output, then paragraphs, and never over
another callout or another item's first cell. With nowhere free it counts as
a clash (`hints.callout.flagged`). Each category is placed in a pass of its
own, since it shows alone. Lines are the exception: every row is a line,
so there's no blank space to use. A line's label sits at its start, in the
indent when it fits, else over its first letters.

The overlay draws a fresh terminal frame (`ghostty_surface_draw`) just
before freezing it, then reads the rows again, so the frozen text and the
boxes come from the same screen.

## Choosing

- **Typing filters.** Items whose labels no longer match fade out; matching
  callouts grey the letters typed so far. Backspace takes a letter back
  and, with none typed, goes back to the categories. Esc or a click outside
  leaves, and a complete label selects.
- **Arrows** move a focus ring across the category's items still shown, so
  ⌘S, `c`, then ↑ walks the commands, and ⌘S, `l`, then ↑ ↓ walk the lines.
  The first arrow starts from the terminal cursor; after that ↑ ↓ go to the
  nearest item above or below, and ← → to the previous or next in reading
  order, crossing panes at the ends. Return selects the focused item; ⇧ +
  arrow selects it and extends. On the category list the arrows and Return
  do nothing.

Terminal frames freeze while choosing, so live output can't move a label.
Scrolling, resizing, tab or focus changes cancel the mode.

## Once something is selected

| Key | Does |
| --- | --- |
| `c` (or ⌘C) | Copy |
| `o` | Open (URLs and existing local paths) |
| `r` | Run (commands; see below) |
| ← → ↑ ↓ | Move to the next item, at the selection's current level (a line: the previous or next line) |
| ⇧ + arrow, ⇧ + a label | Extend the selection (⇧ + a label of the selection's category; held, ⇧ shows them) |
| ⌥↑ | Grow: the item around it (a path → its command), its line, paragraph, block, the whole screen |
| ⌥↓ | Shrink back the way ⌥↑ came |
| Backspace | Back to choosing, in the category (to the categories when the category key selected its only item) |
| Esc | Leave |

The category follows the selection: an arrow onto a path, or ⌥↑ to a line,
makes ⇧ + label and Backspace work with paths or lines.

Copying one item copies its value: a command without continuation
backslashes, a quoted path without its quotes. A range copies as a terminal
selection would. Open resolves paths against the source terminal's working
directory and uses the default app. Missing or remote paths and the other
kinds are copy-only.

A popup beside the selection offers the applicable actions for the mouse;
the key panel lists them with their keys.

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
  wraps into. herdr redraws each pane row by row, so the terminal's own
  wrap flag is never set: the overlay takes a row as soft-wrapped when its
  last cell holds text and the next row starts with text (the `joins` it
  sends the core). Rows an application wrapped itself
  stay separate lines: a coding agent's hard-wrapped paragraph is what
  ⌥↑ grows a line to, and the arrows keep walking it row by row.
- **Paragraphs**, what a line grows to, are runs of rows that read as prose
  (three or more words, mostly plain words, no column gaps). Command rows and
  their output never are.
- When items overlap, the longest wins (a URL over its host); a command keeps
  its own label beside the items inside it.

**Commands** come from three places:

- **After a prompt**: a marker (`$`, `%`, `❯`, `➜`, or `#` / `>` attached to a
  prompt) after text that looks like a prompt (user@host, a path, `(venv)`,
  starship or powerlevel10k glyphs). REPL prompts (`>>>`, `mysql>`) and
  numbered menus don't count. A coding agent's `!cmd` works the same way,
  and so does a `! cmd` an agent hands over to type in its answer. The
  program can be a path (`/usr/sbin/tool`, `./run.sh`, `~/bin/tool`).
- **Coding-agent screens**: Claude Code's `● Bash(…)` tool calls, permission
  prompts and `⎿  $ cmd` rows; Codex's `• Ran …` lines. Box borders and
  output rows are ignored, and so is a row starting with an update notice
  ("Update available! Run: …"): it's advice about the agent, not a
  command on the screen.
- **Without a prompt**: a built-in vocabulary of Unix, macOS, Windows and
  PowerShell command names, starting a line, a backtick span, a coloured run,
  a shell code fence, or following a cue in a sentence ("run it with npx
  vitest"). Names that are also English words (`find`, `test`, `open`) need a
  flag, path or operator. Prose, tables, log lines, usage synopses, diffs and
  single words are rejected.

Items can span rows: wrapped full-width text, lines ending in `\`, `|` or
`&&`, open quotes and heredocs, and indented continuations an application
wrapped itself. A multi-row item gets one label and a box on each row;
Copy and Run rejoin it as the shell would see it, without the line breaks
and indents of the wrap.

### Logical lines

Before looking for items, the core joins rows into logical lines
(`detect::lines`):

- **Soft wraps** (the overlay's `joins`) always join.
- **Hard wraps**: a coding agent (Claude Code's Ink, Codex) or `fmt`
  word-wraps its own output with a newline and a repeated indent, so the
  terminal sees separate lines. Row N joins row N+1 when:
  - row N reaches the wrap edge: the next row's first word wouldn't have
    fit after it. The edge is the pane width less two columns, or the
    wrapper's own edge when at least three indented rows show it wraps
    further in. Widths count cells, so wide characters count twice;
  - row N+1 sits at the paragraph's continuation indent (more than zero:
    rows at column zero come from a shell, which soft-wraps), under the
    first row's text or past its bullet, and from the third row on at the
    row above's indent;
  - row N+1 starts nothing new: no bullet or number, prompt, tool call,
    rule or box corner, and both rows sit in the same box or in none;
  - neither row holds columns (three spaces in a row: tables, `ls`);
  - row N doesn't end a sentence that row N+1 follows with a capital.

  A hard wrap joins with a space, as the wrapper took one out; a word
  broken in the middle because it's longer than a row joins without one.
  Claude Code's Ink starts such a word on the row it's on and breaks it at
  the edge, so only paths and links count (a slash so far, or `http` +
  `s://…` making a scheme), every row of the word's rest ends at the same
  column, and a row that opens a sentence (a capital, then words that
  read as prose) never carries one on. Every item kind reads the joined
  line: a link, path or hash over two rows is one item with a box on each.
- A row that ends in such a broken word is judged with the rest of the
  word: `! /System/…/Launch` on its own holds half a program name, so
  whether it starts a command is decided once the path is whole.
- Commands also join their shell continuations, as above. A command that
  fills its row doesn't run on into a sentence below it: that's the answer
  going on.

The overlay can only guess soft wraps (herdr's redraw never sets the
terminal's wrap flag): a row that fills the pane to its last cell, then a
row starting at column zero with text, counts as one. A program that
prints exactly a pane's width and then a newline joins the next line onto
it.

A row painted in one colour from end to end reads as plain text: agents
paint their prose in a text colour, which marks nothing. Colours only count
where they set part of a row apart (inline code, a highlighted keyword),
and on a one-colour row that carries on such a run from the row above or
into the row below (inline code wrapped over three rows).

Lines (`l`) stay per terminal line: a hard-wrapped agent paragraph is
several lines, and ⌥↑ grows one to the paragraph.

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
- `core/crates/hints/tests/logical.rs` checks the logical-line rule:
  agent-wrapped commands (painted or not, at several widths and a wrapper
  edge short of the pane), soft-wrapped lines, sentences that end at the
  edge, bullets, boxes, tables, prompts and URLs broken mid-word (after
  the scheme too), paths and hashes over a wrap, a link ending at the edge
  before a new line, and an agent's `! cmd` lines whose program path
  breaks over the edge, with the prose between them left out.
- `apps/macos/automation/hints.sh` runs `cargo test -p azulejo-hints`, then
  drives the GUI on a dev instance in a dark and a light theme. Copy uses a
  private pasteboard, Open records its target without launching anything,
  and Run executes a fixture command in idle fish, zsh and bash shells. It
  checks that ⌘S shows the key panel and no labels, that each category key
  shows its own items alone with short labels, that a category of one (a
  lone URL, a lone command) selects its item at once, that `l` labels every
  non-blank line (a wrapped one once, a repeated one twice, a full screen
  of them) and that the arrows walk them, that a command an agent wrapped
  itself gets one label, a box on each row and copies as one line, that
  in a narrow pane an agent's `! cmd` lines broken mid-path are three whole
  commands and a link broken after its scheme (or soft-wrapped) is one URL
  that copies and opens whole, that an agent's footer and a bare
  prompt row hold no items, and that the panel fades in except with Reduce
  Motion.

## On-device model spike

A dev-only experiment, not wired into the app: could Apple's on-device
model (FoundationModels, macOS 26) pick commands better than the rules?

- `core/crates/hints/data/model-eval.txt`: 54 fictional screens with 40
  labelled commands, covering commands in an agent's prose, code blocks,
  prompts with typed commands, errors, status lines, update banners,
  editor and tmux status rows, and agent-wrapped commands.
- `azj-hints-candidates` exports each screen's rows, expected commands, the
  rules' commands and the candidates (one per logical line, plus the
  rules' own commands).
- `apps/macos/tools/HintsModelBench` asks the model, with guided generation
  (a `DynamicGenerationSchema`: `@Generable`'s macro plugin ships only with
  Xcode), which candidates hold commands and the exact text of each, keeps
  only picks copied from their candidate, and scores the model alone, the
  rules filtered by it and the two together, with cold and warm latency.
  Nothing leaves the Mac: the system model runs locally.
- `just hints-model-bench` runs it; `cargo test -p azulejo-hints --test
  model_eval` pins the rules' score.

Rules on the corpus (row classifier on): precision 97.5%, recall 97.5%
(39 of 40; misses a hint in parentheses, and joins an npm script banner to
the command above it). The model's side needs Apple Intelligence turned on;
the bench reports the model unavailable otherwise and compares nothing.
Until the model is clearly better and refines within about a second, the
rules stay the only path.
