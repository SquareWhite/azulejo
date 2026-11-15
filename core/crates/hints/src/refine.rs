//! Row labels → command spans, merged with the regex detector's commands.
//!
//! The regex detector stays authoritative for atoms (URLs, paths, IPs,
//! hashes); this module only touches commands:
//!
//! - A command row ([`RowLabel::Command`] or [`RowLabel::Code`]) gives a
//!   command from where its prompt ends (or its text starts) through the
//!   continuation rows below it: soft wraps join without a space, hard wraps
//!   and `\` / secondary prompts / heredoc bodies with one, and a `● Bash(`
//!   call ends at its closing parenthesis.
//! - It replaces a regex command that starts on the same row (the regex may
//!   have cut it short or started it inside the prompt) unless the regex one
//!   reaches further down.
//! - A regex command that fills a whole output, prose or chrome row (a
//!   `test result: ok` line, a numbered plan item) is dropped. Inline ones
//!   (backticks, colour) on those rows stay.
//! - Labelled commands the regex missed are added.

use serde::{Deserialize, Serialize};

use crate::text;
use crate::{Labelling, RowLabel};

/// A slice of one physical row, in UTF-16 code units (the shell's ranges).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub row: usize,
    pub start: usize,
    pub length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub value: String,
    pub spans: Vec<Span>,
}

impl Command {
    fn first_row(&self) -> Option<usize> {
        self.spans.first().map(|s| s.row)
    }
}

/// What [`refine`] did, for diagnostics and tests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Changes {
    pub suppressed: usize,
    pub replaced: usize,
    pub added: usize,
}

/// Commands from labels alone. `columns` (the pane width) tells a row an
/// application wrapped (joined with a space) from a real line break (kept,
/// so heredocs and code blocks run as shown); without it rows join with
/// spaces.
pub fn label_commands(
    rows: &[&str],
    joins: &[usize],
    labels: &[RowLabel],
    columns: Option<usize>,
) -> Vec<Command> {
    let mut out = Vec::new();
    for i in 0..rows.len().min(labels.len()) {
        if !matches!(labels[i], RowLabel::Command | RowLabel::Code) {
            continue;
        }
        if let Some(command) = build(rows, joins, labels, i, columns) {
            out.push(command);
        }
    }
    out
}

