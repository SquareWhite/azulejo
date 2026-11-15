//! The detector's corpus (`data/hints-corpus.txt`): screens as a terminal
//! or coding agent shows them, with the commands a person would want, and
//! its scorer (`cargo test -p azulejo-hints --test corpus`).
//!
//! ```text
//! @@ fixture <name> | <category> | cols <n>   starts a screen; rows follow verbatim.
//!    Rows longer than <n> soft-wrap like a terminal; \e in a row is ESC (SGR colour).
//! @@ cmd <value>      an expected command; "@@ + <text>" adds a newline and <text>.
//! @@ item <kind> <v>  a non-command item that must be found too.
//! @@ xfail <why>      a known miss: reported, not fatal.
//! ```

use std::collections::BTreeMap;

use unicode_segmentation::UnicodeSegmentation;

use super::{ColorSpan, Filesystem, Input, Item, Kind, detect, styled_rows};

#[derive(Debug, Clone, Default)]
pub struct Fixture {
    pub name: String,
    pub category: String,
    pub columns: usize,
    pub rows: Vec<String>,
    pub commands: Vec<String>,
    pub items: Vec<(String, String)>,
    pub xfail: Option<String>,
}

/// A fixture as the overlay captures it: styled rows decoded, rows wider
/// than the pane soft-wrapped with their join flags.
#[derive(Debug, Clone, Default)]
pub struct Screen {
    pub rows: Vec<String>,
    pub joins: Vec<usize>,
    pub colors: Vec<ColorSpan>,
}

pub fn load(text: &str) -> Result<Vec<Fixture>, String> {
    let mut fixtures: Vec<Fixture> = Vec::new();
    let mut in_rows = false;
    for line in text.split('\n') {
        let directive = line.strip_prefix("@@ ").and_then(|rest| {
            let (word, rest) = rest.split_once(' ').unwrap_or((rest, ""));
            ["fixture", "cmd", "+", "item", "xfail"]
                .contains(&word)
                .then_some((word, rest))
        });
        let Some((word, rest)) = directive else {
            if in_rows {
                fixtures
                    .last_mut()
                    .unwrap()
                    .rows
                    .push(line.replace("\\e", "\u{1b}"));
            }
            continue;
        };
        if word != "fixture" && fixtures.is_empty() {
            return Err(format!("corpus: {line} before any fixture"));
        }
        match word {
            "fixture" => {
                let fields: Vec<&str> = rest.split(" | ").collect();
                let columns = fields
                    .get(2)
                    .and_then(|f| f.strip_prefix("cols "))
                    .and_then(|n| n.parse().ok())
                    .filter(|_| fields.len() == 3)
                    .ok_or_else(|| format!("corpus: bad header {line}"))?;
                fixtures.push(Fixture {
                    name: fields[0].to_owned(),
                    category: fields[1].to_owned(),
                    columns,
                    ..Fixture::default()
                });
                in_rows = true;
            }
            "cmd" => {
                in_rows = false;
                fixtures.last_mut().unwrap().commands.push(rest.to_owned());
            }
            "+" => {
                in_rows = false;
                let fixture = fixtures.last_mut().unwrap();
                let last = fixture
                    .commands
                    .last_mut()
                    .ok_or("corpus: @@ + without @@ cmd")?;
                last.push('\n');
                last.push_str(rest);
            }
            "item" => {
                in_rows = false;
                let (kind, value) = rest.split_once(' ').unwrap_or((rest, ""));
                fixtures
                    .last_mut()
                    .unwrap()
                    .items
                    .push((kind.to_owned(), value.to_owned()));
            }
            _ => {
                in_rows = false;
                fixtures.last_mut().unwrap().xfail = Some(rest.to_owned());
            }
        }
    }
    for fixture in &mut fixtures {
        while fixture.rows.last().is_some_and(String::is_empty) {
            fixture.rows.pop();
        }
    }
    Ok(fixtures)
}

