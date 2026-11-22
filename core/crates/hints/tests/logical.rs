//! Logical lines: soft wraps and the hard wraps an application made
//! itself join before detection, so an item over several rows is found
//! whole, with a box on each row and its value on one line. All screens
//! are fictional.

use std::collections::HashMap;

use azulejo_hints::detect::lines::{join_lines, soft_lines};
use azulejo_hints::detect::{ColorSpan, Filesystem, Input, Item, Kind, detect};

struct NoFiles;

impl Filesystem for NoFiles {
    fn kind(&self, _: &str) -> Option<bool> {
        None
    }
    fn home(&self) -> Option<String> {
        None
    }
}

/// A coding agent's word wrap (Ink, `fmt`): two-space indent, words moved
/// whole to the next row once the row would pass `edge` cells, rows padded
/// to the pane width as the terminal reads them.
fn agent(text: &str, edge: usize, columns: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::from("  ");
    for word in text.split(' ') {
        if row.len() > 2 && row.len() + 1 + word.len() > edge {
            rows.push(row);
            row = String::from("  ");
        }
        if row.len() > 2 {
            row.push(' ');
        }
        row.push_str(word);
    }
    rows.push(row);
    rows.into_iter().map(|r| format!("{r:<columns$}")).collect()
}

/// Each non-blank row in one text colour, as an agent paints its prose.
fn painted(rows: &[String]) -> Vec<ColorSpan> {
    rows.iter()
        .enumerate()
        .filter(|(_, r)| !r.trim().is_empty())
        .map(|(row, r)| ColorSpan {
            row,
            start: 0,
            end: r.trim_end().len(),
            color: (1 << 24) | 0xe6e6e6,
        })
        .collect()
}

fn find(rows: &[String], columns: usize, joins: &[usize], colors: &[ColorSpan]) -> Vec<Item> {
    let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    detect(
        &Input {
            rows: &refs,
            joins,
            columns: Some(columns),
            colors,
            row_classifier: true,
            ..Input::default()
        },
        &NoFiles,
    )
}

fn commands(items: &[Item]) -> Vec<&Item> {
    items.iter().filter(|i| i.kind == Kind::Command).collect()
}

fn rows_of(item: &Item) -> Vec<usize> {
    item.spans.iter().map(|s| s.row).collect()
}

/// The rows each logical line covers.
fn logical(rows: &[&str], joins: &[usize], columns: usize) -> Vec<Vec<usize>> {
    let mut joined = vec![false; rows.len()];
    for &j in joins {
        joined[j] = true;
    }
    let soft = soft_lines(rows, &joined);
    join_lines(&soft, Some(columns), &HashMap::new())
        .iter()
        .map(|line| {
            let mut covered: Vec<usize> = line.parts.iter().map(|p| p.row).collect();
            covered.dedup();
            covered
        })
        .collect()
}

const LOOP: &str = r#"for env in dev prod; do s=sub-demo-$env-001; echo "== $env $(az provider show -n Example.App --subscription $s --query registrationState -o tsv) kv-rbac=$(az keyvault show -n kvdemo-$env-app01 --subscription $s --query properties.enableRbacAuthorization -o tsv)"; done"#;

/// An agent's answer: a sentence, then a long shell one-liner on its own
/// line, wrapped over several rows, then another paragraph.
fn answer(edge: usize, columns: usize) -> (Vec<String>, usize) {
    let mut rows = agent(
        "If you'd rather skip the script for now, the check is short enough to type by hand. In the demo shell:",
        edge,
        columns,
    );
    let first = rows.len();
    rows.extend(agent(LOOP, edge, columns));
    rows.push(" ".repeat(columns));
    rows.extend(agent(
        "That prints one line per environment.",
        edge,
        columns,
    ));
    (rows, first)
}

#[test]
fn agent_wrapped_command_is_one_item() {
    for columns in [80, 100, 117, 140] {
        let (rows, first) = answer(columns - 2, columns);
        let wrapped = rows
            .iter()
            .skip(first)
            .take_while(|r| !r.trim().is_empty())
            .count();
        assert!(wrapped >= 2, "the fixture wraps at {columns} columns");
        for colors in [vec![], painted(&rows)] {
            let items = find(&rows, columns, &[], &colors);
            let found = commands(&items);
            assert_eq!(
                found.len(),
                1,
                "one command at {columns} columns, painted {}: {:?}",
                !colors.is_empty(),
                found.iter().map(|c| &c.value).collect::<Vec<_>>()
            );
            assert_eq!(found[0].value, LOOP, "copies as one line, joins removed");
            assert_eq!(
                rows_of(found[0]),
                (first..first + wrapped).collect::<Vec<_>>(),
                "a box on every row at {columns} columns"
            );
            for span in &found[0].spans {
                assert_eq!(span.start, 2, "boxes start after the indent");
            }
        }
    }
}