fn build(
    rows: &[&str],
    joins: &[usize],
    labels: &[RowLabel],
    first: usize,
    columns: Option<usize>,
) -> Option<Command> {
    let row = rows[first];
    let (_, box_end) = text::unboxed(row);
    let mut bash = false;
    let start = if labels[first] == RowLabel::Command {
        if let Some(at) = text::bash_call(row) {
            bash = true;
            at
        } else if let Some(at) = agent_command(row) {
            at
        } else if let Some(at) = agent_run(row) {
            at
        } else {
            let p = text::prompt(row)?;
            if p.strength == 0 {
                return None;
            }
            p.start
        }
    } else {
        let at = text::byte_at_utf16(row, content_start(row));
        at + embedded_start(&row[at..])
    };
    let mut end = if box_end > start {
        box_end
    } else {
        row.trim_end().len()
    };
    if !bash {
        // A right-hand prompt, or a build step's timing (`RUN … 18.4s`).
        end = start + right_prompt(&row[start..end]);
    }
    if end <= start {
        return None;
    }
    let mut pieces: Vec<(usize, usize, usize, bool)> = vec![(first, start, end, false)];
    let mut depth = 1i32;
    if bash && let Some(cut) = close_paren(&row[start..end], &mut depth) {
        pieces[0].2 = start + cut;
    }
    let mut j = first + 1;
    while depth > 0 && j < rows.len() && j < labels.len() && labels[j] == RowLabel::Continuation {
        let r = rows[j];
        let soft = joins.contains(&(j - 1));
        let (bs, be) = text::unboxed(r);
        let s = if soft {
            0
        } else {
            let s = text::ps2_len(r).max(bs);
            s + (r[s..].len() - r[s..].trim_start().len())
        };
        let e = if soft { r.trim_end().len() } else { be.max(s) };
        let mut e2 = e;
        if bash && let Some(cut) = close_paren(&r[s..e], &mut depth) {
            e2 = s + cut;
        }
        if e2 > s {
            pieces.push((j, s, e2, soft));
        }
        j += 1;
    }
    if bash && depth > 0 {
        // The call's end is off screen or mislabelled; keep what we have,
        // minus a trailing `)` that can't be the call's.
    }

    // At a shell prompt `\` continuations join with a space, as the shell
    // sees them; in a code block or tool call the line break stays.
    let prompt = labels[first] == RowLabel::Command && !bash;
    let mut value = String::new();
    let mut spans = Vec::new();
    let mut separator = "";
    for k in 0..pieces.len() {
        let (r, s, mut e, soft) = pieces[k];
        let text = rows[r];
        let next = pieces.get(k + 1).filter(|p| !p.3);
        let mut next_separator = "";
        if let Some(&(nr, ns, _, _)) = next {
            let piece = text[s..e].trim_end();
            let backslash = piece.chars().rev().take_while(|&c| c == '\\').count() % 2 == 1;
            let width = text::cells(text.trim_end());
            let word = text::cells(text::first_word(&rows[nr][ns..]));
            let wrapped = columns.is_none_or(|c| width + 1 + word + WRAP_TOLERANCE > c);
            next_separator = if backslash && prompt {
                // The backslash joins rows; it isn't part of the value.
                e = s + piece.len() - 1;
                e = s + text[s..e].trim_end().len();
                " "
            } else if wrapped && !backslash {
                " "
            } else {
                "\n"
            };
        }
        if k > 0 && !soft {
            value.push_str(separator);
        }
        separator = next_separator;
        value.push_str(&text[s..e]);
        if e > s {
            spans.push(Span {
                row: r,
                start: text::utf16_len(&text[..s]),
                length: text::utf16_len(&text[s..e]),
            });
        }
    }
    let value = value.trim().to_owned();
    if value.is_empty() || value.chars().any(|c| c.is_control() && c != '\n') || spans.is_empty() {
        return None;
    }
    Some(Command { value, spans })
}

/// Wrappers may stop this many cells short of the pane's edge
/// (detect::lines::WRAP_TOLERANCE).
const WRAP_TOLERANCE: usize = 2;

/// `!cmd` or `! cmd`: a coding agent's shell escape.
fn agent_command(row: &str) -> Option<usize> {
    let trimmed = row.trim_start();
    let rest = trimmed.strip_prefix('!')?;
    let rest = rest.trim_start();
    let c = rest.chars().next()?;
    (c.is_ascii_alphabetic() || "_./~".contains(c)).then_some(row.len() - rest.len())
}

/// A command the regex missed is only added when it starts like one (a
/// known command, a program path or an assignment) and is whole: not cut
/// short with `…` or an open quote or heredoc, not a tool's log line
/// (`npm ERR! …`), not a diff line, not a sentence ("find out more at …").
fn addable(command: &Command, rows: &[&str]) -> bool {
    let value = command.value.as_str();
    let words: Vec<&str> = value.split_whitespace().collect();
    let Some(first) = words.first() else {
        return false;
    };
    let second = words.get(1).copied().unwrap_or("");
    let stops = words
        .iter()
        .filter(|w| {
            matches!(
                w.to_lowercase()
                    .trim_matches(|c: char| !c.is_alphanumeric()),
                "the"
                    | "a"
                    | "an"
                    | "to"
                    | "of"
                    | "at"
                    | "in"
                    | "out"
                    | "more"
                    | "and"
                    | "you"
                    | "your"
                    | "work"
                    | "well"
                    | "here"
                    | "under"
            )
        })
        .count();
    let sentence = value.ends_with('.') && stops >= 1 || stops >= 2;
    let diff = command
        .spans
        .first()
        .and_then(|s| rows.get(s.row))
        .is_some_and(|r| {
            let r = r.trim_start();
            (r.starts_with('+') || r.starts_with('-')) && r[1..].starts_with([' ', '\t'])
        });
    text::word_starts_command(first)
        && !value.ends_with('…')
        && !text::open_quote(value)
        && !(value.contains("<<") && !value.contains('\n'))
        && !matches!(
            second,
            "ERR!" | "WARN" | "ERROR" | "warn" | "notice" | "error" | "info" | "INFO"
        )
        && !sentence
        && !diff
}

