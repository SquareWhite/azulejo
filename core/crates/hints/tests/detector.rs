//! Recognition boundaries of the ⌘S detector (moved from the macOS shell's
//! automation/hints-detector.swift with the detector itself): URLs, paths
//! and their filesystem checks, branches, IPs, commands at prompts, in
//! agents' tool calls, in prose, backticks and colour runs, and wraps.

use std::collections::HashMap;

use azulejo_hints::detect::files::file_url;
use azulejo_hints::detect::{
    ColorSpan, Filesystem, Input, Item, Kind, detect, detect_rows, styled_rows,
};
use azulejo_hints::text::utf16_len;

mod common;
use common::SHEET;

/// A fixed tree under a fictional working directory.
struct Fake(HashMap<String, bool>);

const DIR: &str = "/fixture/proj";

impl Fake {
    fn new() -> Fake {
        let mut map = HashMap::new();
        for (path, dir) in [
            ("", true),
            ("/src", true),
            ("/src/example.swift", false),
            ("/two words.txt", false),
            ("/round(name)", false),
            ("/docs", true),
        ] {
            map.insert(format!("{DIR}{path}"), dir);
        }
        Fake(map)
    }
}

impl Filesystem for Fake {
    fn kind(&self, path: &str) -> Option<bool> {
        self.0.get(path).copied()
    }
    fn home(&self) -> Option<String> {
        Some("/fixture/home".into())
    }
}

/// One row per line, as the boundary checks read them.
fn rows(rows: &[&str], local: bool, branch: Option<&str>) -> Vec<Item> {
    detect_rows(
        &Input {
            rows,
            cwd: Some(DIR),
            local,
            branch,
            ..Input::default()
        },
        &Fake::new(),
    )
}

fn styled(rows: &[String], colors: &[ColorSpan], wrapped: bool, joins: &[usize]) -> Vec<Item> {
    let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    let input = Input {
        rows: &refs,
        joins,
        cwd: Some(DIR),
        local: true,
        colors,
        ..Input::default()
    };
    if wrapped {
        detect(&input, &Fake::new())
    } else {
        detect_rows(&input, &Fake::new())
    }
}

fn wrapped(
    rows: &[&str],
    joins: &[usize],
    columns: Option<usize>,
    local: bool,
    branch: Option<&str>,
) -> Vec<Item> {
    let input = Input {
        rows,
        joins,
        columns,
        cwd: local.then_some(DIR),
        local,
        branch,
        ..Input::default()
    };
    detect(&input, &Fake::new())
}

fn values(items: &[Item]) -> Vec<&str> {
    items.iter().map(|i| i.value.as_str()).collect()
}

fn commands(items: &[Item]) -> Vec<&str> {
    items
        .iter()
        .filter(|i| i.kind == Kind::Command)
        .map(|i| i.value.as_str())
        .collect()
}

fn of(items: &[Item], kind: Kind) -> Vec<&Item> {
    items.iter().filter(|i| i.kind == kind).collect()
}

fn spans(item: Option<&Item>) -> Vec<String> {
    item.map_or(vec![], |i| {
        i.spans
            .iter()
            .map(|s| format!("{}:{}+{}", s.row, s.start, s.length))
            .collect()
    })
}

fn described(items: &[Item]) -> Vec<String> {
    items
        .iter()
        .map(|i| format!("{} {}", i.kind.name(), i.value))
        .collect()
}

/// The UTF-16 offset and length of `needle` in `row`.
fn utf16_range(row: &str, needle: &str) -> (usize, usize) {
    let at = row.find(needle).unwrap();
    (utf16_len(&row[..at]), utf16_len(needle))
}

fn url(path: &str) -> Option<String> {
    Some(file_url(&format!("{DIR}/{path}"), false))
}

