//! ⌘S "smart select" detection: the items a person might want from a
//! terminal screen (URLs, paths, commands, IPs, domains, hashes and IDs,
//! branches), found in the visible rows.
//!
//! Pure text logic, shared by every shell: rows in, items out. Positions
//! cross the API in UTF-16 code units (the macOS shell's string offsets,
//! which it maps to cells through the terminal); inside, the detector works
//! in bytes. The one platform question, whether a path exists, goes through
//! [`Filesystem`].
//!
//! The pipeline: rows join into logical lines ([`lines`]: soft wraps, hard
//! wraps an application made, shell continuations, coding agents' tool
//! calls), each line's command is found while joining, then [`scan`] finds
//! inline commands and atoms per line, and spans map back to the physical
//! rows. With the row classifier on ([`Input::row_classifier`]), its labels
//! refine the commands ([`crate::refine`]).

pub mod files;
pub mod fixtures;
pub mod lines;
pub mod scan;
pub mod shell;
pub mod styled;
pub mod unicode;
pub mod words;

use std::collections::HashMap;
use std::ops::Range;

use serde::{Deserialize, Serialize};

pub use files::{Filesystem, StdFilesystem};
pub use styled::{ColorSpan, matching_colors, styled_rows};

use crate::Model;
use crate::refine::{Command, Span, refine};
use files::Files;
use lines::{Line, RowColors, join_lines, runnable, single_lines, soft_lines};

/// What an item is: a ⌘S category, picked by its letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Url,
    Path,
    Branch,
    Command,
    Ip,
    Domain,
    /// A hash or an ID: a git hash, a UUID, another hex ID.
    Hash,
    /// A paragraph of prose ([`crate::select::paragraphs`]): what ⌥↑
    /// grows a line to. Never labelled.
    Text,
    /// A non-blank line, soft wraps joined ([`crate::select::line_items`]).
    /// Labelled like the rest, so its category shows every line.
    Line,
}

impl Kind {
    pub const ALL: [Kind; 9] = [
        Kind::Url,
        Kind::Path,
        Kind::Branch,
        Kind::Command,
        Kind::Ip,
        Kind::Domain,
        Kind::Hash,
        Kind::Text,
        Kind::Line,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Url => "url",
            Kind::Path => "path",
            Kind::Branch => "branch",
            Kind::Command => "command",
            Kind::Ip => "ip",
            Kind::Domain => "domain",
            Kind::Hash => "hash",
            Kind::Text => "text",
            Kind::Line => "line",
        }
    }

    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// The key that picks this kind's category in the ⌘S panel; none for a
    /// paragraph, which has no label.
    pub fn letter(self) -> Option<char> {
        Some(match self {
            Kind::Url => 'u',
            Kind::Path => 'p',
            Kind::Ip => 'i',
            Kind::Hash => 'h',
            Kind::Branch => 'b',
            Kind::Domain => 'd',
            Kind::Command => 'c',
            Kind::Line => 'l',
            Kind::Text => return None,
        })
    }

    /// Which wins when atoms of equal length overlap.
    fn priority(self) -> u8 {
        match self {
            Kind::Branch => 0,
            Kind::Url => 1,
            Kind::Path => 2,
            Kind::Ip => 3,
            Kind::Hash => 4,
            Kind::Domain => 5,
            _ => 9,
        }
    }
}

/// A recognised item: its value (a command as it would run, a path
/// unquoted) and where it sits, one span per physical row, in UTF-16 units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    pub value: String,
    pub spans: Vec<Span>,
    /// What "open" hands the platform: an http(s), mailto or file URL.
    pub open: Option<String>,
}

/// One screen to read.
#[derive(Debug, Clone, Default)]
pub struct Input<'a> {
    /// The visible rows, as the terminal reads them.
    pub rows: &'a [&'a str],
    /// Rows whose last cell soft-wraps into the next row.
    pub joins: &'a [usize],
    /// The pane width in cells: rows an application wrapped itself join
    /// only when it is known.
    pub columns: Option<usize>,
    /// The shell's working directory, for relative paths.
    pub cwd: Option<&'a str>,
    /// The pane runs on this machine: paths may open.
    pub local: bool,
    /// The repository's current branch.
    pub branch: Option<&'a str>,
    /// Colour runs, in bytes of each row ([`styled_rows`]).
    pub colors: &'a [ColorSpan],
    /// Refine commands with the row classifier (`[hints] row_classifier`).
    pub row_classifier: bool,
}

