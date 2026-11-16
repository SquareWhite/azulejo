//! ⌘S smart select, everything after recognition: type-prefixed labels,
//! lines, text paragraphs, the containers ⌥↑ grows through, arrow-key
//! neighbours, the text a range copies and the line Run types.
//!
//! Positions are rows and UTF-16 offsets in them, as in [`crate::detect`].

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use fancy_regex::Regex;

use crate::detect::unicode::{graphemes, utf16_len};
use crate::detect::words::{matches, re};
use crate::detect::{Item, Kind};
use crate::refine::Span;
use crate::text::byte_at_utf16;

/// A place in one pane's rows: a row and a UTF-16 offset in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pos {
    pub row: usize,
    pub offset: usize,
}

/// `start` up to `end` (exclusive), across rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextRange {
    pub start: Pos,
    pub end: Pos,
}

impl TextRange {
    pub fn contains(&self, other: &TextRange) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    pub fn strictly_contains(&self, other: &TextRange) -> bool {
        self.contains(other) && self != other
    }

    pub fn union(&self, other: &TextRange) -> TextRange {
        TextRange {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// How far a selection reaches: a recognised item, a text paragraph, its
/// line (the rows it sits on, with the rows they soft-wrap into), its block
/// of non-blank rows or the pane's whole visible text. Arrow keys move at
/// the selection's level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Level {
    Item = 0,
    Text = 1,
    Line = 2,
    Block = 3,
    Screen = 4,
}

impl Level {
    pub fn from_u8(value: u8) -> Option<Level> {
        [
            Level::Item,
            Level::Text,
            Level::Line,
            Level::Block,
            Level::Screen,
        ]
        .get(value as usize)
        .copied()
    }
}

/// One step of a selection: a range, its level and, for an item, a
/// paragraph or a line, its index in the pane's items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unit {
    pub range: TextRange,
    pub level: Level,
    pub entry: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

// MARK: Labels

const ALPHABET: &[u8] = b"asdfghjklqwertyuiopzxcvbnm";

/// One label per item, in the given order: the kind's letter alone when
/// it's the only item of that kind, else the letter and a home-row suffix
/// (`pa`, `ps`, …), one letter longer past 26 of a kind. Labels of one kind
/// share a length, so none is a prefix of another. A kind without a letter
/// (a paragraph) gets an empty label.
pub fn labels(kinds: &[Kind]) -> Vec<String> {
    let mut counts: HashMap<Kind, usize> = HashMap::new();
    for &kind in kinds {
        *counts.entry(kind).or_default() += 1;
    }
    let mut seen: HashMap<Kind, usize> = HashMap::new();
    kinds
        .iter()
        .map(|&kind| {
            let Some(letter) = kind.letter() else {
                return String::new();
            };
            let count = counts[&kind];
            let index = seen.entry(kind).or_default();
            let mut label = letter.to_string();
            if count > 1 {
                label.push_str(&suffix(*index, count));
            }
            *index += 1;
            label
        })
        .collect()
}

fn suffix(index: usize, count: usize) -> String {
    let base = ALPHABET.len();
    let (mut length, mut capacity) = (1, base);
    while capacity < count {
        capacity *= base;
        length += 1;
    }
    let mut chars = vec![ALPHABET[0]; length];
    let mut n = index;
    for i in (0..length).rev() {
        chars[i] = ALPHABET[n % base];
        n /= base;
    }
    String::from_utf8(chars).unwrap()
}

/// Items shown again and again (`keys` in reading order, equal for the
/// same kind and value in the same pane: a prompt's working directory on
/// every prompt line): all but the newest, the lowest on screen. The
/// overlay leaves them unlabelled.
pub fn older_repeats<K: std::hash::Hash + Eq>(keys: &[K]) -> HashSet<usize> {
    let mut newest: HashMap<&K, usize> = HashMap::new();
    for (index, key) in keys.iter().enumerate() {
        newest.insert(key, index);
    }
    (0..keys.len()).filter(|&i| newest[&keys[i]] != i).collect()
}

/// Labels for every item across panes (`(pane, kind, value)` in reading
/// order): repeats in a pane get none (an empty label), nor do paragraphs;
/// the rest share one labelling. Lines aren't repeats: two lines that read
/// the same are still two places, and each gets its own label.
pub fn pane_labels(items: &[(usize, Kind, &str)]) -> Vec<String> {
    let repeats = older_repeats(items);
    let labelled: Vec<usize> = (0..items.len())
        .filter(|&i| {
            let kind = items[i].1;
            kind.letter().is_some() && (kind == Kind::Line || !repeats.contains(&i))
        })
        .collect();
    let kinds: Vec<Kind> = labelled.iter().map(|&i| items[i].1).collect();
    let mut result = vec![String::new(); items.len()];
    for (index, label) in labelled.into_iter().zip(labels(&kinds)) {
        result[index] = label;
    }
    result
}

/// Whether an item's label shows while choosing, with `typed` typed so
/// far: it has a label that starts with `typed`. Lines wait for a first
/// letter (theirs, `l`): boxing every row up front would bury the rest.
pub fn shown(kind: Kind, label: &str, typed: &str) -> bool {
    !label.is_empty() && label.starts_with(typed) && (kind != Kind::Line || !typed.is_empty())
}

// MARK: Paragraphs

static WORD: LazyLock<Regex> =
    LazyLock::new(|| re(r#"^[(\[{"'`“‘]*[A-Za-z][A-Za-z'’-]*[)\]}"'`”’.,;:!?]*$"#));
static LIST_MARKER: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:[-*•] |\d+[.)] )"));
static LABELLED: LazyLock<Regex> = LazyLock::new(|| re(r"^[^:]{1,40}:[ \t]+\S+$"));

fn is_blank(row: &str) -> bool {
    row.chars().all(char::is_whitespace)
}

fn trim(row: &str) -> &str {
    crate::detect::unicode::trim_hspace(row)
}

fn indentation(row: &str) -> usize {
    crate::detect::unicode::indentation(row)
}

/// Text paragraphs: runs of non-blank rows that read as prose. A run
/// starts on a prose row (three or more words, at least half of them plain
/// words, no column gaps, not a "Label: value" pair) and takes the rows
/// below with the same indent while they are prose too, or while the row
/// above fills most of the pane (a wrapped sentence's tail:
/// "(198.51.100.12:8443)."). A list marker starts a new paragraph. Command
/// rows (a command covering half the row, or starting it) and every row
/// below a command in the same block (its output) are never prose.
pub fn paragraphs<S: AsRef<str>>(
    rows: &[S],
    items: &[Item],
    joins: &[usize],
    columns: Option<usize>,
) -> Vec<Item> {
    let rows: Vec<&str> = rows.iter().map(AsRef::as_ref).collect();
    let mut command_rows: HashSet<usize> = HashSet::new();
    for item in items.iter().filter(|i| i.kind == Kind::Command) {
        for span in &item.spans {
            let Some((start, end)) = row_extent(&rows, span.row) else {
                continue;
            };
            let width = (end - start).max(1);
            if span.length * 2 >= width || span.start <= start + 3 {
                command_rows.insert(span.row);
            }
        }
    }
    // Rows in a block below a command row: output.
    let mut output: HashSet<usize> = HashSet::new();
    let mut below = false;
    for (row, text) in rows.iter().enumerate() {
        if is_blank(text) {
            below = false;
            continue;
        }
        if command_rows.contains(&row) {
            below = true;
            continue;
        }
        if below {
            output.insert(row);
        }
    }
    let usable =
        |row: usize| !is_blank(rows[row]) && !command_rows.contains(&row) && !output.contains(&row);
    let mut result = Vec::new();
    let mut row = 0;
    while row < rows.len() {
        if !usable(row) || !prose(rows[row]) {
            row += 1;
            continue;
        }
        let start = row;
        let indent = indentation(rows[row]);
        row += 1;
        while row < rows.len()
            && usable(row)
            && indentation(rows[row]) == indent
            && !list_item(rows[row])
        {
            let full = columns.is_some_and(|c| {
                graphemes(rows[row - 1].trim_end_matches([' ', '\t'])) * 10 >= c * 6
            });
            if !(prose(rows[row]) || full || joins.contains(&(row - 1))) {
                break;
            }
            row += 1;
        }
        let spans: Vec<Span> = (start..row)
            .filter_map(|r| {
                row_extent(&rows, r).map(|(s, e)| Span {
                    row: r,
                    start: s,
                    length: e - s,
                })
            })
            .collect();
        let (Some(first), Some(last)) = (spans.first(), spans.last()) else {
            continue;
        };
        let range = TextRange {
            start: Pos {
                row: first.row,
                offset: first.start,
            },
            end: Pos {
                row: last.row,
                offset: last.start + last.length,
            },
        };
        result.push(Item {
            kind: Kind::Text,
            value: text(&range, &rows, joins),
            spans,
            open: None,
        });
    }
    result
}

/// Reads as prose: three or more words, at least half of them plain words.
pub fn prose(row: &str) -> bool {
    let mut body = trim(row);
    if let Ok(Some(m)) = LIST_MARKER.find(body) {
        body = &body[m.end()..];
    }
    if body.contains("   ") {
        return false;
    }
    // "Session UUID: <uuid>": a label and one value, not a sentence.
    if matches(&LABELLED, body) {
        return false;
    }
    let tokens: Vec<&str> = body
        .split(char::is_whitespace)
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.len() < 3 {
        return false;
    }
    let words = tokens.iter().filter(|t| matches(&WORD, t)).count();
    words >= 2 && words * 2 >= tokens.len()
}

fn list_item(row: &str) -> bool {
    matches(&LIST_MARKER, trim(row))
}

// MARK: Lines

/// A line's rows: `row` and the rows it soft-wraps into (`joins`), as
/// `(top, bottom)`.
fn wrapped_rows(row: usize, rows: usize, joins: &HashSet<usize>) -> (usize, usize) {
    let (mut top, mut bottom) = (row, row);
    while top > 0 && joins.contains(&(top - 1)) {
        top -= 1;
    }
    while bottom + 1 < rows && joins.contains(&bottom) {
        bottom += 1;
    }
    (top, bottom)
}

/// Every non-blank line, in reading order: a row and the rows it
/// soft-wraps into (`joins`: rows whose last cell wraps into the next) make
/// one line.
/// Each spans its text without the indent and trailing blanks (a wrapped
/// row's continuation from its first cell) and copies as a terminal
/// selection of that range would: wrapped rows joined without a newline.
pub fn line_items<S: AsRef<str>>(rows: &[S], joins: &[usize]) -> Vec<Item> {
    let joined: HashSet<usize> = joins.iter().copied().collect();
    let mut result = Vec::new();
    let mut row = 0;
    while row < rows.len() {
        let (top, bottom) = wrapped_rows(row, rows.len(), &joined);
        row = bottom + 1;
        let Some(range) = span(rows, top, bottom) else {
            continue;
        };
        let spans: Vec<Span> = (range.start.row..=range.end.row)
            .filter_map(|r| {
                let (start, end) = row_extent(rows, r)?;
                let start = if r == range.start.row { start } else { 0 };
                let end = if r == range.end.row {
                    range.end.offset
                } else {
                    end
                };
                (end > start).then_some(Span {
                    row: r,
                    start,
                    length: end - start,
                })
            })
            .collect();
        result.push(Item {
            kind: Kind::Line,
            value: text(&range, rows, joins),
            spans,
            open: None,
        });
    }
    result
}

// MARK: Ranges

/// The range an item covers, from its first span's start to its last's end.
pub fn extent(item: &Item) -> TextRange {
    let first = item.spans.first().unwrap();
    let last = item.spans.last().unwrap();
    TextRange {
        start: Pos {
            row: first.row,
            offset: first.start,
        },
        end: Pos {
            row: last.row,
            offset: last.start + last.length,
        },
    }
}

/// A row's text without its indent and trailing blanks, in UTF-16 offsets.
pub fn row_extent<S: AsRef<str>>(rows: &[S], row: usize) -> Option<(usize, usize)> {
    let text = rows.get(row)?.as_ref();
    if is_blank(text) {
        return None;
    }
    let blank = |b: &u8| *b == b' ' || *b == b'\t';
    let start = text.bytes().take_while(blank).count();
    let trailing = text.bytes().rev().take_while(blank).count();
    Some((start, utf16_len(text) - trailing))
}

/// The text a terminal selection of `range` would copy: rows joined with
/// newlines (soft-wrapped rows without one), trailing blanks dropped.
pub fn text<S: AsRef<str>>(range: &TextRange, rows: &[S], joins: &[usize]) -> String {
    let mut out = String::new();
    for row in range.start.row..=range.end.row {
        let Some(line) = rows.get(row).map(AsRef::as_ref) else {
            continue;
        };
        let length = utf16_len(line);
        let from = if row == range.start.row {
            range.start.offset.min(length)
        } else {
            0
        };
        let to = if row == range.end.row {
            range.end.offset.min(length)
        } else {
            length
        };
        let mut piece = if to > from {
            &line[byte_at_utf16(line, from)..byte_at_utf16(line, to)]
        } else {
            ""
        };
        if row != range.end.row {
            if joins.contains(&row) {
                out.push_str(piece);
                continue;
            }
            piece = piece.trim_end_matches([' ', '\t']);
        }
        out.push_str(piece);
        if row != range.end.row {
            out.push('\n');
        }
    }
    out
}

fn span<S: AsRef<str>>(rows: &[S], top: usize, bottom: usize) -> Option<TextRange> {
    let first = (top..=bottom).find(|&r| row_extent(rows, r).is_some())?;
    let last = (top..=bottom)
        .rev()
        .find(|&r| row_extent(rows, r).is_some())?;
    Some(TextRange {
        start: Pos {
            row: first,
            offset: row_extent(rows, first)?.0,
        },
        end: Pos {
            row: last,
            offset: row_extent(rows, last)?.1,
        },
    })
}

/// The rows `range` touches and the rows they soft-wrap with (`joins`),
/// from the first one's indent to the last one's end.
pub fn line<S: AsRef<str>>(range: &TextRange, rows: &[S], joins: &[usize]) -> Option<TextRange> {
    let joined: HashSet<usize> = joins.iter().copied().collect();
    let (top, _) = wrapped_rows(range.start.row, rows.len(), &joined);
    let (_, bottom) = wrapped_rows(range.end.row, rows.len(), &joined);
    span(rows, top, bottom)
}

/// The run of non-blank rows around `range`.
pub fn block<S: AsRef<str>>(range: &TextRange, rows: &[S]) -> Option<TextRange> {
    let (mut top, mut bottom) = (range.start.row, range.end.row);
    while top > 0 && !is_blank(rows[top - 1].as_ref()) {
        top -= 1;
    }
    while bottom + 1 < rows.len() && !is_blank(rows[bottom + 1].as_ref()) {
        bottom += 1;
    }
    span(rows, top, bottom)
}

/// Every non-blank row.
pub fn screen<S: AsRef<str>>(rows: &[S]) -> Option<TextRange> {
    let top = (0..rows.len()).find(|&r| !is_blank(rows[r].as_ref()))?;
    let bottom = (0..rows.len())
        .rev()
        .find(|&r| !is_blank(rows[r].as_ref()))?;
    span(rows, top, bottom)
}

/// Each non-blank line, soft wraps joined ([`line_items`]' ranges).
pub fn lines<S: AsRef<str>>(rows: &[S], joins: &[usize]) -> Vec<TextRange> {
    line_items(rows, joins).iter().map(extent).collect()
}

/// Each run of non-blank rows (block-level arrow targets).
pub fn blocks<S: AsRef<str>>(rows: &[S]) -> Vec<TextRange> {
    let mut result = Vec::new();
    let mut row = 0;
    while row < rows.len() {
        if is_blank(rows[row].as_ref()) {
            row += 1;
            continue;
        }
        let top = row;
        while row + 1 < rows.len() && !is_blank(rows[row + 1].as_ref()) {
            row += 1;
        }
        if let Some(range) = span(rows, top, row) {
            result.push(range);
        }
        row += 1;
    }
    result
}

/// A row's runs of non-blank characters, as UTF-16 `(start, length)`.
pub fn text_runs(row: &str) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut start: Option<usize> = None;
    let mut offset = 0;
    for c in row.chars() {
        let blank = c == ' ' || c == '\t' || c == '\u{a0}';
        if blank && let Some(s) = start.take() {
            runs.push((s, offset - s));
        }
        if !blank && start.is_none() {
            start = Some(offset);
        }
        offset += c.len_utf16();
    }
    if let Some(s) = start {
        runs.push((s, offset - s));
    }
    runs
}

// MARK: Grow, shrink, move

fn size(range: &TextRange) -> i64 {
    (range.end.row as i64 - range.start.row as i64) * 100_000 + range.end.offset as i64
        - range.start.offset as i64
}

/// The level a selection of `item` sits at.
pub fn level_of(item: &Item) -> Level {
    match item.kind {
        Kind::Text => Level::Text,
        Kind::Line => Level::Line,
        _ => Level::Item,
    }
}

/// ⌥↑: the next thing out that holds `range`, in this order: the smallest
/// recognised item around it (a path → its command), its line (the rows it
/// sits on, soft wraps joined; the line item when there is one), the
/// smallest paragraph around it, its block, the screen.
pub fn grow<S: AsRef<str>>(
    range: &TextRange,
    items: &[Item],
    rows: &[S],
    joins: &[usize],
) -> Option<Unit> {
    let smallest = |level: Level| -> Option<Unit> {
        (0..items.len())
            .filter(|&i| level_of(&items[i]) == level && extent(&items[i]).strictly_contains(range))
            .min_by_key(|&i| size(&extent(&items[i])))
            .map(|i| Unit {
                range: extent(&items[i]),
                level,
                entry: Some(i),
            })
    };
    let steps: [&dyn Fn() -> Option<Unit>; 5] = [
        &|| smallest(Level::Item),
        &|| {
            line(range, rows, joins).map(|r| Unit {
                range: r,
                level: Level::Line,
                entry: (0..items.len())
                    .find(|&i| items[i].kind == Kind::Line && extent(&items[i]) == r),
            })
        },
        &|| smallest(Level::Text),
        &|| {
            block(range, rows).map(|r| Unit {
                range: r,
                level: Level::Block,
                entry: None,
            })
        },
        &|| {
            screen(rows).map(|r| Unit {
                range: r,
                level: Level::Screen,
                entry: None,
            })
        },
    ];
    steps
        .iter()
        .filter_map(|step| step())
        .find(|unit| unit.range.strictly_contains(range))
}

/// ⌥↓ with nothing to go back to: the first item inside `range`
/// (recognised items, then paragraphs, then lines), in reading order. From
/// a line, block or screen that is exactly one item, that item; from a
/// block or screen that is exactly one line, that line.
pub fn shrink(range: &TextRange, items: &[Item], from: Level) -> Option<Unit> {
    let inside = |i: usize| {
        let e = extent(&items[i]);
        let exact = match items[i].kind {
            Kind::Line => from == Level::Block || from == Level::Screen,
            _ => from != Level::Item && from != Level::Text,
        };
        if exact {
            range.contains(&e)
        } else {
            range.strictly_contains(&e)
        }
    };
    let first = |level: Level| {
        (0..items.len())
            .filter(|&i| level_of(&items[i]) == level && inside(i))
            .min_by_key(|&i| extent(&items[i]).start)
    };
    let pick = first(Level::Item)
        .or_else(|| first(Level::Text))
        .or_else(|| first(Level::Line))?;
    Some(Unit {
        range: extent(&items[pick]),
        level: level_of(&items[pick]),
        entry: Some(pick),
    })
}

/// The neighbour of `current` among `targets`: ← → in reading order (an
/// item before the items inside it), ↑ ↓ on the nearest row above or below
/// with a target, the one closest to `current`'s first column.
pub fn neighbour(
    targets: &[TextRange],
    current: &TextRange,
    direction: Direction,
) -> Option<usize> {
    let before = |a: &TextRange, b: &TextRange| {
        if a.start != b.start {
            a.start < b.start
        } else {
            a.end > b.end
        }
    };
    match direction {
        Direction::Left | Direction::Right => {
            let mut order: Vec<usize> = (0..targets.len()).collect();
            order.sort_by(|&a, &b| {
                let (x, y) = (&targets[a], &targets[b]);
                x.start.cmp(&y.start).then(y.end.cmp(&x.end))
            });
            if let Some(here) = order.iter().position(|&i| targets[i] == *current) {
                let next = if direction == Direction::Right {
                    here.checked_add(1)
                } else {
                    here.checked_sub(1)
                };
                return next.and_then(|n| order.get(n).copied());
            }
            if direction == Direction::Right {
                order.into_iter().find(|&i| before(current, &targets[i]))
            } else {
                order
                    .into_iter()
                    .rev()
                    .find(|&i| before(&targets[i], current))
            }
        }
        Direction::Up | Direction::Down => {
            let candidates: Vec<usize> = (0..targets.len())
                .filter(|&i| {
                    if direction == Direction::Down {
                        targets[i].start.row > current.end.row
                    } else {
                        targets[i].start.row < current.start.row && targets[i].end <= current.start
                    }
                })
                .collect();
            let rows = candidates.iter().map(|&i| targets[i].start.row);
            let row = if direction == Direction::Down {
                rows.min()
            } else {
                rows.max()
            }?;
            candidates
                .into_iter()
                .filter(|&i| targets[i].start.row == row)
                .min_by_key(|&i| {
                    (targets[i].start.offset as i64 - current.start.offset as i64).abs()
                })
        }
    }
}

// MARK: Choosing

/// An arrow key while choosing, before anything is selected: where the
/// focus ring goes among `targets` (the items whose labels still show).
/// From `focus` it moves as [`neighbour`] does. With nothing focused yet it
/// starts at the terminal cursor, where the latest output usually ends:
/// ↑ the closest target on the rows above it, ↓ the first one below it or
/// else the last one, ← the last one before it, → the first one after it or
/// else the last one. Nothing above or before the cursor: the nearest
/// target ([`nearest`]).
pub fn focus_step(
    targets: &[TextRange],
    focus: Option<&TextRange>,
    cursor: Pos,
    direction: Direction,
) -> Option<usize> {
    if let Some(focus) = focus {
        return neighbour(targets, focus, direction);
    }
    let here = TextRange {
        start: cursor,
        end: cursor,
    };
    // The last in reading order (an item after the paragraph round it).
    let last = || (0..targets.len()).max_by_key(|&i| (targets[i].start, Reverse(targets[i].end)));
    match direction {
        Direction::Up | Direction::Left => neighbour(targets, &here, direction),
        Direction::Down | Direction::Right => neighbour(targets, &here, direction).or_else(last),
    }
    .or_else(|| nearest(targets, &here))
}

/// The target closest to `to`: fewest rows apart (0 when they share one),
/// then fewest columns between their starts, then the first in reading
/// order. Where the focus goes when typing filters out the focused item.
pub fn nearest(targets: &[TextRange], to: &TextRange) -> Option<usize> {
    let rows_apart = |t: &TextRange| {
        if t.end.row < to.start.row {
            to.start.row - t.end.row
        } else {
            t.start.row.saturating_sub(to.end.row)
        }
    };
    (0..targets.len()).min_by_key(|&i| {
        let t = &targets[i];
        (
            rows_apart(t),
            t.start.offset.abs_diff(to.start.offset),
            t.start,
            Reverse(t.end),
        )
    })
}

// MARK: Run

/// The line Run types into the new pane's login shell. A single-line
/// command goes as it is, in the user's own shell. A multi-line one (a
/// heredoc, a `\` continuation from a coding agent's tool call) runs
/// through `/bin/sh`: typed line by line it breaks in fish (no `<<`), and
/// tabs typed into a shell complete. The value travels base64 encoded
/// inside single quotes, a line every shell (POSIX or fish) parses the same
/// way, whatever quotes or backslashes it holds; `eval` keeps the terminal
/// as its standard input.
pub fn run_line(value: &str) -> String {
    if !value.contains(['\n', '\r']) {
        return value.to_owned();
    }
    let encoded = crate::model::base64_encode(value.as_bytes());
    format!("/bin/sh -c 'eval \"$(printf %s {encoded} | base64 -d)\"'")
}

// MARK: A pane

/// One pane's rows and items, for the selection keys after ⌘S.
#[derive(Debug, Clone, Default)]
pub struct Pane {
    pub rows: Vec<String>,
    pub joins: Vec<usize>,
    /// Detected items, paragraphs and lines, in reading order (a line or a
    /// paragraph before the items inside it).
    pub items: Vec<Item>,
}

impl Pane {
    /// The detected items, the paragraphs ([`paragraphs`]) and the lines
    /// ([`line_items`]) in reading order, a container before the items
    /// inside it.
    pub fn new(
        rows: Vec<String>,
        joins: Vec<usize>,
        detected: Vec<Item>,
        columns: Option<usize>,
    ) -> Pane {
        let found = paragraphs(&rows, &detected, &joins, columns);
        let lines = line_items(&rows, &joins);
        let mut all: Vec<(usize, Item)> = detected
            .into_iter()
            .chain(found)
            .chain(lines)
            .enumerate()
            .collect();
        all.sort_by(|(i, a), (j, b)| {
            let (x, y) = (extent(a), extent(b));
            x.start.cmp(&y.start).then(y.end.cmp(&x.end)).then(i.cmp(j))
        });
        Pane {
            rows,
            joins,
            items: all.into_iter().map(|(_, item)| item).collect(),
        }
    }

