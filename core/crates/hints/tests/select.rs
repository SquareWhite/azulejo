//! ⌘S smart select after recognition (moved from hints-detector.swift):
//! kind-letter labels, text paragraphs, grow/shrink containers, arrow
//! neighbours, range text, repeats, the Run line and callout placement.

use std::collections::HashSet;
use std::process::Command;

use azulejo_hints::callout::{
    self, Callout, Metrics, Point, Rect, Request, Side, place, place_items, tail_base,
};
use azulejo_hints::detect::{Filesystem, Input, Item, Kind, detect, detect_rows};
use azulejo_hints::select::{
    self, Direction, Level, Pane, Pos, TextRange, blocks, extent, focus_step, grow, labels, lines,
    nearest, neighbour, older_repeats, paragraphs, prose, run_line, shrink, text, text_runs,
};

mod common;
use common::SHEET;

struct NoFiles;

impl Filesystem for NoFiles {
    fn kind(&self, _: &str) -> Option<bool> {
        None
    }
    fn home(&self) -> Option<String> {
        None
    }
}

fn wrapped(rows: &[&str], columns: Option<usize>, branch: Option<&str>) -> Vec<Item> {
    detect(
        &Input {
            rows,
            columns,
            branch,
            ..Input::default()
        },
        &NoFiles,
    )
}

fn rows_of(items: &[Item]) -> Vec<Vec<usize>> {
    items
        .iter()
        .map(|i| i.spans.iter().map(|s| s.row).collect())
        .collect()
}

#[test]
fn kind_labels() {
    use Kind::*;
    let kinds = [
        Url, Path, Path, Command, Text, Url, Ip, Hash, Branch, Uuid, Domain,
    ];
    assert_eq!(
        labels(&kinds),
        ["la", "pa", "ps", "c", "t", "ls", "i", "h", "b", "u", "d"]
    );
    let mut many_kinds = vec![Path; 30];
    many_kinds.push(Url);
    let many = labels(&many_kinds);
    assert!(
        many[..30]
            .iter()
            .all(|l| l.len() == 3 && l.starts_with('p')),
        "past 26 of a kind, labels grow a letter"
    );
    assert_eq!(many.iter().collect::<HashSet<_>>().len(), 31);
    assert_eq!(many[30], "l");
    for set in [labels(&kinds), many] {
        assert!(
            !set.iter()
                .any(|a| set.iter().any(|b| a != b && b.starts_with(a.as_str()))),
            "no label is a prefix of another"
        );
    }
}

#[test]
fn sheet_paragraphs() {
    let items = wrapped(SHEET, Some(95), Some("main"));
    let found = paragraphs(SHEET, &items, &[], Some(95));
    assert_eq!(
        rows_of(&found),
        [
            vec![0],
            vec![6, 7, 8],
            vec![15, 16],
            vec![20, 21],
            vec![23, 24]
        ]
    );
    assert!(
        found
            .iter()
            .all(|p| p.kind == Kind::Text && p.open.is_none()),
        "paragraphs are text items"
    );
    assert_eq!(
        found[2].value,
        "The flaky box sits at 192.0.2.47, and its noisy neighbour is at 198.51.100.12 on port 8443\n  (198.51.100.12:8443).",
        "a paragraph copies as a terminal selection"
    );
    assert_eq!(
        (found[2].spans[0].start, found[2].spans[0].length),
        (2, 90),
        "paragraph highlight skips the indent"
    );
    let shell = [
        "$ ls -l",
        "total 8",
        "-rw-r--r--  1 example  staff  12 Nov 11 10:00 notes about the plan",
        "",
        "The build finished and the tests all passed, which is nice.",
        "- first item in a list of things",
        "- second item in the same list",
    ];
    let shell_items = detect_rows(
        &Input {
            rows: &shell,
            ..Input::default()
        },
        &NoFiles,
    );
    assert_eq!(
        rows_of(&paragraphs(&shell, &shell_items, &[], None)),
        [vec![4], vec![5], vec![6]],
        "command output isn't prose; list items are paragraphs of their own"
    );
    assert!(
        paragraphs(
            &["name    size    kind", "a.txt   12      file"],
            &[],
            &[],
            None
        )
        .is_empty(),
        "columns aren't prose"
    );
    assert!(!prose("Session UUID: 3f9c2a7e-8b41-4d6f-a0c3-5e17b9d2f864"));
    assert!(!prose("  Plain domain, no scheme: mirror.example.org"));
    assert!(
        prose("The build is fine: it passed every check today."),
        "a label and one value isn't prose"
    );
}