/// Every item on a screen, in reading order (row, then start, then kind).
/// A command stays selectable next to the items inside it; other items
/// never overlap each other.
pub fn detect(input: &Input, fs: &dyn Filesystem) -> Vec<Item> {
    let colors = by_row(input.rows, input.colors);
    let mut joined = vec![false; input.rows.len()];
    for &row in input.joins {
        if row < joined.len() {
            joined[row] = true;
        }
    }
    let soft = soft_lines(input.rows, &joined);
    let lines = join_lines(&soft, input.columns, &colors);
    let mut files = Files::new(fs, input.cwd, input.local);
    let found = items(input.rows, &lines, input.branch, &colors, &mut files);
    if input.row_classifier {
        classify(input, found)
    } else {
        found
    }
}

/// One row per line, without joining rows (soft wraps, application wraps,
/// continuations): the boundary checks' view.
pub fn detect_rows(input: &Input, fs: &dyn Filesystem) -> Vec<Item> {
    let colors = by_row(input.rows, input.colors);
    let lines = single_lines(input.rows, &colors);
    let mut files = Files::new(fs, input.cwd, input.local);
    items(input.rows, &lines, input.branch, &colors, &mut files)
}

/// Colour runs by row. A row in one colour from its first character to
/// its last keeps none: that colour is the application's text colour (a
/// coding agent paints its prose and the commands in it alike), not a mark
/// on part of the row, so the row reads as plain text. Unless it carries on
/// a run that a neighbouring row of several colours ends or starts with:
/// then it's the middle of a coloured item wrapped over rows.
fn by_row(rows: &[&str], colors: &[ColorSpan]) -> RowColors {
    let mut map: RowColors = HashMap::new();
    for c in colors {
        map.entry(c.row)
            .or_default()
            .push((c.start..c.end, c.color));
    }
    let painted: HashMap<usize, u32> = map
        .iter()
        .filter(|(row, runs)| rows.get(**row).is_some_and(|text| uniform(text, runs)))
        .map(|(row, runs)| (*row, runs[0].1))
        .collect();
    let touches = |row: usize, color: u32, end: bool| -> bool {
        let (Some(text), Some(runs)) = (rows.get(row), map.get(&row)) else {
            return false;
        };
        if painted.contains_key(&row) {
            return false;
        }
        if end {
            let edge = text.trim_end().len();
            runs.iter()
                .any(|r| r.1 == color && r.0.end >= edge && r.0.start < edge)
        } else {
            let edge = text.len() - text.trim_start().len();
            runs.iter().any(|r| r.1 == color && r.0.contains(&edge))
        }
    };
    let dropped: Vec<usize> = painted
        .iter()
        .filter(|&(&row, &color)| {
            !(row > 0 && touches(row - 1, color, true)) && !touches(row + 1, color, false)
        })
        .map(|(&row, _)| row)
        .collect();
    for row in dropped {
        map.remove(&row);
    }
    map
}

/// Whether one colour covers every non-blank character of `text`.
fn uniform(text: &str, runs: &[(Range<usize>, u32)]) -> bool {
    let Some(color) = runs.first().map(|r| r.1) else {
        return false;
    };
    if runs.iter().any(|r| r.1 != color) {
        return false;
    }
    let mut any = false;
    for (offset, c) in text.char_indices() {
        if c.is_whitespace() {
            continue;
        }
        any = true;
        if !runs.iter().any(|r| r.0.contains(&offset)) {
            return false;
        }
    }
    any
}