/// The agent wraps well short of the pane's edge: its own edge, which
/// several rows reach, counts.
#[test]
fn wrapper_edge_short_of_the_pane() {
    let columns = 140;
    let mut rows = agent(
        "The deploy script reads its settings from the environment, checks that every required value is set, \
         and stops with a short message naming the first one missing, so a half-configured shell never reaches \
         the cloud at all. Run it from the repository root:",
        100,
        columns,
    );
    let first = rows.len();
    rows.extend(agent(LOOP, 100, columns));
    let items = find(&rows, columns, &[], &painted(&rows));
    let found = commands(&items);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].value, LOOP);
    assert_eq!(rows_of(found[0])[0], first);
}

/// A program printing one long line gets the terminal's soft wrap: the
/// rows join whatever they hold.
#[test]
fn soft_wrapped_line_joins() {
    let columns = 60;
    let line = format!("$ {LOOP}");
    let rows: Vec<String> = line
        .as_bytes()
        .chunks(columns)
        .map(|c| format!("{:<columns$}", String::from_utf8(c.to_vec()).unwrap()))
        .collect();
    let joins: Vec<usize> = (0..rows.len() - 1).collect();
    let items = find(&rows, columns, &joins, &[]);
    let found = commands(&items);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].value, LOOP);
    assert_eq!(rows_of(found[0]), (0..rows.len()).collect::<Vec<_>>());
}

#[test]
fn join_rule() {
    let columns = 72;
    let pad = |r: &str| format!("{r:<columns$}");
    // A paragraph wrapped by the agent joins; the blank row ends it.
    let paragraph = [
        "  The release job builds the archive, signs it with the team key and then",
        "  uploads it to the mirror, which takes about two minutes on a quiet day.",
        "",
        "  Next paragraph.",
    ]
    .map(pad);
    let refs: Vec<&str> = paragraph.iter().map(String::as_str).collect();
    assert_eq!(logical(&refs, &[], columns), [vec![0, 1], vec![2], vec![3]]);

    // A long row that ends a sentence, then a capital: a new paragraph.
    let sentences = [
        "  The build finished and every artefact landed in the release folder.",
        "  Copy them over to the staging host before you tag the release please.",
    ]
    .map(pad);
    let refs: Vec<&str> = sentences.iter().map(String::as_str).collect();
    assert_eq!(logical(&refs, &[], columns), [vec![0], vec![1]]);

    // Short rows never join: the next word would have fit.
    let short = ["  first item", "  second item"].map(pad);
    let refs: Vec<&str> = short.iter().map(String::as_str).collect();
    assert_eq!(logical(&refs, &[], columns), [vec![0], vec![1]]);

    // Rows at column zero come from a shell, which soft-wraps instead.
    let flush = [
        "The release job builds the archive, signs it with the team key and then",
        "uploads it to the mirror, which takes about two minutes on a quiet day.",
    ]
    .map(pad);
    let refs: Vec<&str> = flush.iter().map(String::as_str).collect();
    assert_eq!(logical(&refs, &[], columns), [vec![0], vec![1]]);

    // A bullet's rows hang under its text; the next bullet starts anew.
    let bullets = [
        "  - The release job builds the archive, signs it with the team key and",
        "    then uploads it to the mirror, which takes about two minutes or so.",
        "  - Another bullet that is long enough to reach the edge of this pane ok",
        "    and wraps too.",
    ]
    .map(pad);
    let refs: Vec<&str> = bullets.iter().map(String::as_str).collect();
    assert_eq!(logical(&refs, &[], columns), [vec![0, 1], vec![2, 3]]);

    // Box-drawn UIs: a row inside a box never joins a row outside it, and
    // rules and corners never join.
    let boxed = [
        "  ╭────────────────────────────────────────────────────────────────╮",
        "  │ The release job builds the archive, signs it with the team key │",
        "  ╰────────────────────────────────────────────────────────────────╯",
        "  and then uploads it to the mirror, which takes about two minutes.",
    ]
    .map(pad);
    let refs: Vec<&str> = boxed.iter().map(String::as_str).collect();
    assert_eq!(
        logical(&refs, &[], columns),
        [vec![0], vec![1], vec![2], vec![3]]
    );

    // Columns of output (a table) never join.
    let table = [
        "  example-api        running    3 replicas   eu-west   since yesterday",
        "  example-worker     running    2 replicas   eu-west   since last week",
    ]
    .map(pad);
    let refs: Vec<&str> = table.iter().map(String::as_str).collect();
    assert_eq!(logical(&refs, &[], columns), [vec![0], vec![1]]);

    // A prompt or an agent's tool call starts a line of its own.
    let prompts = [
        "  The release job builds the archive, signs it with the team key and then",
        "  ❯ git status --short",
        "  The release job builds the archive, signs it with the team key and then",
        "  ⏺ Bash(git push origin main)",
    ]
    .map(pad);
    let refs: Vec<&str> = prompts.iter().map(String::as_str).collect();
    assert_eq!(
        logical(&refs, &[], columns),
        [vec![0], vec![1], vec![2], vec![3]]
    );
}