#[test]
fn links_and_schemes() {
    let links = rows(
        &["(https://example.invalid/a_(b)). https://example.invalid/x?q=1&v=2,"],
        true,
        Some("feature/hints"),
    );
    assert_eq!(
        values(
            &of(&links, Kind::Url)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>()
        ),
        [
            "https://example.invalid/a_(b)",
            "https://example.invalid/x?q=1&v=2"
        ],
        "balanced URL punctuation"
    );
    assert!(
        rows(&["javascript:alert(1) data:text/html,test"], true, None).is_empty(),
        "non-openable schemes"
    );
}

#[test]
fn paths_and_files() {
    let files = rows(
        &["./src/example.swift:12:3 'two words.txt' two\\ words.txt"],
        true,
        Some("feature/hints"),
    );
    assert_eq!(
        values(&files),
        ["./src/example.swift:12:3", "two words.txt", "two words.txt"],
        "quoted and escaped paths"
    );
    let opens: Vec<Option<String>> = files.iter().map(|i| i.open.clone()).collect();
    assert_eq!(
        opens,
        [
            url("src/example.swift"),
            url("two words.txt"),
            url("two words.txt")
        ],
        "cwd and line-number resolution"
    );
    let remote = rows(
        &["./src/example.swift file:///tmp/example.txt https://example.invalid"],
        false,
        None,
    );
    let opens: Vec<Option<&str>> = remote.iter().map(|i| i.open.as_deref()).collect();
    assert_eq!(
        opens,
        [None, None, Some("https://example.invalid")],
        "remote files never become local open targets"
    );
    assert_eq!(
        rows(&["./missing.txt"], true, None)[0].open,
        None,
        "missing path remains copy-only"
    );

    let compile = format!("Compiling sample v0.1.0 ({DIR}/src)");
    let build = [
        compile.clone(),
        format!("--prepared:{DIR}/src/example.swift"),
        format!("output={DIR}/src/example.swift"),
        "See (./src/example.swift:12:3).".into(),
        "[./missing.txt]".into(),
        "target/release/example.app".into(),
        format!("{DIR}/round(name)"),
    ];
    let refs: Vec<&str> = build.iter().map(String::as_str).collect();
    let paths: Vec<Item> = rows(&refs, true, Some("feature/hints"))
        .into_iter()
        .filter(|i| i.kind == Kind::Path)
        .collect();
    assert_eq!(
        values(&paths),
        [
            format!("{DIR}/src").as_str(),
            &format!("{DIR}/src/example.swift"),
            &format!("{DIR}/src/example.swift"),
            "./src/example.swift:12:3",
            "./missing.txt",
            "target/release/example.app",
            &format!("{DIR}/round(name)"),
        ],
        "formatted build-output paths"
    );
    let (start, length) = utf16_range(&compile, &format!("{DIR}/src"));
    assert_eq!(
        spans(paths.first()),
        [format!("0:{start}+{length}")],
        "path highlights exclude surrounding punctuation"
    );
    assert!(
        paths[4].open.is_none() && paths[5].open.is_none(),
        "missing relative paths remain copy-only"
    );
}

#[test]
fn branches() {
    let listing = rows(
        &["  feature/other", "* feature/hints", "  main"],
        true,
        Some("feature/hints"),
    );
    assert_eq!(
        values(
            &of(&listing, Kind::Branch)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>()
        ),
        ["feature/other", "feature/hints", "main"],
        "branch listing context"
    );
    let unicode = "界👩‍💻e\u{301} (feature/hints) feature/hints-extra";
    let found = rows(&[unicode], true, Some("feature/hints"));
    let branches = of(&found, Kind::Branch);
    assert_eq!(
        branches
            .iter()
            .map(|i| i.value.as_str())
            .collect::<Vec<_>>(),
        ["feature/hints"],
        "branch token boundaries"
    );
    let (start, length) = utf16_range(unicode, "feature/hints");
    assert_eq!(
        spans(branches.first().copied()),
        [format!("0:{start}+{length}")],
        "UTF-16 range after wide and combining glyphs"
    );
}