/// Where the command starts in a row of output that shows one: after a
/// Dockerfile or build step's `RUN` (`=> [2/6] RUN …`, `Step 3/7 : RUN …`)
/// or a workflow's `run:`. Zero for anything else.
fn embedded_start(text: &str) -> usize {
    for marker in ["- run: ", "run: "] {
        if let Some(rest) = text.strip_prefix(marker) {
            return text.len() - rest.trim_start().len();
        }
    }
    if let Some(rest) = text.strip_prefix("RUN ") {
        return text.len() - rest.trim_start().len();
    }
    // A step label first: `=> [2/6] RUN`, `#5 [2/4] RUN`, `Step 3/7 : RUN`.
    if let Some(at) = text.find(" RUN ") {
        let head = &text[..at];
        let step = head.contains('[') && head.ends_with(']')
            || head.starts_with("Step ") && head.ends_with(" :");
        if step {
            let rest = &text[at + 5..];
            return text.len() - rest.trim_start().len();
        }
    }
    0
}

/// Where a command typed at a prompt ends when a right-hand prompt (a
/// time, a duration, a status) follows it across a wide gap.
fn right_prompt(text: &str) -> usize {
    let trimmed = text.trim_end();
    if let Some(gap) = trimmed.rfind("    ") {
        let tail = trimmed[gap..].trim_start();
        let short = !tail.is_empty() && tail.chars().count() <= 24 && !tail.contains(' ');
        let looks = tail
            .chars()
            .any(|c| c.is_ascii_digit() || "✔✘✗↵⏎".contains(c));
        if short && looks && !text::open_quote(&trimmed[..gap]) {
            return trimmed[..gap].trim_end().len();
        }
    }
    trimmed.len()
}

/// `• Ran cmd`: another agent's record of a command it ran.
fn agent_run(row: &str) -> Option<usize> {
    let trimmed = row.trim_start();
    let mut chars = trimmed.chars();
    if !text::is_agent_bullet(chars.next()?) {
        return None;
    }
    let rest = chars
        .as_str()
        .trim_start()
        .strip_prefix("Ran ")?
        .trim_start();
    (!rest.is_empty()).then_some(row.len() - rest.len())
}

fn utf16_char_before(text: &str, utf16: usize) -> Option<char> {
    let at = text::byte_at_utf16(text, utf16);
    text[..at].chars().next_back()
}

/// Tracks parentheses (outside quotes) and returns the byte offset of the
/// `)` that closes the call, if it's in `text`.
fn close_paren(text: &str, depth: &mut i32) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in text.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if q == c => quote = None,
            (None, '(') => *depth += 1,
            (None, ')') => {
                *depth -= 1;
                if *depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Where a row's text starts once indent, box border and a list marker
/// (`- `, `* `, `• `, `1. `) are skipped, in UTF-16 units.
fn content_start(row: &str) -> usize {
    let (s, _) = text::unboxed(row);
    let mut rest = &row[s..];
    for marker in ["- ", "* ", "• ", "+ "] {
        if let Some(r) = rest.strip_prefix(marker) {
            rest = r;
            break;
        }
    }
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && digits <= 3 {
        let r = &rest[digits..];
        if let Some(r) = r.strip_prefix(". ").or_else(|| r.strip_prefix(") ")) {
            rest = r;
        }
    }
    let rest = rest.trim_start();
    text::utf16_len(&row[..row.len() - rest.len()])
}

/// Merges labelled commands into the regex detector's commands.
///
/// The regex detector reads prompts and joins continuations at least as
/// well as this crate, so on a command row its command stays, unless it
/// started inside the prompt where we found a known command after it.
/// Output and chrome rows
/// lose commands that aren't in backticks; prose rows keep theirs (prose
/// names commands mid-sentence, and the detector already demands a
/// command's shape there).
pub fn refine(
    rows: &[&str],
    joins: &[usize],
    columns: Option<usize>,
    labelling: &Labelling,
    regex: &[Command],
) -> (Vec<Command>, Changes) {
    refine_with(rows, joins, columns, labelling, regex, &Policy::default())
}

/// What the labels may change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Policy {
    /// Drop regex commands on output and chrome rows (outside backticks).
    pub suppress: bool,
    /// Add commands on command and code rows the regex missed.
    pub add: bool,
    /// How sure the model must be of a row before it acts there.
    pub min_margin: f32,
}