fn range(a: (usize, usize), b: (usize, usize)) -> TextRange {
    TextRange {
        start: Pos {
            row: a.0,
            offset: a.1,
        },
        end: Pos {
            row: b.0,
            offset: b.1,
        },
    }
}

#[test]
fn range_text() {
    let rows = ["  alpha beta   ", "gamma delta", "epsilon"];
    let r = range((0, 8), (2, 3));
    assert_eq!(
        text(&r, &rows, &[]),
        "beta\ngamma delta\neps",
        "range text joins rows with newlines"
    );
    assert_eq!(
        text(&r, &rows, &[1]),
        "beta\ngamma deltaeps",
        "soft-wrapped rows join without one"
    );
}

#[test]
fn grow_shrink_and_arrows() {
    let items = wrapped(SHEET, Some(95), Some("main"));
    let paras = paragraphs(SHEET, &items, &[], Some(95));
    let all: Vec<Item> = items.iter().cloned().chain(paras.iter().cloned()).collect();
    // Grow: path → its command → its rows → the screen (the block is the
    // same rows here, so it's skipped); shrink goes back in.
    let vendor = all.iter().position(|i| i.value == "./vendor/*").unwrap();
    let mut chain = Vec::new();
    let mut current = extent(&all[vendor]);
    while let Some(unit) = grow(&current, &all, SHEET) {
        chain.push(unit);
        current = unit.range;
    }
    assert_eq!(
        chain.iter().map(|u| u.level).collect::<Vec<_>>(),
        [Level::Item, Level::Line, Level::Screen]
    );
    assert_eq!(all[chain[0].entry.unwrap()].kind, Kind::Command);
    assert_eq!(
        chain[1].range,
        range((2, 2), (4, 23)),
        "a command's line takes its prompt marker and every row"
    );
    let ip = all.iter().position(|i| i.value == "192.0.2.47").unwrap();
    let mut levels = Vec::new();
    current = extent(&all[ip]);
    while let Some(unit) = grow(&current, &all, SHEET) {
        levels.push(unit.level);
        current = unit.range;
    }
    assert_eq!(
        levels,
        [Level::Line, Level::Text, Level::Screen],
        "grow chain in prose"
    );
    assert_eq!(
        shrink(&extent(&paras[2]), &all, Level::Item).map(|u| all[u.entry.unwrap()].value.as_str()),
        Some("192.0.2.47"),
        "shrink with no history: the first item inside"
    );
    let lone = all.iter().position(|i| i.kind == Kind::Url).unwrap();
    assert!(shrink(&extent(&all[lone]), &all, Level::Item).is_none());
    assert_eq!(
        shrink(&extent(&all[lone]), &all, Level::Line).and_then(|u| u.entry),
        Some(lone),
        "a line that is one item"
    );

    // Arrows: reading order (a command before the paths inside it) and rows.
    let ranges: Vec<TextRange> = items.iter().map(extent).collect();
    let find = items.iter().position(|i| i.kind == Kind::Command).unwrap();
    assert_eq!(
        neighbour(&ranges, &ranges[find], Direction::Right).map(|i| items[i].value.as_str()),
        Some("./vendor/*")
    );
    assert_eq!(
        neighbour(&ranges, &ranges[find + 1], Direction::Left),
        Some(find),
        "← goes back out"
    );
    let url = items.iter().position(|i| i.kind == Kind::Url).unwrap();
    assert_eq!(
        neighbour(&ranges, &ranges[url], Direction::Down).map(|i| items[i].value.as_str()),
        Some("mirror.example.org")
    );
    assert_eq!(
        neighbour(&ranges, &ranges[url], Direction::Up).map(|i| items[i].kind),
        Some(Kind::Path)
    );
    assert_eq!(
        neighbour(&ranges, &ranges[0], Direction::Left),
        None,
        "nothing before the first item"
    );
    let row_lines = lines(SHEET);
    assert_eq!(
        neighbour(&row_lines, &row_lines[0], Direction::Down).map(|i| row_lines[i].start.row),
        Some(2)
    );
    assert_eq!(blocks(SHEET).len(), 9, "blocks: runs of non-blank rows");

    // The same through a pane, as the shell drives it.
    let pane = Pane::new(
        SHEET.iter().map(|s| s.to_string()).collect(),
        vec![],
        items.clone(),
        Some(95),
    );
    assert_eq!(pane.items.len(), items.len() + paras.len());
    let first = pane.targets(Level::Item)[0];
    assert_eq!(
        pane.neighbour(&first, Direction::Right)
            .map(|u| pane.items[u.entry.unwrap()].value.clone()),
        Some("./vendor/*".into())
    );
    assert_eq!(pane.targets(Level::Screen).len(), 0);
    assert_eq!(
        pane.text(&first.range),
        select::text(&first.range, SHEET, &[])
    );
}