#[test]
fn prompts_and_agent_markers() {
    let found = rows(
        &[
            "$ printf example > ran.txt",
            "docs mention printf example",
            "$ printf\tunsafe",
            "100% complete",
            "price$ 42",
            "dev@example.invalid:~/src$ printf qualified",
        ],
        true,
        Some("feature/hints"),
    );
    assert_eq!(
        commands(&found),
        ["printf example > ran.txt", "printf qualified"],
        "explicit prompt and control-character boundary"
    );
    let agent = rows(
        &[
            "!printf agent",
            "  ! printf spaced",
            "!./check --fast",
            "docs mention !printf",
            "!!",
            "!123",
            "!=",
            "!",
            "! printf\tunsafe",
            "$ ! grep absent",
        ],
        true,
        Some("feature/hints"),
    );
    let agent: Vec<Item> = agent
        .into_iter()
        .filter(|i| i.kind == Kind::Command)
        .collect();
    assert_eq!(
        values(&agent),
        [
            "printf agent",
            "printf spaced",
            "./check --fast",
            "! grep absent"
        ],
        "agent markers"
    );
    assert_eq!(
        spans(agent.first()),
        ["0:1+12"],
        "agent command selection excludes its marker"
    );
    assert_eq!(
        commands(&rows(
            &[
                "sudo bash /tmp/setup.sh",
                "git status --short",
                "brew install example",
                "DIR /b",
                "Get-ChildItem -Force"
            ],
            true,
            None
        )),
        [
            "sudo bash /tmp/setup.sh",
            "git status --short",
            "brew install example",
            "DIR /b",
            "Get-ChildItem -Force"
        ],
        "unprompted cross-platform commands"
    );
    assert_eq!(
        commands(&rows(
            &[
                "Use `git status --short`, then continue.",
                "echo `pwd`",
                "git.exe status",
                "notgit status",
                "docs mention sudo bash /tmp/setup.sh"
            ],
            true,
            None
        )),
        ["git status --short", "echo `pwd`", "git.exe status"],
        "code boundaries, shell substitutions and executable names"
    );
}

#[test]
fn colour_runs() {
    let (rows_, colors) = styled_rows(
        "界👩‍💻e\u{301} run once: \u{1b}[94msudo bash /home/example/setup.sh\u{1b}[0m's folder\r\n\
         or add \u{1b}[38;5;117msudo usermod -aG wheel example\u{1b}[39m, and continue\r\n\
         read \u{1b}[38;2;100;170;230m/home/example/setup.sh\u{1b}[0m first\r\n\
         then \u{1b}[34mprintf \u{1b}[0m\u{1b}[1m\u{1b}[34m'%s' \u{1b}[4:3mexample\u{1b}[0m, not the prose",
    );
    let items = styled(&rows_, &colors, false, &[]);
    let found: Vec<&Item> = of(&items, Kind::Command);
    assert_eq!(
        found.iter().map(|i| i.value.as_str()).collect::<Vec<_>>(),
        [
            "sudo bash /home/example/setup.sh",
            "sudo usermod -aG wheel example",
            "printf '%s' example"
        ],
        "colour boundaries select full inline commands and exclude adjoining prose"
    );
    let (start, length) = utf16_range(&rows_[0], "sudo bash /home/example/setup.sh");
    assert_eq!(
        spans(found.first().copied()),
        [format!("0:{start}+{length}")],
        "coloured command UTF-16 boundaries"
    );
    assert!(
        items
            .iter()
            .any(|i| i.kind == Kind::Path && i.value == "/home/example/setup.sh"),
        "paths inside a command stay"
    );
    assert!(
        found.iter().all(|i| i.spans[0].row != 2),
        "colour alone does not make a path runnable"
    );

    let (rows_, colors) =
        styled_rows("Use \u{1b}[34mgit status\u{1b}[0m or \u{1b}[34mbrew update\u{1b}[0m.");
    assert_eq!(
        commands(&styled(&rows_, &colors, false, &[])),
        ["git status", "brew update"],
        "separate same-coloured commands never absorb intervening prose"
    );
    let (rows_, colors) = styled_rows(
        "Run \u{1b}[38;2;100;170;230msudo bash /tmp/a\r\nlong-script.sh\u{1b}[0m then continue",
    );
    let items = styled(&rows_, &colors, true, &[0]);
    let found = of(&items, Kind::Command);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].value, "sudo bash /tmp/along-script.sh",
        "full coloured commands across physical row wraps"
    );
    assert_eq!(found[0].spans.len(), 2);
}