/// The default confidence threshold, in model units (a max-marginal gap).
/// Chosen on data/heldout.txt, blind.txt, blind2.txt and the shell
/// detector's corpus: from 40 up the classifier no longer overrides a
/// command the detector got right there. Recheck it after `just
/// hints-model` (`azj-hints-eval --margin N`): a retrain changes the scale.
pub const MIN_MARGIN: f32 = 40.0;

impl Default for Policy {
    fn default() -> Self {
        Policy {
            suppress: true,
            add: true,
            min_margin: MIN_MARGIN,
        }
    }
}

/// [`refine`] with an explicit policy.
pub fn refine_with(
    rows: &[&str],
    joins: &[usize],
    columns: Option<usize>,
    labelling: &Labelling,
    regex: &[Command],
    policy: &Policy,
) -> (Vec<Command>, Changes) {
    let labels = &labelling.labels[..];
    let min_margin = policy.min_margin;
    let sure = |row: usize| labelling.margins.get(row).is_some_and(|m| *m >= min_margin);
    let ours: Vec<Command> = label_commands(rows, joins, labels, columns)
        .into_iter()
        .filter(|c| c.first_row().is_some_and(sure))
        .collect();
    let mut changes = Changes::default();
    let mut kept: Vec<Command> = Vec::new();
    let mut used = vec![false; ours.len()];
    for cmd in regex {
        let Some(row) = cmd.first_row() else { continue };
        let label = labels.get(row).copied().unwrap_or(RowLabel::Output);
        let first = cmd.spans[0];
        let overlapping = ours.iter().position(|o| {
            o.spans.iter().any(|s| {
                s.row == row
                    && s.start < first.start + first.length
                    && first.start < s.start + s.length
            })
        });
        if !sure(row) {
            kept.push(cmd.clone());
            continue;
        }
        match label {
            RowLabel::Output | RowLabel::Chrome | RowLabel::Blank => {
                // Only a command that fills the row: output shows commands
                // too (`RUN …`, "To update, run: …", a prompt's `!cmd`), and
                // those start mid-row.
                let text = rows.get(row).copied().unwrap_or("");
                let ticked = first.start > 0 && utf16_char_before(text, first.start) == Some('`');
                let whole = first.start <= content_start(text);
                if ticked || !whole || !policy.suppress {
                    kept.push(cmd.clone());
                } else {
                    changes.suppressed += 1;
                }
            }
            RowLabel::Prose => kept.push(cmd.clone()),
            RowLabel::Command | RowLabel::Code | RowLabel::Continuation => match overlapping {
                Some(k) => {
                    let o = &ours[k];
                    // The regex started inside a prompt (`proj git:(main) npm run build`)
                    // where we found a known command after it.
                    let inside_prompt = label == RowLabel::Command
                        && o.spans[0].row == first.row
                        && o.spans[0].start > first.start
                        && cmd.value.ends_with(o.value.as_str())
                        && rows
                            .get(row)
                            .and_then(|r| text::prompt(r))
                            .is_some_and(|p| p.strength == 3);
                    if inside_prompt && !used[k] {
                        changes.replaced += 1;
                        kept.push(o.clone());
                    } else {
                        kept.push(cmd.clone());
                    }
                    used[k] = true;
                }
                None => kept.push(cmd.clone()),
            },
        }
    }
    for (k, o) in ours.iter().enumerate() {
        if used[k] || !policy.add || !addable(o, rows) {
            continue;
        }
        let overlaps = kept.iter().any(|c| {
            c.spans.iter().any(|a| {
                o.spans.iter().any(|b| {
                    a.row == b.row && a.start < b.start + b.length && b.start < a.start + a.length
                })
            })
        });
        if !overlaps {
            changes.added += 1;
            kept.push(o.clone());
        }
    }
    kept.sort_by_key(|c| {
        c.spans
            .first()
            .map(|s| (s.row, s.start))
            .unwrap_or((usize::MAX, 0))
    });
    kept.dedup();
    (kept, changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use RowLabel::{Code, Continuation, Output, Prose};
    const CMD: RowLabel = RowLabel::Command;

    fn sure(labels: &[RowLabel]) -> Labelling {
        Labelling {
            labels: labels.to_vec(),
            margins: vec![f32::INFINITY; labels.len()],
        }
    }

    fn cmd(value: &str, row: usize, start: usize) -> Command {
        Command {
            value: value.into(),
            spans: vec![Span {
                row,
                start,
                length: text::utf16_len(value),
            }],
        }
    }

    #[test]
    fn prompt_and_continuations() {
        let rows = [
            "user@devbox:~/svc$ docker run --rm \\",
            ">   -p 8080:8080 \\",
            ">   example/svc:1",
            "Unable to find image",
        ];
        let labels = [CMD, Continuation, Continuation, Output];
        let got = label_commands(&rows, &[], &labels, None);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].value, "docker run --rm -p 8080:8080 example/svc:1");
        assert_eq!(
            got[0].spans.iter().map(|s| s.row).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(got[0].spans[1].start, 4);
    }

    #[test]
    fn real_line_breaks_stay_and_wraps_join() {
        let rows = ["  for f in a b; do", "    echo \"$f\"", "  done"];
        let got = label_commands(&rows, &[], &[Code, Continuation, Continuation], Some(80));
        assert_eq!(got[0].value, "for f in a b; do\necho \"$f\"\ndone");
        let rows = [
            "  echo | openssl s_client -connect shop.example.invalid:443 2>/dev/null | openssl x509",
            "  -noout -enddate",
        ];
        let got = label_commands(&rows, &[], &[Code, Continuation], Some(90));
        assert_eq!(
            got[0].value,
            "echo | openssl s_client -connect shop.example.invalid:443 2>/dev/null | openssl x509 -noout -enddate"
        );
        let rows = ["$ cat <<EOF > a.toml", "> x = 1", "> EOF"];
        let got = label_commands(&rows, &[], &[CMD, Continuation, Continuation], Some(80));
        assert_eq!(got[0].value, "cat <<EOF > a.toml\nx = 1\nEOF");
    }

    #[test]
    fn soft_wraps_join_without_space() {
        let rows = ["$ git fetch origin && git reb", "ase -i origin/main"];
        let got = label_commands(&rows, &[0], &[CMD, Continuation], None);
        assert_eq!(
            got[0].value,
            "git fetch origin && git rebase -i origin/main"
        );
    }

    #[test]
    fn bash_tool_calls_end_at_their_parenthesis() {
        let rows = [
            "● Bash(npm run test:int",
            "      -- --runInBand 2>&1 | tail -40)",
            "  ⎿  PASS",
        ];
        let got = label_commands(&rows, &[], &[CMD, Continuation, Output], None);
        assert_eq!(
            got[0].value,
            "npm run test:int -- --runInBand 2>&1 | tail -40"
        );
        let one = label_commands(&["● Bash(git log --oneline -5)"], &[], &[CMD], None);
        assert_eq!(one[0].value, "git log --oneline -5");
    }

    #[test]
    fn boxed_code_rows() {
        let rows = [
            "│   rm -rf target && cargo build   │",
            "│   --release                      │",
        ];
        let got = label_commands(&rows, &[], &[Code, Continuation], None);
        assert_eq!(got[0].value, "rm -rf target && cargo build --release");
    }

    #[test]
    fn merge_suppresses_replaces_and_adds() {
        let rows = [
            "➜  webapp git:(main) ✗ ls -la",
            "test result: ok. 3 passed",
            "  1. Run `cargo test` first",
            "    ./scripts/setup.sh --with-fixtures",
        ];
        let labels = [CMD, Output, Prose, Code];
        let regex = vec![
            cmd("webapp git:(main) ✗ ls -la", 0, 3),
            cmd("test result: ok. 3 passed", 1, 0),
            cmd("cargo test", 2, 10),
        ];
        let (got, changes) = refine(&rows, &[], None, &sure(&labels), &regex);
        let values: Vec<&str> = got.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(
            values,
            vec!["ls -la", "cargo test", "./scripts/setup.sh --with-fixtures"]
        );
        assert_eq!(
            changes,
            Changes {
                suppressed: 1,
                replaced: 1,
                added: 1
            }
        );
    }

    #[test]
    fn the_regex_keeps_its_continuations() {
        let rows = ["$ git branch -a", "  feature/login-form", "* main"];
        let labels = [CMD, Continuation, Output];
        let regex = vec![cmd("git branch -a", 0, 2)];
        let (got, changes) = refine(&rows, &[], None, &sure(&labels), &regex);
        assert_eq!(
            got.iter().map(|c| c.value.as_str()).collect::<Vec<_>>(),
            vec!["git branch -a"]
        );
        assert_eq!(changes, Changes::default());
    }

    #[test]
    fn unsure_rows_leave_the_regex_alone() {
        let rows = ["test result: ok. 3 passed", "    ./scripts/setup.sh"];
        let labelling = Labelling {
            labels: vec![Output, Code],
            margins: vec![0.5, 0.5],
        };
        let regex = vec![cmd("test result: ok. 3 passed", 0, 0)];
        let (got, changes) = refine_with(
            &rows,
            &[],
            None,
            &labelling,
            &regex,
            &Policy {
                min_margin: 1.0,
                ..Policy::default()
            },
        );
        assert_eq!(got, regex);
        assert_eq!(changes, Changes::default());
    }

    #[test]
    fn right_prompts_are_not_part_of_the_command() {
        let rows = ["~/src/webapp main* ❯ git push origin main              10:44:02"];
        assert_eq!(
            label_commands(&rows, &[], &[CMD], None)[0].value,
            "git push origin main"
        );
        assert_eq!(right_prompt("printf '%s    1'"), "printf '%s    1'".len());
    }

    #[test]
    fn commands_shown_in_output() {
        let rows = [
            " => [2/6] RUN apt-get update && apt-get install -y curl          18.4s",
            "Step 3/7 : RUN pip install -r requirements.txt",
            "RUN npm ci && npm run build",
            "      - run: npm test",
            "cc -O2 -Wall -c -o src/server.o src/server.c",
        ];
        let got: Vec<String> = label_commands(&rows, &[], &[Code; 5], None)
            .into_iter()
            .map(|c| c.value)
            .collect();
        assert_eq!(
            got,
            vec![
                "apt-get update && apt-get install -y curl",
                "pip install -r requirements.txt",
                "npm ci && npm run build",
                "npm test",
                "cc -O2 -Wall -c -o src/server.o src/server.c",
            ]
        );
    }

    #[test]
    fn list_items_count_as_whole_rows() {
        assert_eq!(content_start("  1. Move the retry limit"), 5);
        assert_eq!(content_start("- git status"), 2);
        assert_eq!(content_start("│ text │"), 2);
    }
}