/// The reading order: row, start, then kind name.
fn reading_order(items: &mut [Item]) {
    items.sort_by(|a, b| {
        let (x, y) = (a.spans[0], b.spans[0]);
        x.row
            .cmp(&y.row)
            .then(x.start.cmp(&y.start))
            .then(a.kind.name().cmp(b.kind.name()))
    });
}

fn items(
    rows: &[&str],
    lines: &[Line],
    branch: Option<&str>,
    by_row: &RowColors,
    files: &mut Files,
) -> Vec<Item> {
    // Each line's colour runs; a hard wrap's separator doesn't break one.
    let logical: Vec<Vec<Range<usize>>> = lines
        .iter()
        .map(|line| {
            let mut merged: Vec<(Range<usize>, u32)> = Vec::new();
            for part in &line.parts {
                for (range, color) in by_row.get(&part.row).map_or(&[][..], Vec::as_slice) {
                    let start = range.start.max(part.row_start);
                    let end = range.end.min(part.row_start + part.length);
                    if start >= end {
                        continue;
                    }
                    let clipped =
                        part.start + start - part.row_start..part.start + end - part.row_start;
                    if let Some(last) = merged.last_mut()
                        && last.1 == *color
                        && (last.0.end == clipped.start
                            || (last.0.end + 1 == clipped.start
                                && line.inserted.contains(&last.0.end)))
                    {
                        last.0.end = clipped.end;
                    } else {
                        merged.push((clipped, *color));
                    }
                }
            }
            merged.into_iter().map(|(r, _)| r).collect()
        })
        .collect();
    let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    let commands: Vec<Option<Range<usize>>> = lines.iter().map(|l| l.command.clone()).collect();
    let found = scan::scan(&texts, &commands, &logical, branch, files);
    let mut result: Vec<Item> = found
        .into_iter()
        .map(|f| {
            let line = &lines[f.line];
            let spans = line
                .parts
                .iter()
                .filter_map(|part| {
                    let start = f.range.start.max(part.start);
                    let end = f.range.end.min(part.start + part.length);
                    if start >= end {
                        return None;
                    }
                    let row = rows[part.row];
                    let from = start - part.start + part.row_start;
                    let to = end - part.start + part.row_start;
                    let utf16_start = unicode::utf16_at(row, from);
                    Some(Span {
                        row: part.row,
                        start: utf16_start,
                        length: unicode::utf16_at(row, to) - utf16_start,
                    })
                })
                .collect();
            let mut value = f.value;
            if f.kind == Kind::Command
                && (line.dropped.iter().any(|o| f.range.contains(o))
                    || line.newlines.iter().any(|o| f.range.contains(o)))
            {
                value = String::from_utf8(runnable(line, f.range.clone()).0).unwrap_or(value);
            }
            Item {
                kind: f.kind,
                value,
                spans,
                open: f.open,
            }
        })
        .collect();
    reading_order(&mut result);
    result
}

/// The row classifier's pass over the commands found: it labels each row
/// and, where sure, drops a command that fills an output or chrome row and
/// adds ones on command rows the detector missed. Atoms pass through.
fn classify(input: &Input, items: Vec<Item>) -> Vec<Item> {
    let rows = input.rows;
    let mut joins: Vec<usize> = input.joins.to_vec();
    joins.sort_unstable();
    joins.dedup();
    let commands: Vec<Command> = items
        .iter()
        .filter(|i| i.kind == Kind::Command)
        .map(|i| Command {
            value: i.value.clone(),
            spans: i.spans.clone(),
        })
        .collect();
    let labelling = Model::embedded().label(rows, &joins, input.columns);
    let (refined, _) = refine(rows, &joins, input.columns, &labelling, &commands);
    let mut result: Vec<Item> = items
        .into_iter()
        .filter(|i| i.kind != Kind::Command)
        .collect();
    for command in refined {
        let valid = !command.spans.is_empty()
            && command.spans.iter().all(|s| {
                s.row < rows.len() && s.start + s.length <= unicode::utf16_len(rows[s.row])
            });
        if valid {
            result.push(Item {
                kind: Kind::Command,
                value: command.value,
                spans: command.spans,
                open: None,
            });
        }
    }
    reading_order(&mut result);
    result
}