#[test]
fn soft_wraps_and_tool_calls() {
    let items = wrapped(
        &["https://example.invalid/", "long/path"],
        &[0],
        None,
        true,
        None,
    );
    assert_eq!(
        items[0].value, "https://example.invalid/long/path",
        "wrapped URL reconstruction"
    );
    assert_eq!(
        spans(items.first()),
        ["0:0+24", "1:0+9"],
        "wrapped physical row spans"
    );
    // A tool call's command keeps its lines; highlights skip the wrapper,
    // the indent and the closing parenthesis.
    let tool = [
        "● Bash(git commit -m \"$(cat <<'EOF'",
        "      Fix it",
        "",
        "      EOF",
        "      )\")",
        "  ⎿  done",
    ];
    let items = wrapped(&tool, &[], Some(80), false, None);
    let command = items.iter().find(|i| i.kind == Kind::Command);
    assert_eq!(
        command.unwrap().value,
        "git commit -m \"$(cat <<'EOF'\nFix it\n\nEOF\n)\"",
        "tool call heredoc keeps its lines"
    );
    assert_eq!(
        spans(command),
        [
            format!("0:7+{}", utf16_len(tool[0]) - 7),
            "1:6+6".into(),
            "3:6+3".into(),
            "4:6+2".into()
        ],
        "tool call spans skip wrapper and indent"
    );
}

#[test]
fn agent_sheet() {
    let find = concat!(
        r#"find . -type f -name "*.swift" -not -path "./vendor/*" -not -path "./.dev/*" -exec grep -l "#,
        r#""TerminalHint" {} + | xargs -I{} sh -c 'echo "== {}"; wc -l "{}"' | sort -t: -k2 -n | tee /tmp/hints-report.txt"#
    );
    let cargo = r#"cargo build --release --manifest-path shared/Cargo.toml --features "hints,mirror" && ls -lah target/release"#;
    let expected: Vec<String> = [
        ("command", find),
        ("path", "."),
        ("path", "./vendor/*"),
        ("path", "./.dev/*"),
        ("path", "/tmp/hints-report.txt"),
        (
            "url",
            "https://docs.example.invalid/azulejo/hints?section=terminal&lang=en-GB#visible-items",
        ),
        ("domain", "mirror.example.org"),
        ("command", cargo),
        ("path", "shared/Cargo.toml"),
        ("path", "target/release"),
        ("ip", "192.0.2.47"),
        ("ip", "198.51.100.12"),
        ("ip", "198.51.100.12:8443"),
        ("hash", "3f9c2a7e-8b41-4d6f-a0c3-5e17b9d2f864"),
        ("hash", "7c3e9a1f4b2d8e05a6f1c9b3d7e2a4f80b5c6d19"),
        ("hash", "7c3e9a1"),
        ("path", "feature/terminal-hints-v2"),
        ("path", "release/2026.11"),
        ("branch", "main"),
    ]
    .iter()
    .map(|(k, v)| format!("{k} {v}"))
    .collect();
    // The agent wrapped at 95 cells: exactly the pane, or a pane a little
    // wider when the wrapper keeps a right margin.
    for columns in [95, 97] {
        let items = wrapped(SHEET, &[], Some(columns), true, Some("main"));
        assert_eq!(
            described(&items),
            expected,
            "sheet items at {columns} columns"
        );
        assert_eq!(
            spans(items.iter().find(|i| i.value == find)),
            ["2:4+90", "3:2+89", "4:2+21"],
            "agent command spans three rows"
        );
        assert_eq!(
            spans(items.iter().find(|i| i.value == cargo)),
            ["12:4+87", "13:2+19"],
            "prompted command spans two rows"
        );
        assert_eq!(
            spans(items.iter().find(|i| i.value == "198.51.100.12:8443")),
            ["16:3+18"],
            "ip:port drops parentheses"
        );
    }
    // Without the pane width only explicit continuations join.
    assert_eq!(
        commands(&wrapped(SHEET, &[], None, true, None)),
        [
            r#"find . -type f -name "*.swift" -not -path "./vendor/*" -not -path "./.dev/*" -exec grep -l"#,
            r#"cargo build --release --manifest-path shared/Cargo.toml --features "hints,mirror" && ls"#,
        ],
        "hard-wrap joins need the column count"
    );
    assert!(
        !wrapped(SHEET, &[], Some(95), true, None)
            .iter()
            .any(|i| i.value == "docs"),
        "existing bare word in prose"
    );
}