#[test]
fn choosing_focus_moves_among_shown_items() {
    let a = range((0, 2), (0, 12)); // a command
    let b = range((1, 4), (1, 8)); // a path
    let c = range((3, 2), (3, 14)); // a command
    let d = range((5, 2), (5, 9)); // a command
    let all = [a, b, c, d];
    let at = |row, offset| Pos { row, offset };
    let step = |targets: &[TextRange], focus: Option<&TextRange>, cursor, direction| {
        focus_step(targets, focus, cursor, direction).map(|i| targets[i])
    };

    // First press: from the cursor, at the bottom after the latest output.
    for direction in [
        Direction::Up,
        Direction::Down,
        Direction::Left,
        Direction::Right,
    ] {
        assert_eq!(
            step(&all, None, at(6, 0), direction),
            Some(d),
            "{direction:?} from a cursor below everything: the last item"
        );
    }
    // A cursor in the middle: ↑ the closest row above, ↓ the first below.
    assert_eq!(step(&all, None, at(2, 0), Direction::Up), Some(b));
    assert_eq!(step(&all, None, at(2, 0), Direction::Down), Some(c));
    assert_eq!(step(&all, None, at(2, 0), Direction::Left), Some(b));
    assert_eq!(step(&all, None, at(2, 0), Direction::Right), Some(c));
    assert_eq!(
        step(&all, None, at(0, 0), Direction::Up),
        Some(a),
        "nothing above the cursor: the nearest item"
    );
    assert_eq!(
        step(&all, None, at(0, 0), Direction::Left),
        Some(a),
        "nothing before the cursor: the nearest item"
    );

    // Restricted to the commands (after typing `c`), the path is skipped.
    let commands = [a, c, d];
    assert_eq!(
        step(&commands, Some(&a), at(6, 0), Direction::Down),
        Some(c)
    );
    assert_eq!(step(&commands, Some(&d), at(6, 0), Direction::Up), Some(c));
    assert_eq!(
        step(&commands, Some(&a), at(6, 0), Direction::Right),
        Some(c)
    );
    assert_eq!(step(&commands, Some(&d), at(6, 0), Direction::Right), None);
    assert_eq!(step(&commands, Some(&a), at(6, 0), Direction::Up), None);
    assert_eq!(step(&[], None, at(6, 0), Direction::Up), None);

    // Typing filters out the focused path: the nearest command takes over
    // (one row away either side: the one nearer in column, then the first).
    assert_eq!(nearest(&commands, &b).map(|i| commands[i]), Some(a));
    assert_eq!(nearest(&commands, &d).map(|i| commands[i]), Some(d));
    assert_eq!(nearest(&[c, d], &a).map(|i| [c, d][i]), Some(c));
    assert_eq!(nearest(&[], &a), None);
}