    /// Keeps only these items (the shell couldn't place the rest).
    pub fn keep(&mut self, indices: &[usize]) {
        let keep: HashSet<usize> = indices.iter().copied().collect();
        let mut index = 0;
        self.items.retain(|_| {
            index += 1;
            keep.contains(&(index - 1))
        });
    }

    fn unit(&self, index: usize) -> Unit {
        Unit {
            range: extent(&self.items[index]),
            level: level_of(&self.items[index]),
            entry: Some(index),
        }
    }

    /// Where arrow keys go at `level`: the recognised items, paragraphs or
    /// lines; the blocks.
    pub fn targets(&self, level: Level) -> Vec<Unit> {
        match level {
            Level::Item | Level::Text | Level::Line => (0..self.items.len())
                .filter(|&i| level_of(&self.items[i]) == level)
                .map(|i| self.unit(i))
                .collect(),
            Level::Block => blocks(&self.rows)
                .into_iter()
                .map(|r| Unit {
                    range: r,
                    level,
                    entry: None,
                })
                .collect(),
            Level::Screen => Vec::new(),
        }
    }

    /// The target an arrow key moves `from` to, at `from`'s level.
    pub fn neighbour(&self, from: &Unit, direction: Direction) -> Option<Unit> {
        let targets = self.targets(from.level);
        let ranges: Vec<TextRange> = targets.iter().map(|u| u.range).collect();
        neighbour(&ranges, &from.range, direction).map(|i| targets[i])
    }