#[test]
fn continuations() {
    let shell = wrapped(
        &[
            "$ docker run \\",
            "> --rm example",
            "$ cat log.txt |",
            "  grep -c error",
            r#"$ echo "one"#,
            r#"two" && true"#,
            "total 48",
        ],
        &[],
        Some(30),
        true,
        None,
    );
    assert_eq!(
        commands(&shell),
        [
            "docker run --rm example",
            "cat log.txt | grep -c error",
            r#"echo "one two" && true"#
        ],
        "backslash, pipe and open-quote continuations"
    );
    assert_eq!(
        spans(shell.first()),
        ["0:2+12", "1:2+12"],
        "continuation highlight skips the secondary prompt"
    );
    assert_eq!(
        wrapped(
            &["! printf it's", "output row", "another row"],
            &[],
            None,
            true,
            None
        )[0]
        .value,
        "printf it's",
        "a quote that never closes doesn't swallow later rows"
    );
    let edge = "$ printf '%s\\n' first second third fourth";
    let items = wrapped(&[edge, "total 48"], &[], Some(edge.len()), true, None);
    assert_eq!(
        items
            .iter()
            .find(|i| i.kind == Kind::Command)
            .unwrap()
            .spans
            .len(),
        1,
        "shell rows at column zero never hard-wrap"
    );
    assert_eq!(
        wrapped(&["  $ printf one", "  two"], &[], Some(80), true, None)[0].value,
        "printf one",
        "short rows don't join"
    );
    assert_eq!(
        wrapped(
            &["  $ printf one two three", "    four"],
            &[],
            Some(24),
            true,
            None
        )[0]
        .value,
        "printf one two three",
        "a different indent ends the command"
    );
    let url = format!("https://example.invalid/{}end", "seg/".repeat(10));
    let head = format!("  see {}", &url[..34]);
    let tail = format!("  {} for details", &url[34..]);
    let cut = wrapped(&[&head, &tail], &[], Some(40), true, None);
    assert_eq!(values(&cut), [url.as_str()], "URL cut at the right edge");
    assert_eq!(spans(cut.first()), ["0:6+34", "1:2+33"]);
    let short = format!("  {} ./src/a", "x".repeat(32));
    assert_eq!(
        values(&wrapped(&[&short, "  word"], &[], Some(42), true, None)),
        ["./src/a"],
        "a short path at the edge stays separate"
    );
}