#[test]
fn choosing_focus_through_a_pane() {
    let rows = [
        "$ git status --short",
        " M src/main.rs",
        "$ cargo test -p azulejo-hints",
        "",
        "$ ls src/",
        "",
    ];
    let items = wrapped(&rows, Some(80), None);
    let pane = Pane::new(
        rows.iter().map(|s| s.to_string()).collect(),
        vec![],
        items,
        Some(80),
    );
    let describe = |i: Option<usize>| i.map(|i| pane.items[i].value.clone());
    let commands: Vec<usize> = (0..pane.items.len())
        .filter(|&i| pane.items[i].kind == Kind::Command)
        .collect();
    assert_eq!(commands.len(), 3);
    let cursor = Some(Pos { row: 5, offset: 0 });
    let first = pane.focus_step(&commands, None, cursor, Direction::Up);
    assert_eq!(describe(first), Some("ls src/".into()), "↑ from the prompt");
    let second = pane.focus_step(&commands, first, cursor, Direction::Up);
    assert_eq!(
        describe(second),
        Some("cargo test -p azulejo-hints".into()),
        "↑ again skips everything but commands"
    );
    let third = pane.focus_step(&commands, second, cursor, Direction::Up);
    assert_eq!(describe(third), Some("git status --short".into()));
    assert_eq!(
        pane.focus_step(&commands, third, cursor, Direction::Up),
        None
    );
    assert_eq!(
        pane.focus_step(&commands, None, None, Direction::Down),
        first,
        "no cursor: as if below the last row"
    );
    let path = (0..pane.items.len())
        .find(|&i| pane.items[i].value == "src/main.rs")
        .unwrap();
    assert_eq!(
        describe(pane.nearest(&commands, path)),
        Some("git status --short".into()),
        "a filtered-out focus moves to the nearest item left"
    );
    assert_eq!(pane.nearest(&[], path), None);
    assert_eq!(
        pane.focus_step(&[99], None, cursor, Direction::Up),
        None,
        "indices past the items are ignored"
    );
}

#[test]
fn repeats() {
    let older = older_repeats(&["a", "b", "a", "a"]);
    assert_eq!(older, HashSet::from([0, 2]));
    assert!(older_repeats(&["a", "b"]).is_empty());
    let prompts = [
        "~/proj (main|↑7|✔) echo kek",
        "kek",
        "~/proj (main|↑7|✔) ls",
        "notes.txt  src",
        "~/proj (main|↑7|✔) git status --short",
        " M src/main.rs",
        "~/proj (main|↑7|✔)",
    ];
    let items = wrapped(&prompts, Some(80), None);
    let keys: Vec<(Kind, &str)> = items.iter().map(|i| (i.kind, i.value.as_str())).collect();
    let repeats = older_repeats(&keys);
    let copies: Vec<usize> = (0..items.len())
        .filter(|&i| items[i].kind == Kind::Path && items[i].value == "~/proj")
        .collect();
    let last = *copies.last().unwrap();
    assert!(copies.len() > 1);
    assert!(
        copies[..copies.len() - 1]
            .iter()
            .all(|i| repeats.contains(i))
            && !repeats.contains(&last)
    );
    assert_eq!(
        items[last].spans[0].row,
        prompts.len() - 1,
        "a prompt's repeated cwd: only the lowest copy keeps a label"
    );
    let labelled = select::pane_labels(
        &items
            .iter()
            .map(|i| (0, i.kind, i.value.as_str()))
            .collect::<Vec<_>>(),
    );
    assert!(
        copies[..copies.len() - 1]
            .iter()
            .all(|&i| labelled[i].is_empty())
            && !labelled[last].is_empty()
    );
}

#[test]
fn run_lines() {
    assert_eq!(
        run_line("printf hint-ran > ran.txt"),
        "printf hint-ran > ran.txt",
        "single-line commands run as typed"
    );
    let heredoc = "cat <<'EOF'\nit's \"quoted\" \\\\ back\\slash $HOME\n\ttabbed\nEOF\nprintf '%s\\n' one \\\n  two";
    let line = run_line(heredoc);
    assert!(
        !line.contains('\n')
            && !line.contains('\t')
            && line.starts_with("/bin/sh -c '")
            && line.ends_with('\'')
    );
    let expected = "it's \"quoted\" \\\\ back\\slash $HOME\n\ttabbed\none\ntwo\n";
    let output = |shell: &str, args: &[&str]| -> Option<String> {
        let out = Command::new(shell)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", "/nonexistent.invalid")
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    assert_eq!(
        output("/bin/sh", &["-c", &line]).as_deref(),
        Some(expected),
        "the /bin/sh run line runs the heredoc unchanged"
    );
    let fish = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(|d| format!("{d}/fish"))
        .find(|p| std::path::Path::new(p).is_file());
    if let Some(fish) = fish {
        assert_eq!(
            output(&fish, &["--no-config", "-c", &line]).as_deref(),
            Some(expected),
            "the same line under fish"
        );
    }
}

#[test]
fn text_runs_skip_blanks() {
    assert_eq!(text_runs("  ab\u{a0}c  界d "), [(2, 2), (5, 1), (8, 2)]);
}

// MARK: Callouts: 10×20 cells, five rows; 18×16 pills whose tail keeps 7
// from their ends and may lean half a cell (reach: 2 + 5 sideways).

fn cell(column: usize, row: usize) -> Rect {
    Rect::new(column as f64 * 10.0, row as f64 * 20.0, 10.0, 20.0)
}

fn cells(row: usize, columns: std::ops::Range<usize>) -> Vec<Rect> {
    columns.map(|c| cell(c, row)).collect()
}

const PANE: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 400.0,
    h: 100.0,
};