/// Atoms an agent wrapped: a URL longer than a row breaks mid-word and
/// rejoins without a space; one moved whole to the next row stays whole.
#[test]
fn wrapped_atoms() {
    let columns = 60;
    let url = format!("https://docs.example.invalid/{}end", "guide/".repeat(14));
    let mut rows = vec![format!("{:<columns$}", "  The full write-up lives at")];
    let body = &url[..columns - 2 - 2];
    rows.push(format!("  {body}  "));
    rows.push(format!(
        "{:<columns$}",
        format!("  {} for anyone curious.", &url[body.len()..])
    ));
    let items = find(&rows, columns, &[], &[]);
    let urls: Vec<&Item> = items.iter().filter(|i| i.kind == Kind::Url).collect();
    assert_eq!(urls.len(), 1, "{items:?}");
    assert_eq!(urls[0].value, url);
    assert_eq!(rows_of(urls[0]), [1, 2]);
}

/// A coloured inline command soft-wrapped over three rows: the middle row
/// is all one colour, but it carries on the run, so the colour still
/// marks the command.
#[test]
fn coloured_run_through_a_painted_row() {
    let columns = 80;
    let command = format!("sudo bash /tmp/{}check.sh", "sample/".repeat(30));
    let text = format!("Run once: {command}, then continue.");
    let rows: Vec<String> = text
        .as_bytes()
        .chunks(columns)
        .map(|c| String::from_utf8(c.to_vec()).unwrap())
        .collect();
    assert!(rows.len() >= 3);
    let (start, end) = ("Run once: ".len(), "Run once: ".len() + command.len());
    let colors: Vec<ColorSpan> = (0..rows.len())
        .filter_map(|row| {
            let (from, to) = (row * columns, row * columns + rows[row].len());
            let (a, b) = (start.max(from), end.min(to));
            (a < b).then(|| ColorSpan {
                row,
                start: a - from,
                end: b - from,
                color: (1 << 24) | 0x64aae6,
            })
        })
        .collect();
    let joins: Vec<usize> = (0..rows.len() - 1).collect();
    let items = find(&rows, columns, &joins, &colors);
    let found = commands(&items);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].value, command);
}

/// An agent's answer handing over commands to type with its `!` shell
/// escape: two long `lsregister` lines and a short one, with prose
/// between.
const BANG: &str = "! /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /Applications/Azulejo.app && killall Dock
That forces macOS to re-register the installed app.

! /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -gc && killall Dock
That drops the registrations for builds that no longer exist on disk.

As a last resort, wipe the icon cache (it rebuilds itself) and restart the Dock:
! sudo rm -rf /Library/Caches/com.apple.iconservices.store && killall Dock";

const BANG_COMMANDS: [&str; 3] = [
    "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /Applications/Azulejo.app && killall Dock",
    "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -gc && killall Dock",
    "sudo rm -rf /Library/Caches/com.apple.iconservices.store && killall Dock",
];