#[test]
fn atoms_in_ordinary_text() {
    let negatives = wrapped(
        &[
            "Version 2026.11 or v1.2.3, see file.txt, README.md, hints.sh, Cargo.toml, e.g. this.",
            "Words deadbeef facade cafebabe and numbers 1234567 2026111 999.1.1.1 1.2.3.4.5",
            "Times 12:34:56, MAC aa:bb:cc:dd:ee:ff, std::vector, mail dev@example.invalid",
            "docs mention the docs folder",
        ],
        &[],
        None,
        true,
        None,
    );
    assert!(
        negatives.is_empty(),
        "no domains, IPs or hashes in ordinary text: {:?}",
        values(&negatives)
    );
    let more = wrapped(
        &[
            "hosts 2001:db8::1, [2001:db8::2]:8443 and ::ffff:192.0.2.9 or mirror.example.org:8080.",
            "https://192.0.2.10:8080/x mirror.example.com/path/x abcdef0 (feature/hints.)",
            "ls docs",
            "main.rs is not main",
        ],
        &[],
        None,
        true,
        Some("main"),
    );
    assert_eq!(
        described(&more),
        [
            "ip 2001:db8::1",
            "ip [2001:db8::2]:8443",
            "ip ::ffff:192.0.2.9",
            "domain mirror.example.org:8080",
            "url https://192.0.2.10:8080/x",
            "path mirror.example.com/path/x",
            "hash abcdef0",
            "path feature/hints",
            "command ls docs",
            "path docs",
            "branch main"
        ],
        "IPv6, ports, longest overlap and command context"
    );
    assert_eq!(
        values(&wrapped(
            &["Merged into feature/hints.", "(feature/hints)"],
            &[],
            None,
            true,
            Some("feature/hints")
        )),
        ["feature/hints", "feature/hints"],
        "branch names drop sentence punctuation"
    );
}