fn metrics(bounds: Rect) -> Metrics {
    Metrics {
        bounds,
        tail: 4.0,
        spacing: 2.0,
        inset: 7.0,
        drift: 5.0,
    }
}

fn place_cells(
    cells: &[Rect],
    obstacles: &[Rect],
    avoid: &[Vec<Rect>],
    bounds: Rect,
) -> Vec<Callout> {
    let requests: Vec<Request> = cells
        .iter()
        .map(|&c| Request {
            cell: c,
            size: (18.0, 16.0),
            inline: None,
            inline_end: None,
        })
        .collect();
    place(&requests, obstacles, avoid, metrics(bounds))
}

fn upright(callouts: &[Callout], inset: f64, drift: f64) -> bool {
    callouts.iter().all(|c| {
        let base = tail_base(&c.frame, c.tip, c.side, inset);
        assert_eq!(base, c.base, "the placement's tail base");
        c.side == Side::Inline
            || (if c.side == Side::Left {
                base.y - c.tip.y
            } else {
                base.x - c.tip.x
            })
            .abs()
                <= drift + 0.01
    })
}

#[test]
fn callouts_on_cells() {
    let one = place_cells(&[cell(5, 1)], &[], &[], PANE)[0];
    assert_eq!(one.frame, Rect::new(46.0, 44.0, 18.0, 16.0));
    assert_eq!(one.tip, Point { x: 55.0, y: 40.0 });
    assert!(
        one.side == Side::Below && !one.nudged && !one.clash,
        "hangs below its first cell, centred"
    );
    let bottom = place_cells(&[cell(5, 4)], &[], &[], PANE)[0];
    assert!(
        bottom.side == Side::Above
            && bottom.frame.max_y() == 76.0
            && bottom.tip == Point { x: 55.0, y: 80.0 },
        "last row flips above"
    );
    let pair = place_cells(&[cell(5, 1), cell(6, 1)], &[], &[], PANE);
    assert!(
        !pair[1].frame.intersects(&pair[0].frame)
            && pair[1].side == Side::Above
            && !pair[1].nudged
            && pair[1].tip.x == 65.0
            && !pair[1].clash
    );
    let short = place_cells(&[cell(5, 1)], &[cell(5, 1), cell(4, 2)], &[], PANE)[0];
    assert!(
        short.side == Side::Below
            && short.nudged
            && short.frame.min_x() == 52.0
            && upright(&[short], 7.0, 5.0),
        "a short slide"
    );
    let blocked = place_cells(&[cell(5, 1)], &[cell(5, 1), cell(5, 2)], &[], PANE)[0];
    assert!(
        !blocked.frame.intersects(&cell(5, 2)) && blocked.side == Side::Above && !blocked.clash,
        "clears the item below"
    );
    let soft = place_cells(&[cell(5, 1)], &[], &[cells(2, 0..40)], PANE)[0];
    assert!(
        soft.side == Side::Above
            && !soft.nudged
            && soft.relaxed == 0
            && soft.tip == Point { x: 55.0, y: 20.0 }
    );
    let margin = place_cells(
        &[cell(5, 1)],
        &[],
        &[[cells(0, 0..40), cells(2, 0..40)].concat()],
        PANE,
    )[0];
    assert!(
        margin.side == Side::Left
            && margin.relaxed == 0
            && margin.tip == Point { x: 50.0, y: 30.0 }
    );
    assert_eq!(
        margin.frame,
        Rect::new(28.0, 22.0, 18.0, 16.0),
        "a blank margin before the item"
    );
    let all_round = [cells(0, 0..40), cells(2, 0..40), cells(1, 0..4)].concat();
    let packed = place_cells(&[cell(5, 1)], &[], std::slice::from_ref(&all_round), PANE)[0];
    assert!(
        packed.side == Side::Below && packed.relaxed == 1 && !packed.clash,
        "text all round: back below, never flagged"
    );
    let layered = place_cells(&[cell(5, 1)], &[], &[all_round, cells(2, 0..40)], PANE)[0];
    assert!(
        layered.side == Side::Above && layered.relaxed == 1,
        "over plain text rather than an item's"
    );
    let edge = place_cells(&[cell(0, 1)], &[], &[], PANE)[0];
    assert!(
        edge.frame.min_x() == 0.0 && edge.tip.x == 5.0 && upright(&[edge], 7.0, 5.0),
        "clamped at the pane edge"
    );
    let padded = place_cells(&[cell(0, 1)], &[], &[], Rect::new(-8.0, 0.0, 416.0, 100.0))[0];
    assert!(
        padded.frame.min_x() == -4.0 && !padded.nudged,
        "the pill may hang into the pane's padding"
    );
    let row = place_cells(&cells(1, 0..30), &[], &[], PANE);
    assert!(!row.iter().any(|c| c.clash) && upright(&row, 7.0, 5.0));
    for (i, c) in row.iter().enumerate() {
        assert_eq!(c.side, if i % 2 == 0 { Side::Below } else { Side::Above });
        assert!(
            !row[i + 1..].iter().any(|o| o.frame.intersects(&c.frame)),
            "a row of neighbours alternates, never overlapping"
        );
    }
    let crowd = place_cells(
        &cells(1, 0..40),
        &[cells(0, 0..40), cells(1, 0..40), cells(2, 0..40)].concat(),
        &[],
        PANE,
    );
    assert!(
        crowd.iter().all(|c| c.clash) && upright(&crowd, 7.0, 5.0),
        "first cells all round: flagged, still upright"
    );
    // A column of paths at the pane's left edge, one per row: each label
    // sits over its own path's start.
    let column: Vec<callout::Item> = (0..5)
        .map(|row| callout::Item {
            anchor: cell(0, row),
            rects: vec![Rect::new(0.0, row as f64 * 20.0, 120.0, 20.0)],
            prose: false,
            size: [18.0, 16.0],
        })
        .collect();
    let text: Vec<Rect> = column.iter().flat_map(|i| i.rects.clone()).collect();
    let stacked = place_items(&column, &text, metrics(PANE));
    assert!(
        stacked
            .iter()
            .zip(&column)
            .all(|(c, item)| c.side == Side::Inline
                && !c.clash
                && item.rects[0].contains_rect(&c.frame))
    );
}