/// Claude Code's word wrap (Ink's `wrap-ansi`, hard): words move whole to
/// the next row, and a word longer than a row starts where it is and
/// breaks at the edge, unless starting on the next row saves a break.
/// The first row carries `first` (`⏺ ` or the indent), later rows the
/// indent; rows padded to the pane width as the terminal reads them.
fn ink(text: &str, columns: usize, first: &str) -> Vec<String> {
    let width = columns - 2;
    let mut rows: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            rows.push(String::new());
            continue;
        }
        let mut wrapped = vec![String::new()];
        for word in line.split(' ') {
            let length = word.chars().count();
            let current = wrapped.last().unwrap().chars().count();
            if current > 0 {
                if length <= width && current + 1 + length > width {
                    wrapped.push(String::new());
                } else if current < width {
                    wrapped.last_mut().unwrap().push(' ');
                } else {
                    wrapped.push(String::new());
                }
            }
            if length > width {
                let current = wrapped.last().unwrap().chars().count();
                let here = 1 + (length - (width - current) - 1) / width;
                let below = (length - 1) / width;
                if below < here {
                    wrapped.push(String::new());
                }
                for c in word.chars() {
                    if wrapped.last().unwrap().chars().count() >= width {
                        wrapped.push(String::new());
                    }
                    wrapped.last_mut().unwrap().push(c);
                }
            } else {
                wrapped.last_mut().unwrap().push_str(word);
            }
        }
        for row in wrapped {
            let lead = if rows.is_empty() { first } else { "  " };
            rows.push(format!("{lead}{row}"));
        }
    }
    rows.into_iter().map(|r| format!("{r:<columns$}")).collect()
}

/// The first row holding `needle`.
fn row_with(rows: &[String], needle: &str) -> usize {
    rows.iter().position(|r| r.contains(needle)).unwrap()
}

/// The agent wraps each `!` line itself; the long ones break inside the
/// program's path, so the first row holds half a program name and
/// nothing else. Each is still one command, whole and without the `!`,
/// with a box on each of its rows, and the prose rows stay out of them.
#[test]
fn bang_commands_an_agent_wrapped() {
    for columns in [44, 52, 60, 75, 78, 90] {
        for first in ["  ", "⏺ "] {
            let rows = ink(BANG, columns, first);
            for colors in [vec![], painted(&rows)] {
                let items = find(&rows, columns, &[], &colors);
                let found = commands(&items);
                let values: Vec<&str> = found.iter().map(|c| c.value.as_str()).collect();
                assert_eq!(
                    values,
                    BANG_COMMANDS,
                    "{columns} columns, first {first:?}, painted {}:\n{}",
                    !colors.is_empty(),
                    rows.join("\n")
                );
                for prose in ["That forces", "That drops", "As a last resort"] {
                    let row = row_with(&rows, prose);
                    assert!(
                        found.iter().all(|c| !rows_of(c).contains(&row)),
                        "{prose:?} isn't part of a command at {columns} columns"
                    );
                }
                let spans = &found[0].spans;
                assert_eq!(spans[0].row, 0);
                assert_eq!(
                    spans[0].start,
                    first.encode_utf16().count() + 2,
                    "the box starts past the `! `"
                );
                assert!(spans.len() >= 2, "a box on each row");
                for span in &spans[1..] {
                    assert_eq!(span.start, 2, "wrapped rows' boxes start past the indent");
                }
            }
        }
    }
}

/// The same answer from a program that doesn't wrap: the terminal
/// soft-wraps the long lines at column zero.
#[test]
fn bang_commands_soft_wrapped() {
    let columns = 75;
    let mut rows = Vec::new();
    let mut joins = Vec::new();
    for line in BANG.lines() {
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            rows.push(String::new());
        }
        for (n, chunk) in chars.chunks(columns).enumerate() {
            if n > 0 {
                joins.push(rows.len() - 1);
            }
            rows.push(format!("{:<columns$}", chunk.iter().collect::<String>()));
        }
    }
    assert!(joins.len() >= 3, "the fixture wraps");
    let items = find(&rows, columns, &joins, &[]);
    let found = commands(&items);
    let values: Vec<&str> = found.iter().map(|c| c.value.as_str()).collect();
    assert_eq!(values, BANG_COMMANDS);
    assert_eq!(rows_of(found[0]), [0, 1, 2]);
    assert_eq!(rows_of(found[1]), [5, 6]);
}

