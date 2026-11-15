//! Physical rows into logical lines, and where commands start on them.
//!
//! Terminal soft wraps join first (the terminal's wrap flag); then rows an
//! application wrapped itself (newline plus repeated indent), shell
//! continuations (`\`, `|`, an open quote, heredoc or bracket) and a coding
//! agent's tool calls join into one line per command. Every line keeps
//! where each of its slices sits on screen ([`Part`]).

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::sync::LazyLock;

use fancy_regex::Regex;

use super::shell::{ShellState, lex};
use super::unicode::{
    cells, char_at, char_before, indentation, is_blank, is_hspace, leading_ws, trim_hspace, words,
};
use super::words::{
    Context, DIFF_LINE, SHELL_FENCES, accepts, matches, matches_graphemes, path_executable,
    prose_commands, re, tier, tokens,
};

/// Text wrappers (coding agents, `fmt`) may stop a little short of the
/// pane's last column. Measured in cells.
pub const WRAP_TOLERANCE: i64 = 2;

/// Indent plus a secondary prompt (`> `, zsh `pipe> `, `dquote> `).
static PS2: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^[ \t]*(?:(?:(?:pipe|cmdand|cmdor|d?quote|bquote|heredoc|subsh|cursh) ?)*> )?")
});
static LIST_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| re(r"^[ \t]*(?:(?:[-*•●⏺] |\d+[.)] |╰─ )(?:\[[ xX]\] )?)?"));
/// A coding agent's shell tool call (`● Bash(…)`).
pub(crate) static TOOL_CALL: LazyLock<Regex> = LazyLock::new(|| re(r"^[ \t]*[●⏺][ \t]+Bash\("));
/// Claude Code's running or finished shell command: `⎿  $ cmd` (the
/// elbow may draw as `└`), wrapped rows indented under the `$`.
pub(crate) static SHOWN_CALL: LazyLock<Regex> =
    LazyLock::new(|| re(r"^[ \t]*[⎿└][ \t]+\$[ \t]+(?=\S)"));
/// What follows a shown command: its elapsed time, `(4m 29s)`.
static ELAPSED: LazyLock<Regex> =
    LazyLock::new(|| re(r"[ \t]+\((?:\d+h[ \t]*)?(?:\d+m[ \t]*)?(?:\d+(?:\.\d+)?s)?\)[ \t]*$"));
/// A key hint under a shown command: `(ctrl+b to run in background)`.
static KEY_HINT: LazyLock<Regex> =
    LazyLock::new(|| re(r"^[ \t]*\((?:ctrl|esc|shift|alt|option|cmd)\+"));
/// A run line (`• Ran …`).
static RAN_CALL: LazyLock<Regex> =
    LazyLock::new(|| re(r"^[ \t]*[•●⏺][ \t]+(?:Ran|Running)[ \t]+(?=\S)"));
/// An agent's shell escape, `!cmd`, also echoed as `> !cmd`.
static AGENT_PROMPT: LazyLock<Regex> =
    LazyLock::new(|| re(r"^[ \t]*(?:>[ \t]?)?![ \t]*([A-Za-z_./~][^\r\n]*?)\s*$"));