/// The row classifier's pass over the detector's commands
/// (`[hints] row_classifier`), moved from hints-classifier.swift.
#[test]
fn row_classifier() {
    let run = |rows: &[&str], on: bool| -> Vec<Item> {
        detect(
            &Input {
                rows,
                columns: Some(90),
                row_classifier: on,
                ..Input::default()
            },
            &Fake::new(),
        )
    };
    let cmds = |rows: &[&str], on: bool| {
        commands(&run(rows, on))
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let diff = [
        "user@devbox:~/src/webapp$ git diff scripts/release.sh",
        "diff --git a/scripts/release.sh b/scripts/release.sh",
        "@@ -12,7 +12,8 @@ set -euo pipefail",
        "-npm run build",
        "+npm ci",
        " git tag \"v$version\"",
        " git push origin \"v$version\"",
        "user@devbox:~/src/webapp$",
    ];
    let before = cmds(&diff, false);
    let after = cmds(&diff, true);
    assert!(
        after.first().map(String::as_str) == Some("git diff scripts/release.sh")
            && after.len() < before.len()
            && after.iter().all(|c| before.contains(c)),
        "a command filling a diff's output row is dropped, nothing added: {before:?} -> {after:?}"
    );
    assert!(
        run(&diff, true)
            .iter()
            .any(|i| i.kind == Kind::Path && i.value == "scripts/release.sh"),
        "atoms pass through the classifier"
    );
    assert_eq!(
        cmds(
            &[
                "➜  dash git:(main) npm run build",
                "> dash@0.9.0 build",
                "> vite build"
            ],
            true
        ),
        ["npm run build"]
    );
    let cargo = [
        "❯ cargo test quota",
        "running 2 tests",
        "test quota::tests::resets_daily ... ok",
        "test quota::tests::persists ... ok",
        "",
        "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s",
    ];
    assert_eq!(
        cmds(&cargo, true),
        ["cargo test quota"],
        "test result lines aren't commands"
    );
    let answer = [
        "● To renew the certificate and reload the proxy:",
        "",
        "  sudo certbot renew --cert-name shop.example.invalid",
        "  sudo systemctl reload haproxy",
    ];
    assert_eq!(
        cmds(&answer, true),
        [
            "sudo certbot renew --cert-name shop.example.invalid",
            "sudo systemctl reload haproxy"
        ],
        "code rows in an agent's answer stay commands"
    );
    assert_eq!(cmds(&diff, false), before, "off again: the detector alone");
}

/// Screens as the terminal reads them: every row padded to the pane width,
/// detected with and without the row classifier.
fn screen(rows: &[&str], branch: Option<&str>, colors: &[ColorSpan]) -> [Vec<Item>; 2] {
    let padded: Vec<String> = rows.iter().map(|r| format!("{r:<100}")).collect();
    let refs: Vec<&str> = padded.iter().map(String::as_str).collect();
    [false, true].map(|on| {
        detect(
            &Input {
                rows: &refs,
                columns: Some(100),
                cwd: Some(DIR),
                local: true,
                branch,
                colors,
                row_classifier: on,
                ..Input::default()
            },
            &Fake::new(),
        )
    })
}

/// Items on a row, as `kind value`.
fn on_row(items: &[Item], row: usize) -> Vec<String> {
    items
        .iter()
        .filter(|i| i.spans.iter().any(|s| s.row == row))
        .map(|i| format!("{} {}", i.kind.name(), i.value))
        .collect()
}

/// Coding agents' status areas, editor mode lines and bare prompt rows
/// hold no items; the items around them stay.
#[test]
fn status_lines_and_bare_prompts() {
    let rule = "─".repeat(96);
    let notice = format!(
        "{}Update available! Run: brew upgrade example-tool@latest",
        " ".repeat(40)
    );
    let agent = [
        "✻ Worked for 1m 12s · done 9:41 AM · 1 shell still running",
        notice.as_str(),
        rule.as_str(),
        "❯ tidy up the parser and ship it",
        rule.as_str(),
        "  Model 9 | effort: high | ctx: 40% | 5h: 6% (→14:50) | 7d: 33% (→Sun 11:00)",
        "  feature/demo * | /fixture/proj/src",
        "-- INSERT -- ⏵⏵ auto mode on · 1 shell · ← for agents",
        "  ⏵⏵ accept edits on (shift+tab to cycle)",
        "-- VISUAL LINE --",
        "-- NORMAL --",
    ];
    // The notice is drawn in one colour, as the agent does.
    let colors = [ColorSpan {
        row: 1,
        start: notice.find("Update").unwrap(),
        end: notice.len(),
        color: 0xe5c07b,
    }];
    for (classifier, items) in screen(&agent, Some("feature/demo"), &colors)
        .iter()
        .enumerate()
    {
        let context = format!("row classifier {}", classifier == 1);
        assert!(
            commands(items).is_empty(),
            "{context}: an update notice is no command: {:?}",
            described(items)
        );
        for row in [0, 1, 2, 3, 4, 5, 7, 8, 9, 10] {
            assert!(
                on_row(items, row).is_empty(),
                "{context}: nothing on status row {row} ({:?}): {:?}",
                agent[row],
                on_row(items, row)
            );
        }
        assert_eq!(
            on_row(items, 6),
            ["branch feature/demo", "path /fixture/proj/src"],
            "{context}: the branch and path on the status line stay"
        );
    }

    // Two-line prompts (starship, oh-my-posh): the bare second row holds
    // nothing; with a command typed, the command is an item.
    let prompts = [
        "╭─ /fixture/proj/src (feature/demo|✔)",
        "╰─> ",
        "",
        "/fixture/proj/src on  feature/demo",
        "❯ ",
        "",
        "╭─ /fixture/proj/src (feature/demo|✔)",
        "╰─> git status --short",
    ];
    for (classifier, items) in screen(&prompts, Some("feature/demo"), &[])
        .iter()
        .enumerate()
    {
        let context = format!("row classifier {}", classifier == 1);
        for row in [1, 4] {
            assert!(
                on_row(items, row).is_empty(),
                "{context}: a bare prompt row holds nothing: {:?}",
                on_row(items, row)
            );
        }
        assert!(
            on_row(items, 0).contains(&"path /fixture/proj/src".to_owned()),
            "{context}: the prompt's path stays: {:?}",
            on_row(items, 0)
        );
        assert_eq!(
            commands(items),
            ["git status --short"],
            "{context}: the command after a two-line prompt"
        );
    }

    // Upgrade advice in a sentence is still a command.
    let advice = ["To fix it, run brew upgrade example-tool and try again."];
    for items in screen(&advice, None, &[]) {
        assert_eq!(commands(&items), ["brew upgrade example-tool"]);
    }
}