/// A program named by its path starts a command after a prompt or `!`,
/// and a path broken over the edge is judged whole; a long `git log`
/// wraps into one command. Prose never does.
#[test]
fn path_programs_and_wrapped_commands() {
    let columns = 60;
    let screen = [
        "  ! ./run.sh --flag",
        "  ! ~/bin/tool x",
        "  $ ./scripts/release/check-artefacts.sh --verbose",
        "",
        "  ! /opt/example/toolchains/2026.11/bin/x86_64-unknown-linux-",
        "  gnu-objdump -d target/release/azj",
        "",
        "  ! git log --oneline --graph --decorate --since=2.weeks",
        "  --author=someone -- core/crates/hints",
        "",
        "  The script lives in /opt/example/bin and takes about a minute.",
        "  /System/Library holds the frameworks every app links against.",
    ];
    let rows: Vec<String> = screen.iter().map(|r| format!("{r:<columns$}")).collect();
    for colors in [vec![], painted(&rows)] {
        let items = find(&rows, columns, &[], &colors);
        let found = commands(&items);
        let values: Vec<&str> = found.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(
            values,
            [
                "./run.sh --flag",
                "~/bin/tool x",
                "./scripts/release/check-artefacts.sh --verbose",
                "/opt/example/toolchains/2026.11/bin/x86_64-unknown-linux-gnu-objdump -d target/release/azj",
                "git log --oneline --graph --decorate --since=2.weeks --author=someone -- core/crates/hints",
            ],
            "painted {}",
            !colors.is_empty()
        );
        assert_eq!(rows_of(found[3]), [4, 5]);
        assert_eq!(rows_of(found[4]), [7, 8]);
    }
}

/// A path longer than a row, broken mid-word: a sentence that opens with
/// one stays prose once the path is whole.
#[test]
fn cut_path_in_prose_is_no_command() {
    let columns = 50;
    let path =
        "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework";
    let text = format!("{path} is where the launch database code lives.");
    let rows = ink(&text, columns, "  ");
    assert!(rows.len() >= 2);
    let items = find(&rows, columns, &[], &[]);
    let found = commands(&items);
    assert!(
        found.is_empty(),
        "{:?}",
        found.iter().map(|c| &c.value).collect::<Vec<_>>()
    );
}

/// A command that fills its row to the edge, then a sentence at the same
/// indent: the sentence is the answer going on, not the command wrapping.
#[test]
fn sentence_after_a_full_row_stays_out() {
    let columns = 60;
    let command = "! sudo launchctl kickstart -k system/com.example.daemon && lo";
    let rows: Vec<String> = [
        format!("  {command}"),
        "  That restarts the daemon and tails its log.".into(),
    ]
    .iter()
    .map(|r| format!("{r:<columns$}"))
    .collect();
    assert!(rows[0].trim_end().len() >= columns - 2);
    let items = find(&rows, columns, &[], &[]);
    let found = commands(&items);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].value, &command[2..]);
    assert_eq!(rows_of(found[0]), [0]);
}

fn of_kind(items: &[Item], kind: Kind) -> Vec<&Item> {
    items.iter().filter(|i| i.kind == kind).collect()
}

/// A terminal soft wrap at column zero: `text` cut every `columns` cells,
/// and the rows it wrapped into joined.
fn soft(text: &str, columns: usize) -> (Vec<String>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut joins = Vec::new();
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            rows.push(" ".repeat(columns));
        }
        for (n, chunk) in chars.chunks(columns).enumerate() {
            if n > 0 {
                joins.push(rows.len() - 1);
            }
            rows.push(format!("{:<columns$}", chunk.iter().collect::<String>()));
        }
    }
    (rows, joins)
}

const ARTBOARD: &str =
    "https://claude.ai/artifact/DoFE22EBMbqBjD41cwUQf6#artboard-Icon-A3-1-single-stroke";
const ARTBOARD_TEXT: &str = "The A3 icon boards are on the canvas, to the right of A2 in the icon row: https://claude.ai/artifact/DoFE22EBMbqBjD41cwUQf6#artboard-Icon-A3-1-single-stroke";

