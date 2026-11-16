//! ⌘S "smart select" for every shell: what's on a terminal screen and how
//! a selection moves over it.
//!
//! - [`detect`]: the items on screen (URLs, paths, commands, IPs, domains,
//!   hashes and IDs, branches), from the visible rows, their soft wraps and
//!   colours. Paths check the filesystem through [`detect::Filesystem`].
//! - [`select`]: kind-letter labels, lines, text paragraphs, grow and shrink,
//!   arrow neighbours, range text and the Run line.
//! - [`run`]: where Run sends a command (an idle shell, else a new pane).
//! - [`callout`]: where labels go (geometry only).
//! - The row classifier: the detector is good at atoms and decent at
//!   commands, but picks up output and prose rows that start with a command
//!   word. This labels each screen row (see [`RowLabel`]) with a small linear
//!   model over hashed row-shape features plus Viterbi smoothing, then turns
//!   the labels into command spans and suppressions ([`refine::refine`]),
//!   behind `[hints] row_classifier`.
//!
//! Positions cross the API as rows and UTF-16 offsets (the macOS shell's
//! string offsets). The macOS glue is `azj_hints_*` in core/crates/ffi.
//!
//! The model trains offline on synthetic screens only (`tools/generate.py`,
//! `azj-hints-train`, `just hints-model`) and is scored on hand-written
//! screens it never saw (`data/heldout.txt`, `azj-hints-eval`). Weights live
//! in `model/rows.json`, embedded at compile time. No platform dependencies:
//! the crate builds and tests on Linux as on macOS.

pub mod callout;
pub mod corpus;
pub mod detect;
pub mod features;
pub mod model;
pub mod refine;
pub mod run;
pub mod select;
pub mod text;

pub use detect::{Input, Item, Kind, detect};
pub use model::{Labelling, Model};
pub use refine::{Command, Span, refine};

/// What one screen row holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum RowLabel {
    /// A prompt and the command typed after it, a `● Bash(` tool call or a
    /// `!` agent command.
    Command = 0,
    /// The rest of the command above: a wrap, `\`, a secondary prompt, a
    /// heredoc body.
    Continuation = 1,
    /// A command without a prompt: a code block, an indented example, the
    /// command in a permission box.
    Code = 2,
    /// What a program printed.
    Output = 3,
    /// Text written for reading: agent answers, docs, plans.
    Prose = 4,
    /// Frames, status lines, input boxes, bare prompts.
    Chrome = 5,
    /// Nothing but whitespace.
    Blank = 6,
}

impl RowLabel {
    pub const ALL: [RowLabel; 7] = [
        RowLabel::Command,
        RowLabel::Continuation,
        RowLabel::Code,
        RowLabel::Output,
        RowLabel::Prose,
        RowLabel::Chrome,
        RowLabel::Blank,
    ];
    pub const COUNT: usize = 7;

    /// The one-letter code the corpora use.
    pub fn code(self) -> char {
        match self {
            RowLabel::Command => 'C',
            RowLabel::Continuation => 'c',
            RowLabel::Code => 'K',
            RowLabel::Output => 'O',
            RowLabel::Prose => 'P',
            RowLabel::Chrome => 'T',
            RowLabel::Blank => 'B',
        }
    }

    pub fn from_code(c: char) -> Option<RowLabel> {
        RowLabel::ALL.into_iter().find(|l| l.code() == c)
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            RowLabel::Command => "command",
            RowLabel::Continuation => "continuation",
            RowLabel::Code => "code",
            RowLabel::Output => "output",
            RowLabel::Prose => "prose",
            RowLabel::Chrome => "chrome",
            RowLabel::Blank => "blank",
        }
    }
}

/// Labels every row of a screen with the embedded model. `joins` holds the
/// rows whose last cell soft-wraps into the next row (the terminal's wrap
/// flag); `columns` is the pane width, when known.
pub fn classify(rows: &[&str], joins: &[usize], columns: Option<usize>) -> Vec<RowLabel> {
    Model::embedded().classify(rows, joins, columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Random rows from a hostile alphabet (wide, combining, box drawing,
    /// prompt markers, quotes) never panic and always get one label each.
    #[test]
    fn arbitrary_rows_never_panic() {
        let alphabet: Vec<char> = "ab Z09 $%#>❯➜✗●⏺⎿│╭╰─\\|&;'\"`()[]{}<>=~/.:-界👩‍💻é\t…"
            .chars()
            .collect();
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..300 {
            let n = (next() % 12) as usize;
            let rows: Vec<String> = (0..n)
                .map(|_| {
                    (0..(next() % 70))
                        .map(|_| alphabet[(next() as usize) % alphabet.len()])
                        .collect()
                })
                .collect();
            let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
            let joins: Vec<usize> = (0..n).filter(|_| next() % 4 == 0).collect();
            let labelling = Model::embedded().label(&refs, &joins, Some((next() % 120) as usize));
            assert_eq!(labelling.labels.len(), n);
            let regex: Vec<Command> = (0..n)
                .map(|row| Command {
                    value: "x".into(),
                    spans: vec![Span {
                        row,
                        start: (next() % 80) as usize,
                        length: (next() % 9) as usize,
                    }],
                })
                .collect();
            let _ = refine(&refs, &joins, Some(80), &labelling, &regex);
        }
    }
}