    /// The ranges of `shown` (indices into `items`; out-of-range ones
    /// dropped), with their indices.
    fn shown(&self, shown: &[usize]) -> (Vec<usize>, Vec<TextRange>) {
        let kept: Vec<usize> = shown
            .iter()
            .copied()
            .filter(|&i| i < self.items.len())
            .collect();
        let ranges = kept.iter().map(|&i| extent(&self.items[i])).collect();
        (kept, ranges)
    }

    /// An arrow key while choosing ([`focus_step`]): the item the focus
    /// ring moves to among `shown`, from item `focus`, or with none from
    /// `cursor` (below the last row when the cursor is unknown).
    pub fn focus_step(
        &self,
        shown: &[usize],
        focus: Option<usize>,
        cursor: Option<Pos>,
        direction: Direction,
    ) -> Option<usize> {
        let (kept, ranges) = self.shown(shown);
        let focus = focus
            .filter(|&i| i < self.items.len())
            .map(|i| extent(&self.items[i]));
        let cursor = cursor.unwrap_or(Pos {
            row: self.rows.len(),
            offset: 0,
        });
        focus_step(&ranges, focus.as_ref(), cursor, direction).map(|i| kept[i])
    }

    /// The item among `shown` nearest item `to` ([`nearest`]).
    pub fn nearest(&self, shown: &[usize], to: usize) -> Option<usize> {
        let to = extent(self.items.get(to)?);
        let (kept, ranges) = self.shown(shown);
        nearest(&ranges, &to).map(|i| kept[i])
    }

    pub fn grow(&self, range: &TextRange) -> Option<Unit> {
        grow(range, &self.items, &self.rows, &self.joins)
    }

    pub fn shrink(&self, range: &TextRange, from: Level) -> Option<Unit> {
        shrink(range, &self.items, from)
    }

    pub fn text(&self, range: &TextRange) -> String {
        text(range, &self.rows, &self.joins)
    }
}
