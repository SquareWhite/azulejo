# 0016: ⌘S smart select lives in the Rust core

Status: accepted · Nov 12, 2026 · ⌘S (`select_visible_item`)

## Context

⌘S labels what's on the visible terminal screens (URLs, paths, commands, IPs, domains, UUIDs, hashes, branches, text paragraphs) and lets the keys grow, shrink and move a selection over them. All of it was Swift in the macOS shell (TerminalHints.swift, TerminalSelection.swift): regexes, a shell lexer, wrap joins, prose cues, paragraphs, grow and shrink, callout placement. Only the row classifier, a second pass over the commands found, was Rust, called through a second FFI round trip (`azj_hints_refine`). A Linux shell would have had to rewrite the lot and keep two detectors in step.

## Decision

- **Everything that reads text or places labels is Rust, in `core/crates/hints`:** detection (`src/detect`: rows into logical lines, command starts, the shell lexer, atoms, SGR colour runs), selection (`src/select.rs`: labels, repeats, paragraphs, grow and shrink, arrows, range text, the Run line) and callout geometry (`src/callout.rs`). The classifier's refine step runs inside detection, behind `[hints] row_classifier`.
- **One call per pane per ⌘S:** `azj_hints_detect` takes the rows, soft-wrap flags, width, cwd, local flag, branch, the terminal's styled snapshot and the classifier switch as JSON. It returns the items (paragraphs included, in reading order), each row's extent and runs of non-blank text, and an opaque pane handle. Selection keys ask the handle with small C structs (`azj_hints_pane_grow`, `_shrink`, `_neighbour`, `_targets`, `_text`, `_keep`), so the screen crosses once. `azj_hints_labels`, `azj_hints_run_line` and `azj_hints_place_callouts` cover the rest.
- **Offsets stay UTF-16** at the API, as the macOS shell's strings and the classifier already count them. Inside, the detector works in bytes and converts at the edges.
- **The platform question goes through a trait.** `detect::Filesystem` says whether a path exists and whether it's a folder; `StdFilesystem` uses `std::fs` (macOS and Linux alike), tests use a fake. The core builds the `file://` URL itself. The `/home` and `/net` rule (automounts: never stat) and the per-pass stat cache moved with it.
- **The shell keeps only what needs AppKit:** reading rows and the styled snapshot from Ghostty, cell geometry, pill sizes (fonts), drawing, keys and the legend.
- **Tests moved with the logic:** `cargo test -p azulejo-hints` (`tests/detector.rs`, `tests/select.rs`, `tests/corpus.rs` over `data/hints-corpus.txt`). `apps/macos/automation/hints.sh` runs them, then the GUI scenarios.

## Consequences

- A Linux shell calls the same crate and gets the same items. The crate has no platform-specific code; it builds and tests on Linux.
- Parity with the Swift detector was checked during the port: same items, spans, values and open targets on every corpus fixture (remote and local, classifier on and off) and on 6,000 generated screens. Corpus scores are unchanged: recall 99.4%, precision 100%, one known miss.
- Faster: a full 60×120 screen with filesystem checks took about 2.5 ms in Swift; it takes about 0.7 ms in the core, 0.8 ms through the Swift glue with JSON both ways. Debug builds compile the crate and its regex engine optimised (`[profile.dev.package]`) so the dev app stays near 1.2 ms.
- Swift's string semantics came along where they mattered: grapheme-aware matching for a few whole-word tests (a letter plus combining mark isn't an ASCII letter), `CharacterSet.whitespaces` and control characters, grapheme cell widths. A few deeper Foundation quirks (canonical-equivalence string comparison) weren't copied; they only touch combining marks right after ASCII punctuation.
- **Rejected:** keeping detection in Swift and porting later (two copies drift); a callback into the shell for every stat (more glue for no gain: `std::fs` is portable); a leaner binary encoding (JSON is under 0.1 ms of the total).