/// Dense screens for callout placement: a coding agent's tool call whose
/// command wraps over three rows, compiler output with a column of paths
/// at the left edge, and search results with paths and addresses packed
/// together.
const PLACEMENT: &[(&str, usize, &[&str])] = &[
    (
        "tool call",
        90,
        &[
            "● Bash(gh run watch 4321 --interval 30 --exit-status > /dev/null 2>&1; echo \"exit=$?\"; gh r…)",
            "  ⎿  $ gh run watch 4321 --interval 30 --exit-status > /dev/null 2>&1; echo \"exit=$?\"; gh",
            "     run view 4321 --json conclusion,jobs -q",
            "     '{c:.conclusion,jobs:[.jobs[]|{n:.name,c:.conclusion}]}' (4m 29s)",
            "     (ctrl+b to run in background)",
            "",
            "● The run passed. Next I'll check the release notes in docs/release.md.",
        ],
    ),
    (
        "tool call, narrow",
        52,
        &[
            "  └  $ gh run watch 4321 --interval 30 --exit-status",
            "     > /dev/null 2>&1; echo \"exit=$?\"; gh run view",
            "     4321 --json conclusion,jobs -q '{c:.conclusion,",
            "     jobs:[.jobs[]|{n:.name,c:.conclusion}]}'",
            "     (4m 29s)",
            "     (ctrl+b to run in background)",
        ],
    ),
    (
        "compiler output",
        90,
        &[
            "src/upload/client.ts:31:7 - error TS2345: Argument of type 'string' is not assignable.",
            "src/upload/retry.ts:12:3 - error TS2304: Cannot find name 'backoff'.",
            "src/session/store.ts:88:14 - warning: unused variable 'ttl'.",
            "tests/int/session.test.ts:4:1 - error TS1005: ';' expected.",
            "$ npm run lint -- --fix && npm test -- --runInBand",
            "$ git diff --stat HEAD~1",
            " src/upload/client.ts | 31 +++++++++++++++++++++++++------",
            " src/upload/retry.ts  | 12 ++++++------",
            " 2 files changed, 30 insertions(+), 13 deletions(-)",
        ],
    ),
    (
        "search results",
        90,
        &[
            "● Bash(rg -n \"retry\" src/ | head -5 && curl -s https://api.example.invalid/v1/status)",
            "  ⎿  src/upload/retry.ts:12:  const retry = backoff(3);",
            "     src/upload/client.ts:31:  await retry(() => put(url));",
            "     src/session/store.ts:40:  // retry once on 503 from 192.0.2.10:8443",
            "     {\"status\":\"ok\",\"region\":\"eu-west\",\"commit\":\"7c3e9a1\"}",
            "● Update(src/upload/retry.ts)",
            "  ⎿  Updated src/upload/retry.ts with 2 additions and 1 removal",
        ],
    ),
];