/// Claude Code's answer with a link longer than a row: Ink starts it on
/// the row it's on and breaks it at the edge, here right after `http`.
/// It's one URL over both rows, with a box on each, and opens whole.
#[test]
fn url_an_agent_broke_after_its_scheme() {
    let columns = 80;
    let mut rows: Vec<String> = [
        "● Artifact(\"canvas/project/canvas.json\")",
        "  ⎿  Updated https://claude.ai/artifact/DoFE22EBMbqBjD41cwUQf6",
        "  ⎿  Allowed by auto mode classifier",
        "",
    ]
    .iter()
    .map(|r| format!("{r:<columns$}"))
    .collect();
    rows.extend(ink(ARTBOARD_TEXT, columns, "● "));
    assert!(rows[4].trim_end().ends_with(": http"), "{}", rows[4]);
    assert!(rows[5].starts_with("  s://"), "{}", rows[5]);
    for colors in [vec![], painted(&rows)] {
        let items = find(&rows, columns, &[], &colors);
        let urls = of_kind(&items, Kind::Url);
        let values: Vec<&str> = urls.iter().map(|u| u.value.as_str()).collect();
        assert_eq!(
            values,
            [
                "https://claude.ai/artifact/DoFE22EBMbqBjD41cwUQf6",
                ARTBOARD
            ]
        );
        assert_eq!(urls[1].open.as_deref(), Some(ARTBOARD));
        assert_eq!(rows_of(urls[1]), [4, 5]);
        let (first, second) = (urls[1].spans[0], urls[1].spans[1]);
        assert_eq!(
            (first.start, first.length),
            (utf16_len(rows[4].trim_end()) - 4, 4)
        );
        assert_eq!((second.start, second.length), (2, ARTBOARD.len() - 4));
    }
    // Wherever the agent breaks it.
    for columns in [30, 32, 41, 50, 60, 70, 90] {
        let rows = ink(ARTBOARD_TEXT, columns, "● ");
        let items = find(&rows, columns, &[], &[]);
        let urls = of_kind(&items, Kind::Url);
        assert_eq!(urls.len(), 1, "{columns} columns:\n{}", rows.join("\n"));
        assert_eq!(urls[0].value, ARTBOARD, "{columns} columns");
    }
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The same link in a program's plain output: the terminal soft-wraps it
/// at column zero, wherever the edge falls in it.
#[test]
fn url_soft_wrapped() {
    for columns in [40, 77, 80, 81, 100] {
        let (rows, joins) = soft(ARTBOARD_TEXT, columns);
        let items = find(&rows, columns, &joins, &[]);
        let urls = of_kind(&items, Kind::Url);
        assert_eq!(urls.len(), 1, "{columns} columns");
        assert_eq!(urls[0].value, ARTBOARD, "{columns} columns");
        assert!(rows_of(urls[0]).len() >= 2, "{columns} columns");
    }
}

/// Paths and hashes over a wrap: soft-wrapped anywhere, or broken by an
/// agent when longer than a row.
#[test]
fn paths_and_hashes_over_a_wrap() {
    let path = "/var/log/example/services/ingest/2026/11/23/worker-pool-7/output.log";
    let hash = "7c3e9a1f4b2d8e05a6f1c9b3d7e2a4f80b5c6d19";
    let text = format!("The worker wrote {path} before commit {hash} landed.");
    for columns in [30, 45, 61, 70] {
        let (rows, joins) = soft(&text, columns);
        let items = find(&rows, columns, &joins, &[]);
        let paths: Vec<&str> = of_kind(&items, Kind::Path)
            .iter()
            .map(|p| p.value.as_str())
            .collect();
        let hashes: Vec<&str> = of_kind(&items, Kind::Hash)
            .iter()
            .map(|p| p.value.as_str())
            .collect();
        assert_eq!(paths, [path], "soft wrap at {columns} columns");
        assert_eq!(hashes, [hash], "soft wrap at {columns} columns");
    }
    for columns in [40, 52, 60] {
        let rows = ink(&format!("Logs: {path} for now."), columns, "  ");
        let items = find(&rows, columns, &[], &[]);
        let paths = of_kind(&items, Kind::Path);
        assert_eq!(paths.len(), 1, "{columns} columns:\n{}", rows.join("\n"));
        assert_eq!(paths[0].value, path, "{columns} columns");
        assert!(rows_of(paths[0]).len() >= 2);
    }
}

/// A link that ends right at the edge, then a new line of the agent's
/// answer: nothing to join, the next row stays out of it.
#[test]
fn url_at_the_edge_then_a_new_line() {
    let columns = 60;
    let url = format!(
        "https://docs.example.invalid/{}",
        "a".repeat(columns - 2 - 29)
    );
    let rows: Vec<String> = [
        format!("  {url}"),
        "  Next comes the part about the mirror and its settings.".into(),
        "  and lowercase prose after it reads on as its own words.".into(),
    ]
    .iter()
    .map(|r| format!("{r:<columns$}"))
    .collect();
    assert_eq!(rows[0].trim_end().len(), columns);
    let items = find(&rows, columns, &[], &[]);
    let urls = of_kind(&items, Kind::Url);
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0].value, url);
    assert_eq!(rows_of(urls[0]), [0]);
    // At column zero with no soft wrap reported: a hard newline.
    let flush: Vec<String> = [url.clone(), "next".into()]
        .iter()
        .map(|r| format!("{r:<columns$}"))
        .collect();
    let items = find(&flush, columns, &[], &[]);
    let urls = of_kind(&items, Kind::Url);
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0].value, url);
}