impl Fixture {
    pub fn screen(&self) -> Screen {
        if self.rows.iter().any(|r| r.contains('\u{1b}')) {
            let (rows, colors) = styled_rows(&self.rows.join("\r\n"));
            return Screen {
                rows,
                joins: Vec::new(),
                colors,
            };
        }
        let mut screen = Screen::default();
        for row in &self.rows {
            let graphemes: Vec<&str> = row.graphemes(true).collect();
            let mut rest = &graphemes[..];
            while rest.len() > self.columns {
                screen.rows.push(rest[..self.columns].concat());
                screen.joins.push(screen.rows.len() - 1);
                rest = &rest[self.columns..];
            }
            screen.rows.push(rest.concat());
        }
        screen
    }

    /// The detector's items on this fixture, as the scorer runs it: no
    /// working directory, a remote pane, no branch, the classifier off.
    pub fn detect(&self, fs: &dyn Filesystem) -> (Screen, Vec<Item>) {
        let screen = self.screen();
        let rows: Vec<&str> = screen.rows.iter().map(String::as_str).collect();
        let items = detect(
            &Input {
                rows: &rows,
                joins: &screen.joins,
                columns: Some(self.columns),
                colors: &screen.colors,
                ..Input::default()
            },
            fs,
        );
        (screen, items)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Score {
    pub found: usize,
    pub missed: usize,
    pub wrong: usize,
}

impl Score {
    pub fn recall(&self) -> f64 {
        if self.found + self.missed == 0 {
            1.0
        } else {
            self.found as f64 / (self.found + self.missed) as f64
        }
    }

    pub fn precision(&self) -> f64 {
        if self.found + self.wrong == 0 {
            1.0
        } else {
            self.found as f64 / (self.found + self.wrong) as f64
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub scores: BTreeMap<String, Score>,
    pub order: Vec<String>,
    /// Fixtures that missed a command, found an extra one or lost an item.
    pub failures: Vec<(Fixture, Vec<String>)>,
}

impl Report {
    pub fn total(&self) -> Score {
        let mut total = Score::default();
        for s in self.scores.values() {
            total.found += s.found;
            total.missed += s.missed;
            total.wrong += s.wrong;
        }
        total
    }

    pub fn describe(&self, verbose: bool) -> String {
        let line = |name: &str, s: &Score| {
            format!(
                "  {name:<10} recall {:5.1}%  precision {:5.1}%  ({} found, {} missed, {} extra)",
                s.recall() * 100.0,
                s.precision() * 100.0,
                s.found,
                s.missed,
                s.wrong
            )
        };
        let mut lines: Vec<String> = self
            .order
            .iter()
            .map(|c| line(c, &self.scores[c]))
            .collect();
        lines.push(line("all", &self.total()));
        if verbose {
            for (fixture, problems) in &self.failures {
                let xfail = fixture
                    .xfail
                    .as_ref()
                    .map(|x| format!(" (xfail: {x})"))
                    .unwrap_or_default();
                lines.push(format!("  {}{xfail}:", fixture.name));
                lines.extend(problems.iter().map(|p| format!("    {p}")));
            }
        }
        lines.join("\n")
    }
}

/// Scores command recognition: every expected command found (recall) and
/// every command found expected (precision), per category.
pub fn evaluate(fixtures: &[Fixture], fs: &dyn Filesystem) -> Report {
    let mut report = Report::default();
    for fixture in fixtures {
        let (_, items) = fixture.detect(fs);
        let mut found: Vec<&str> = items
            .iter()
            .filter(|i| i.kind == Kind::Command)
            .map(|i| i.value.as_str())
            .collect();
        if !report.scores.contains_key(&fixture.category) {
            report.order.push(fixture.category.clone());
        }
        let score = report.scores.entry(fixture.category.clone()).or_default();
        let mut problems = Vec::new();
        for expected in &fixture.commands {
            if let Some(index) = found.iter().position(|f| f == expected) {
                found.remove(index);
                score.found += 1;
            } else {
                score.missed += 1;
                problems.push(format!("missed: {expected:?}"));
            }
        }
        for extra in found {
            score.wrong += 1;
            problems.push(format!("extra:  {extra:?}"));
        }
        for (kind, value) in &fixture.items {
            if !items
                .iter()
                .any(|i| i.kind.name() == kind && &i.value == value)
            {
                problems.push(format!("lost {kind}: {value}"));
            }
        }
        if !problems.is_empty() {
            report.failures.push((fixture.clone(), problems));
        }
    }
    report
}