#[test]
fn callouts_on_dense_screens() {
    for &(name, columns, rows) in PLACEMENT {
        let detected = wrapped(rows, Some(columns), None);
        let pane = Pane::new(
            rows.iter().map(|s| s.to_string()).collect(),
            vec![],
            detected,
            Some(columns),
        );
        let found = &pane.items;
        let kinds: Vec<Kind> = found.iter().map(|i| i.kind).collect();
        let labels = labels(&kinds);
        // 8×17 cells, one per UTF-16 unit (these screens are ASCII but for ● and ⎿).
        let (width, height) = (8.0, 17.0);
        let rect = |row: usize, start: usize, length: usize| {
            Rect::new(
                start as f64 * width,
                row as f64 * height,
                length as f64 * width,
                height,
            )
        };
        let pill = (height * 0.74f64).round();
        let tail = (height * 0.24f64).round().max(3.0);
        let inset = (pill * 0.3).round() + (tail * 0.9f64).max(2.5);
        let items: Vec<callout::Item> = found
            .iter()
            .zip(&labels)
            .map(|(item, label)| {
                let rects: Vec<Rect> = item
                    .spans
                    .iter()
                    .map(|s| rect(s.row, s.start, s.length))
                    .collect();
                callout::Item {
                    anchor: Rect::new(rects[0].x, rects[0].y, width, height),
                    rects,
                    prose: item.kind == Kind::Text,
                    size: [
                        (label.len() as f64 * 6.5 + 2.0 * (pill * 0.32).round()).max(pill),
                        pill,
                    ],
                }
            })
            .collect();
        let text: Vec<Rect> = rows
            .iter()
            .enumerate()
            .flat_map(|(row, line)| text_runs(line).into_iter().map(move |(s, l)| (row, s, l)))
            .map(|(row, s, l)| rect(row, s, l))
            .collect();
        let bounds = Rect::new(
            -8.0,
            -4.0,
            columns as f64 * width + 16.0,
            rows.len() as f64 * height + 12.0,
        );
        let callouts = place_items(
            &items,
            &text,
            Metrics {
                bounds,
                tail,
                spacing: 2.0,
                inset,
                drift: width / 2.0,
            },
        );
        let covers: Vec<usize> = (0..callouts.len())
            .filter(|&index| {
                let point = Point {
                    x: items[index].anchor.mid_x(),
                    y: items[index].anchor.mid_y(),
                };
                let frame = callouts[index].frame.inset(0.5, 0.5).unwrap();
                items.iter().enumerate().any(|(other, item)| {
                    other != index
                        && !item.prose
                        && !item.rects.iter().any(|r| r.contains(point))
                        && item.rects.iter().any(|r| r.intersects(&frame))
                })
            })
            .collect();
        // (A paragraph and the path it starts with share a first cell.)
        let overlaps = callouts.iter().enumerate().any(|(i, c)| {
            let frame = c.frame.inset(0.5, 0.5).unwrap();
            callouts[i + 1..].iter().any(|o| o.frame.intersects(&frame))
                || items
                    .iter()
                    .any(|o| o.anchor != items[i].anchor && frame.intersects(&o.anchor))
        });
        let sides: HashSet<Side> = callouts.iter().map(|c| c.side).collect();
        assert!(
            sides.len() > 1,
            "{name}: callouts use more than one side: {sides:?}"
        );
        assert!(
            !callouts.iter().any(|c| c.clash) && !overlaps,
            "{name}: callouts clear each other and every first cell"
        );
        assert!(
            upright(&callouts, inset, width / 2.0),
            "{name}: every tail nearly upright"
        );
        assert!(
            covers.is_empty(),
            "{name}: no callout covers another item's text: {covers:?}"
        );
    }
}