/// npm's `> package@version script` banner above the script it runs.
static NPM_BANNER: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*> \S+@\S+ \S"));
static SHELL_VERSION: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Za-z]+-\d+(?:\.\d+)*$"));
static NUMBERED: LazyLock<Regex> = LazyLock::new(|| re(r"^\d+[.)]$"));
static CMD_EXE: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Za-z]:\\[^\s]*$"));
static PROMPT_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Za-z][\w.-]*$"));
static SEGMENT: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:git|hg|svn):\("));

/// A slice of one physical row placed at `start` in a logical line, in bytes.
#[derive(Debug, Clone, Copy)]
pub struct Part {
    pub row: usize,
    pub row_start: usize,
    pub start: usize,
    pub length: usize,
}

/// Physical rows joined into one logical line. `inserted` holds the
/// separator spaces of hard wraps (no cell behind them); `dropped` holds
/// offsets left out of a command's value (line-continuation backslashes);
/// `newlines` holds separators that are line breaks in a command's value.
#[derive(Debug, Clone, Default)]
pub struct Line {
    pub text: String,
    pub parts: Vec<Part>,
    pub inserted: BTreeSet<usize>,
    pub dropped: BTreeSet<usize>,
    pub newlines: BTreeSet<usize>,
    /// Cells between a box's borders, when the row had them.
    pub limit: Option<usize>,
    /// The command on this line, decided while joining rows.
    pub command: Option<Range<usize>>,
}

impl Line {
    pub fn new(row: usize, text: &str) -> Line {
        Line {
            text: text.to_owned(),
            parts: vec![Part {
                row,
                row_start: 0,
                start: 0,
                length: text.len(),
            }],
            ..Line::default()
        }
    }

    fn len(&self) -> usize {
        self.text.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Join {
    Cut,
    Wrap,
    Explicit,
    Quote,
    Newline,
}

/// How a command's later rows continue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Typed at a shell prompt or an agent's `!`: continuation rows
    /// may carry a secondary prompt such as `> `.
    Prompt,
    /// A coding agent's tool call, `● Bash(…)`: runs to the wrapper's
    /// closing parenthesis; rows keep their line breaks.
    Wrapper,
    /// Displayed source (a code block, a permission box, a plain line):
    /// rows keep their line breaks.
    Block,
    /// A coding agent's display of a command it runs (`⎿  $ cmd`):
    /// indented rows are wraps, cut words rejoin without a space.
    Shown,
}

/// Where a command starts on a line and how its later rows continue.
#[derive(Debug, Clone, Copy)]
struct CommandStart {
    location: usize,
    mode: Mode,
    /// A fixed end: a right-hand prompt or a table cell's border.
    end: Option<usize>,
    /// The command in a permission box, above its description.
    header: bool,
}

impl CommandStart {
    fn new(location: usize, mode: Mode) -> CommandStart {
        CommandStart {
            location,
            mode,
            end: None,
            header: false,
        }
    }
}

const BORDERS: [char; 3] = ['│', '┃', '║'];

/// A row inside a box (`│ … │`) is read without its borders: an agent's
/// input box, a permission prompt, a table.
pub fn unframe(line: &mut Line) {
    if line.parts.len() != 1 {
        return;
    }
    let text = &line.text;
    let b = text.as_bytes();
    let mut start = 0;
    while start < b.len() && b[start] == b' ' {
        start += 1;
    }
    let Some(border) = char_at(text, start).filter(|c| BORDERS.contains(c)) else {
        return;
    };
    start += border.len_utf8();
    if start < b.len() && b[start] == b' ' {
        start += 1;
    }
    let mut end = b.len();
    while end > start && b[end - 1] == b' ' {
        end -= 1;
    }
    let mut limit = None;
    if end > start
        && let Some(c) = char_before(text, end).filter(|c| BORDERS.contains(c))
    {
        end -= c.len_utf8();
        limit = Some(cells(&text[start..end]));
    }
    let end = end.max(start);
    if limit.is_some() {
        line.limit = limit;
    }
    line.text = text[start..end].to_owned();
    line.parts[0].row_start = start;
    line.parts[0].length = end - start;
}

/// Rows grouped by the terminal's soft wraps (`joins`: rows whose last
/// cell continues at column zero of the next).
pub fn soft_lines(rows: &[&str], joins: &[bool]) -> Vec<Line> {
    let mut soft: Vec<Line> = Vec::new();
    for (row, text) in rows.iter().enumerate() {
        if row > 0 && joins.get(row - 1).copied().unwrap_or(false) && !soft.is_empty() {
            let last = soft.last_mut().unwrap();
            let start = last.len();
            last.parts.push(Part {
                row,
                row_start: 0,
                start,
                length: text.len(),
            });
            last.text.push_str(text);
        } else {
            soft.push(Line::new(row, text));
        }
    }
    for line in &mut soft {
        unframe(line);
    }
    soft
}

/// Colour runs by physical row, in bytes of that row.
pub type RowColors = HashMap<usize, Vec<(Range<usize>, u32)>>;

/// Joins soft lines into logical lines and finds each line's command.
pub fn join_lines(soft: &[Line], columns: Option<usize>, colors: &RowColors) -> Vec<Line> {
    let mut result: Vec<Line> = Vec::new();
    let mut index = 0;
    // Inside ``` fences: true for a shell language.
    let mut fence: Option<bool> = None;
    // A permission box's "Bash command" title sits above its command.
    let mut header = false;
    // A coding agent's transcript, where `❯` starts the user's messages.
    let agent = soft.iter().any(|line| {
        line.text
            .chars()
            .find(|&c| c != ' ' && c != '\t')
            .is_some_and(|c| "●⏺⎿".contains(c))
    });
    while index < soft.len() {
        let mut line = soft[index].clone();
        let trimmed = trim_hspace(&line.text);
        if let Some(language) = trimmed.strip_prefix("```") {
            fence = match fence {
                None => Some(SHELL_FENCES.contains(&language.to_lowercase().as_str())),
                Some(_) => None,
            };
            result.push(line);
            index += 1;
            continue;
        }
        if trimmed == "Bash command" {
            header = true;
            result.push(line);
            index += 1;
            continue;
        }
        let start = if header && !trimmed.is_empty() {
            header = false;
            source_start(&line.text, true)
        } else {
            command_start(
                &line.text,
                line.parts.iter().any(|p| colors.contains_key(&p.row)),
                keyword(&line, colors),
                fence == Some(true),
                (index > 0).then(|| soft[index - 1].text.as_str()),
                agent,
            )
        };
        let mut last = soft[index].clone();
        let mut joined = 0;
        if let Some(start) = start {
            let base = if start.mode == Mode::Wrapper {
                wrapper_indent(soft, index)
            } else {
                indentation(&line.text)
            };
            let mut state = shell_state(&line, &start);
            let mut checkpoint = (line.clone(), index);
            // A screenful is the natural bound; the cap guards odd input.
            while index + 1 < soft.len()
                && joined < 64
                && start.end.is_none()
                && (start.mode != Mode::Wrapper || state.close.is_none())
            {
                let next = &soft[index + 1];
                let Some(join) = continuation(
                    &line,
                    &last,
                    next,
                    &soft[index + 2..],
                    &start,
                    &state,
                    columns,
                ) else {
                    break;
                };
                let skip = strip(next, join, &start, base);
                append(next, &mut line, join, skip);
                last = next.clone();
                index += 1;
                joined += 1;
                state = shell_state(&line, &start);
                if !state.open() {
                    checkpoint = (line.clone(), index);
                }
            }
            // Rows joined only to close a quote, heredoc or bracket are
            // undone if it never closes, so a stray apostrophe can't
            // swallow the output below. A tool call's rows are framed.
            if start.mode != Mode::Wrapper && state.open() {
                (line, index) = checkpoint;
            }
            line.command = command_range(&line, &start);
        } else {
            while index + 1 < soft.len() && joined < 64 {
                let next = &soft[index + 1];
                if is_blank(&next.text) || is_marker(&next.text) {
                    break;
                }
                let geometry = wrap_geometry(&line, &last, next, columns);
                let join = if geometry.cut {
                    Join::Cut
                } else if geometry.fits
                    && indentation(&next.text) == indentation(&line.text)
                    && prose_commands(&line.text).last().is_some_and(|c| c.1)
                {
                    // A command named in a sentence that runs on past the row.
                    Join::Wrap
                } else {
                    break;
                };
                append(next, &mut line, join, leading_ws(&next.text));
                last = next.clone();
                index += 1;
                joined += 1;
            }
        }
        result.push(line);
        index += 1;
    }
    result
}

/// Lines one per row (no joins), each with its command: the detector
/// without wrap handling.
pub fn single_lines(rows: &[&str], colors: &RowColors) -> Vec<Line> {
    let mut lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(row, text)| {
            let mut line = Line::new(row, text);
            unframe(&mut line);
            line
        })
        .collect();
    for index in 0..lines.len() {
        let start = command_start(
            &lines[index].text,
            colors.contains_key(&index),
            keyword(&lines[index], colors),
            false,
            (index > 0).then(|| lines[index - 1].text.as_str()),
            false,
        );
        if let Some(start) = start {
            lines[index].command = command_range(&lines[index], &start);
        }
    }
    lines
}

/// Whether a line's first word has a colour run of exactly its own.
fn keyword(line: &Line, colors: &RowColors) -> bool {
    let Some(part) = line.parts.first() else {
        return false;
    };
    let Some(runs) = colors.get(&part.row) else {
        return false;
    };
    let b = line.text.as_bytes();
    let start = leading_ws(&line.text);
    let mut end = start;
    while end < b.len() && b[end] != b' ' {
        end += 1;
    }
    let word = part.row_start + start..part.row_start + end;
    !word.is_empty() && runs.iter().any(|(range, _)| *range == word)
}

/// The continuation indent of a tool call's rows: the least indent
/// among them, so deeper indents (heredoc bodies) are kept.
fn wrapper_indent(soft: &[Line], index: usize) -> usize {
    let mut least = usize::MAX;
    for line in soft.iter().skip(index + 1).take(64) {
        if is_blank(&line.text) {
            continue;
        }
        let indent = indentation(&line.text);
        if indent == 0 || hard_stop(line, Mode::Wrapper) {
            break;
        }
        least = least.min(indent);
    }
    if least == usize::MAX { 0 } else { least }
}

fn strip(next: &Line, join: Join, start: &CommandStart, base: usize) -> usize {
    match join {
        Join::Cut | Join::Wrap => leading_ws(&next.text),
        Join::Quote => ps2_prefix(&next.text).0,
        Join::Explicit => {
            let ps2 = ps2_prefix(&next.text).0;
            ps2 + leading_ws(&next.text[ps2..])
        }
        Join::Newline => {
            if start.mode == Mode::Prompt {
                ps2_prefix(&next.text).0
            } else {
                indentation(&next.text).min(base)
            }
        }
    }
}

/// The secondary prompt's length and whether it echoes a `>`.
fn ps2_prefix(text: &str) -> (usize, bool) {
    let m = PS2.find(text).ok().flatten();
    let length = m.map_or(0, |m| m.end());
    (length, text[..length].contains('>'))
}

/// Rows a coding agent draws around its content: a new message or tool
/// call, a tool's output, rules and box corners.
fn hard_stop(line: &Line, mode: Mode) -> bool {
    let Some(first) = line.text.chars().find(|&c| c != ' ' && c != '\t') else {
        return false;
    };
    if "●⏺⎿✻✶✳✢•─━╭╰┌└├".contains(first) {
        return true;
    }
    mode == Mode::Wrapper && indentation(&line.text) == 0
}

/// A row that starts a command of its own: a prompt, `!` or tool call.
pub fn is_marker(text: &str) -> bool {
    if matches(&TOOL_CALL, text) || matches(&RAN_CALL, text) || matches(&AGENT_PROMPT, text) {
        return true;
    }
    prompt_marker(text, leading_ws(text)).is_some_and(|m| !m.weak)
}

fn trim_trailing_whitespace(text: &str) -> &str {
    text.trim_end_matches(char::is_whitespace)
}

struct Geometry {
    cut: bool,
    fits: bool,
    aligned: bool,
}

/// The last physical row's text of `last`, trailing blanks trimmed.
fn last_row(last: &Line) -> &str {
    let trimmed = trim_trailing_whitespace(&last.text);
    let from = last.parts.last().map_or(0, |p| p.start).min(trimmed.len());
    &trimmed[from..]
}

fn wrap_geometry(line: &Line, last: &Line, next: &Line, columns: Option<usize>) -> Geometry {
    let Some(columns) = columns else {
        return Geometry {
            cut: false,
            fits: false,
            aligned: false,
        };
    };
    let trimmed = trim_trailing_whitespace(&last.text);
    let last_row = last_row(last);
    let last_token = words(trimmed).next_back().unwrap_or("");
    let first_token = words(&next.text).next().unwrap_or("");
    // Application wraps repeat a non-zero indent. Rows at column zero
    // come from a shell, where the terminal soft-wraps instead.
    let indent = indentation(&line.text) as i64;
    let width = cells(last_row) as i64;
    let aligned = indent > 0 && indentation(&next.text) as i64 == indent;
    let edge = last.limit.unwrap_or(columns) as i64 - WRAP_TOLERANCE;
    // A token longer than the wrap width can only have been cut;
    // a shorter one would have moved to the next row whole.
    let first_cells = cells(first_token) as i64;
    let cut = aligned
        && width >= edge
        && last_token.contains('/')
        && cells(last_token) as i64 + first_cells > edge - indent;
    Geometry {
        cut,
        fits: width + 1 + first_cells > edge,
        aligned,
    }
}

fn continuation(
    line: &Line,
    last: &Line,
    next: &Line,
    after: &[Line],
    start: &CommandStart,
    state: &ShellState,
    columns: Option<usize>,
) -> Option<Join> {
    if start.mode == Mode::Shown {
        if is_blank(&next.text)
            || indentation(&next.text) == 0
            || hard_stop(next, Mode::Block)
            || is_marker(&next.text)
            || matches(&KEY_HINT, &next.text)
        {
            return None;
        }
        // The agent moves a word that doesn't fit to the next row whole,
        // and breaks one only when it is longer than a row: at the edge.
        let Some(columns) = columns else {
            return Some(Join::Wrap);
        };
        let row = last_row(last);
        let last_word = words(row).next_back().unwrap_or("");
        let first_word = words(&next.text).next().unwrap_or("");
        let columns = columns as i64;
        let cut = cells(row) as i64 >= columns - WRAP_TOLERANCE
            && (cells(last_word) + cells(first_word)) as i64
                > columns - indentation(&next.text) as i64;
        return Some(if cut { Join::Cut } else { Join::Wrap });
    }
    if is_blank(&next.text) {
        // Blank rows belong to a heredoc, string or tool call that goes on below.
        if start.mode == Mode::Prompt || !state.open() {
            return None;
        }
        let more = after.iter().find(|l| !is_blank(&l.text))?;
        if hard_stop(more, start.mode) {
            return None;
        }
        return Some(Join::Newline);
    }
    if hard_stop(next, start.mode) {
        return None;
    }
    if (start.mode == Mode::Prompt || (!state.heredoc && !state.quote)) && is_marker(&next.text) {
        return None;
    }
    let geometry = wrap_geometry(line, last, next, columns);
    if geometry.cut {
        return Some(Join::Cut);
    }
    if state.heredoc {
        return Some(Join::Newline);
    }
    match start.mode {
        Mode::Prompt => {
            if state.continues {
                return Some(Join::Explicit);
            }
            if state.quote || state.depth > 0 {
                return Some(if ps2_prefix(&next.text).1 {
                    Join::Newline
                } else {
                    Join::Quote
                });
            }
            (geometry.aligned && geometry.fits).then_some(Join::Wrap)
        }
        Mode::Wrapper => {
            // A row after a continuation backslash starts a new line; after
            // a pipe it may as well be the wrap it looks like.
            if !state.backslash && indentation(&next.text) > 0 && geometry.fits {
                return Some(Join::Wrap);
            }
            state.open().then_some(Join::Newline)
        }
        Mode::Shown => None,
        Mode::Block => {
            let description = start.header && describes(next, after);
            if !state.backslash && geometry.aligned && geometry.fits && !description {
                return Some(Join::Wrap);
            }
            if state.open() || state.continues {
                return Some(Join::Newline);
            }
            if start.header
                && !description
                && let Some(word) = words(&next.text).next()
                && tier(word).is_some()
            {
                return Some(Join::Newline);
            }
            None
        }
    }
}

/// A permission box's last row under a command describes it in prose.
fn describes(next: &Line, after: &[Line]) -> bool {
    if !after.first().is_none_or(|l| is_blank(&l.text)) {
        return false;
    }
    let Some(word) = words(&next.text).next() else {
        return false;
    };
    if !word.chars().next().is_some_and(char::is_uppercase) {
        return false;
    }
    tier(word).is_none()
}

fn append(next: &Line, line: &mut Line, join: Join, skip: usize) {
    let mut end = line.len();
    while end > 0 && !line.inserted.contains(&(end - 1)) {
        match char_before(&line.text, end) {
            Some(c) if is_hspace(c) => end -= c.len_utf8(),
            _ => break,
        }
    }
    line.text.truncate(end);
    line.parts.retain_mut(|part| {
        let length = (part.length as i64).min(end as i64 - part.start as i64);
        if length > 0 {
            part.length = length as usize;
            true
        } else {
            false
        }
    });
    line.inserted.retain(|&o| o < end);
    line.dropped.retain(|&o| o < end);
    line.newlines.retain(|&o| o < end);
    let backslash = join == Join::Explicit
        && line.text.ends_with('\\')
        && line.text.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 1;
    if backslash {
        line.dropped.insert(end - 1);
    }
    if join != Join::Cut {
        let at = line.len();
        line.inserted.insert(at);
        if backslash {
            line.dropped.insert(at);
        }
        if join == Join::Newline {
            line.newlines.insert(at);
        }
        line.text.push(' ');
    }
    let skip = skip.min(next.len());
    let length = line.len();
    line.text.push_str(&next.text[skip..]);
    // Positions in `next` shift by `length - skip`.
    let shift = |offset: usize| (offset + length).saturating_sub(skip);
    for (index, part) in next.parts.iter().enumerate() {
        let mut part = *part;
        let mut part_length = part.length as i64;
        if index == 0 {
            part.row_start += skip;
            part_length -= skip as i64;
            part.start = length;
        } else {
            part.start = shift(part.start);
        }
        if part_length > 0 {
            part.length = part_length as usize;
            line.parts.push(part);
        }
    }
    line.inserted
        .extend(next.inserted.iter().map(|&o| shift(o)));
    line.dropped.extend(next.dropped.iter().map(|&o| shift(o)));
    line.newlines
        .extend(next.newlines.iter().map(|&o| shift(o)));
}

/// A command's text as it would run: separators that stand for line
/// breaks become newlines, continuation backslashes drop out. Returns the
/// bytes and, for each, its offset in the line.
pub fn runnable(line: &Line, range: Range<usize>) -> (Vec<u8>, Vec<usize>) {
    let b = line.text.as_bytes();
    let mut units = Vec::with_capacity(range.len());
    let mut offsets = Vec::with_capacity(range.len());
    let plain = line.dropped.is_empty() && line.newlines.is_empty();
    for offset in range {
        if !plain && line.dropped.contains(&offset) {
            continue;
        }
        units.push(if !plain && line.newlines.contains(&offset) {
            b'\n'
        } else {
            b[offset]
        });
        offsets.push(offset);
    }
    (units, offsets)
}

fn shell_state(line: &Line, start: &CommandStart) -> ShellState {
    let end = start.end.unwrap_or(line.len());
    let (units, _) = runnable(line, start.location..end.max(start.location));
    lex(&units, start.mode == Mode::Wrapper)
}

/// `colored`: the row has colours. `keyword`: its first word has a
/// colour of its own, the way a highlighter marks code.
fn command_start(
    text: &str,
    colored: bool,
    keyword: bool,
    fence: bool,
    previous: Option<&str>,
    agent: bool,
) -> Option<CommandStart> {
    let from = leading_ws(text);
    let lead = char_at(text, from)?;
    if let Ok(Some(m)) = SHOWN_CALL.find(text) {
        let location = m.end();
        if !accepts(&tokens(text, location..text.len()), Context::Prompt) {
            return None;
        }
        return Some(CommandStart::new(location, Mode::Shown));
    }
    if lead == '⎿' {
        return None; // starts a tool's output
    }
    if let Ok(Some(m)) = TOOL_CALL.find(text) {
        return Some(CommandStart::new(m.end(), Mode::Wrapper));
    }
    if let Ok(Some(m)) = RAN_CALL.find(text) {
        let location = m.end();
        if !accepts(&tokens(text, location..text.len()), Context::Prompt) {
            return None;
        }
        return Some(CommandStart::new(location, Mode::Block));
    }
    if let Ok(Some(c)) = AGENT_PROMPT.captures(text) {
        let range = c.get(1).unwrap().range();
        if !accepts(&tokens(text, range.clone()), Context::Prompt) {
            return None;
        }
        return Some(CommandStart::new(range.start, Mode::Prompt));
    }
    if fence {
        if lead == '#' {
            return None;
        }
        if lead != '$' {
            return source_start(text, false);
        }
    }
    if let Some(marker) = prompt_marker(text, from) {
        let b = text.as_bytes();
        let mut location = marker.location;
        while location < b.len() && b[location] == b' ' {
            location += 1;
        }
        if marker.weak {
            if previous.is_some_and(|p| matches(&NPM_BANNER, p)) {
                return None;
            }
            return plain_start(text, location, colored && !keyword, Mode::Prompt);
        }
        if marker.arrow {
            location = after_arrow_segments(text, location);
        }
        let (end, fixed) = right_prompt(text, location);
        let words = tokens(text, location..end);
        let first = words.first()?;
        if matches_graphemes(&NUMBERED, first.text) || !accepts(&words, Context::Prompt) {
            return None;
        }
        // In an agent's transcript a bare `❯` starts the user's own
        // message ("❯ merged"): only a line that reads as a command on
        // its own counts, never a single word.
        if agent && lead == '❯' && !accepts(&words, Context::Plain) {
            return None;
        }
        return Some(CommandStart {
            end: fixed.then_some(end),
            ..CommandStart::new(location, Mode::Prompt)
        });
    }
    if matches(&DIFF_LINE, text) {
        return None;
    }
    plain_start(text, from, colored && !keyword, Mode::Block)
}

/// The end of a prompt's marker (`$ `, `% `, `# `, `> `, `❯ `, `➜ `).
/// `weak` markers (`#` and `>` alone) are also comments and quotes, so
/// their text must read as a command by itself.
struct Marker {
    location: usize,
    weak: bool,
    arrow: bool,
}

fn prompt_marker(text: &str, from: usize) -> Option<Marker> {
    let b = text.as_bytes();
    // At most 160 UTF-16 units in.
    let mut units = 0;
    for (offset, c) in text[from..].char_indices() {
        if units >= 160 {
            break;
        }
        units += c.len_utf16();
        let at = from + offset;
        let k = at + c.len_utf8();
        let glyph = c == '❯' || c == '➜';
        if !(glyph || c == '$' || c == '%' || c == '#' || c == '>') {
            continue;
        }
        let after = char_at(text, k);
        let prefix = trim_hspace(&text[from..at]);
        if glyph {
            if after != Some(' ') {
                continue;
            }
            return Some(Marker {
                location: k,
                weak: false,
                arrow: c == '➜' && prefix.is_empty(),
            });
        }
        // cmd.exe: `C:\Users\user>dir`
        if c == '>' && after != Some(' ') {
            if matches_graphemes(&CMD_EXE, prefix) && after.is_some() {
                return Some(Marker {
                    location: k,
                    weak: false,
                    arrow: false,
                });
            }
            continue;
        }
        if after != Some(' ') {
            continue;
        }
        let attached = at > from && b[at - 1] != b' ';
        if c == '>' && attached && matches!(b[at - 1], b'>' | b'-' | b'=') {
            continue; // >>> -> =>
        }
        if prefix.is_empty() {
            return Some(Marker {
                location: k,
                weak: c == '#' || c == '>',
                arrow: false,
            });
        }
        // `#` and `>` end a prompt only when attached (`root@host:/etc#`,
        // `~/proj>`) or after a lone folder (busybox: `/app # `).
        if (c == '#' || c == '>')
            && !attached
            && (!(prefix.starts_with('/') || prefix.starts_with('~')) || prefix.contains(' '))
        {
            continue;
        }
        // zsh's `host% ` and `folder % `, or `folder $ `.
        if (c == '%' || c == '$') && matches_graphemes(&PROMPT_WORD, prefix) {
            return Some(Marker {
                location: k,
                weak: false,
                arrow: false,
            });
        }
        if prompt_like(prefix) {
            return Some(Marker {
                location: k,
                weak: false,
                arrow: false,
            });
        }
    }
    None
}

/// Text before a prompt marker that a prompt would show: a user@host,
/// a path, a bracketed segment, a shell version or glyphs.
fn prompt_like(prefix: &str) -> bool {
    let all: Vec<&str> = words(prefix).collect();
    let Some(&last) = all.last() else {
        return false;
    };
    if all.len() > 10 {
        return false;
    }
    if last.chars().any(|c| "@~/\\".contains(c)) {
        return true;
    }
    if (prefix.ends_with(']') && prefix.contains('['))
        || (prefix.ends_with(')') && prefix.contains('('))
    {
        return true;
    }
    if matches(&SHELL_VERSION, last) {
        return true;
    }
    if all.len() <= 4 && all[0].contains('@') {
        return true;
    }
    last.chars().all(|c| !c.is_ascii())
}

/// oh-my-zsh: `➜  proj git:(main) ✗ cmd` shows the folder and branch
/// after the arrow.
fn after_arrow_segments(text: &str, location: usize) -> usize {
    let words = tokens(text, location..text.len());
    let mark = |w: &str| ["✗", "✔", "✘"].contains(&w);
    let mut index = 0;
    let segment =
        words.len() >= 2 && (matches_graphemes(&SEGMENT, words[1].text) || mark(words[1].text));
    if words.len() >= 2
        && (segment || (tier(words[0].text).is_none() && !path_executable(words[0].text)))
    {
        index = 1;
    }
    if index < words.len() && matches_graphemes(&SEGMENT, words[index].text) {
        index += 1;
    }
    if index < words.len() && mark(words[index].text) {
        index += 1;
    }
    if index < words.len() {
        words[index].range.start
    } else {
        text.len()
    }
}

/// Where a command typed at a prompt ends: before a right-hand prompt
/// (time, duration, status) across a wide gap, or at the trimmed end.
fn right_prompt(text: &str, from: usize) -> (usize, bool) {
    let b = text.as_bytes();
    let mut end = b.len();
    while end > from && (b[end - 1] == b' ' || b[end - 1] == b'\t') {
        end -= 1;
    }
    let mut quote = 0u8;
    let mut gap = 0;
    for (i, &c) in b.iter().enumerate().take(end).skip(from) {
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'\'' || c == b'"' {
            quote = c;
        }
        if c == b' ' && quote == 0 {
            gap += 1;
            if gap == 5 {
                return (i - 4, true);
            }
        } else {
            gap = 0;
        }
    }
    (end, false)
}

/// A whole line, after any list marker, that reads as a command.
fn plain_start(text: &str, from: usize, colored: bool, mode: Mode) -> Option<CommandStart> {
    let mut location = from;
    if let Ok(Some(m)) = LIST_PREFIX.find(&text[from..]) {
        location = from + m.end();
    }
    let b = text.as_bytes();
    let mut end = b.len();
    let mut fixed = None;
    // A table cell ends at its border.
    if let Some(cell) = text[location..end].find('│') {
        end = location + cell;
        fixed = Some(end);
    }
    while end > location && b[end - 1] == b' ' {
        end -= 1;
    }
    if fixed.is_some() {
        fixed = Some(end);
    }
    // Columns of output (`vite     5.4.1    5.4.8`) aren't a command.
    if end <= location || text[location..end].contains("   ") {
        return None;
    }
    if !accepts(
        &tokens(text, location..end),
        if colored {
            Context::Colored
        } else {
            Context::Plain
        },
    ) {
        return None;
    }
    Some(CommandStart {
        end: fixed,
        ..CommandStart::new(location, mode)
    })
}

/// A row of displayed shell source: a shell code block or the command
/// in a permission box.
fn source_start(text: &str, header: bool) -> Option<CommandStart> {
    let location = leading_ws(text);
    if location >= text.len() || !accepts(&tokens(text, location..text.len()), Context::Prompt) {
        return None;
    }
    Some(CommandStart {
        header,
        ..CommandStart::new(location, Mode::Block)
    })
}

/// The command on a line once its rows are joined.
fn command_range(line: &Line, start: &CommandStart) -> Option<Range<usize>> {
    let text = &line.text;
    let b = text.as_bytes();
    let mut end = start.end.unwrap_or(b.len()).min(b.len());
    if start.mode == Mode::Shown
        && let Ok(Some(m)) = ELAPSED.find(&text[start.location..end])
        && m.range().len() > 3
    {
        end = start.location + m.start();
    }
    if start.mode == Mode::Wrapper {
        let (units, offsets) = runnable(line, start.location..end.max(start.location));
        if let Some(close) = lex(&units, true).close {
            end = offsets[close];
        } else {
            // The tool call's own parenthesis after a command whose
            // quotes or heredoc the screen cut short.
            while end > start.location && b[end - 1] == b' ' {
                end -= 1;
            }
            if end > start.location && b[end - 1] == b')' {
                end -= 1;
            }
        }
        // An agent shortens long commands with an ellipsis: not runnable.
        if end > start.location && text[..end].ends_with('…') {
            return None;
        }
    }
    while end > start.location {
        match char_before(text, end) {
            Some(c) if is_hspace(c) => end -= c.len_utf8(),
            _ => break,
        }
    }
    (end > start.location).then_some(start.location..end)
}
