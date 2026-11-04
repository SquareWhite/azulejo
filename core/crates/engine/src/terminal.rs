//! Terminal I/O (F20). The engine only finds the herdr pane behind a tile
//! ([`crate::Engine::terminal_pane`]); [`run`] then talks to herdr off the
//! engine's thread, so a slow read never blocks layout commands.
//!
//! Search runs here, over `recent_unwrapped` output: herdr's own
//! `pane.copy_search` drives its interactive copy mode (a cursor and content
//! revision per step) and only matches case-sensitively. With `locate` (the
//! find bar), the lines are matched to rows of the `recent` read as well, so
//! a match can be scrolled into view.

use azulejo_herdr as herdr;
use azulejo_model::HerdrRef;
use azulejo_protocol::{Command, Error, ErrorCode, ReadSource, Reply, ScrollState, SearchMatch};

use crate::link::HerdrPort;

/// Whether `command` is terminal I/O that [`run`] handles.
pub fn is_io(command: &Command) -> bool {
    matches!(
        command,
        Command::TerminalSendText(_)
            | Command::TerminalSendKeys(_)
            | Command::TerminalRead(_)
            | Command::TerminalScroll(_)
            | Command::TerminalSearch(_)
    )
}

/// The tile a terminal I/O command targets (`None` means the focused tile).
pub fn target(command: &Command) -> Option<azulejo_model::TileId> {
    match command {
        Command::TerminalSendText(p) => p.tile,
        Command::TerminalSendKeys(p) => p.tile,
        Command::TerminalRead(p) => p.tile,
        Command::TerminalScroll(p) => p.tile,
        Command::TerminalSearch(p) => p.tile,
        _ => None,
    }
}

/// Runs a terminal I/O command against `pane`.
pub async fn run<P: HerdrPort>(port: &P, pane: HerdrRef, command: Command) -> Result<Reply, Error> {
    let pane = pane.pane_id;
    match command {
        Command::TerminalSendText(p) => {
            port.send_text(pane, p.text).await.map_err(unavailable)?;
            Ok(Reply::Ok)
        }
        Command::TerminalSendKeys(p) => {
            if p.keys.is_empty() {
                return Err(Error::new(ErrorCode::InvalidParams, "`keys` is empty"));
            }
            port.send_keys(pane, p.keys).await.map_err(unavailable)?;
            Ok(Reply::Ok)
        }
        Command::TerminalRead(p) => {
            let read = port
                .read(pane, source(p.source), p.lines)
                .await
                .map_err(unavailable)?;
            Ok(Reply::Text { text: read.text })
        }
        Command::TerminalScroll(p) => {
            let s = port
                .scroll(pane, p.offset_from_bottom)
                .await
                .map_err(unavailable)?;
            Ok(Reply::Scroll(ScrollState {
                offset_from_bottom: s.offset_from_bottom,
                max_offset_from_bottom: s.max_offset_from_bottom,
                viewport_rows: s.viewport_rows,
            }))
        }
        Command::TerminalSearch(p) => {
            if p.query.is_empty() {
                return Err(Error::new(ErrorCode::InvalidParams, "`query` is empty"));
            }
            if !p.locate {
                let read = port
                    .read(pane, herdr::ReadSource::RecentUnwrapped, p.lines)
                    .await
                    .map_err(unavailable)?;
                return Ok(Reply::Matches {
                    matches: search(&read.text, &p.query, p.case_sensitive),
                    truncated: read.truncated,
                    scroll: None,
                });
            }
            // Locating: the whole buffer (herdr counts `lines` in rows for
            // both sources), as typed and as on screen, so lines can be
            // matched to rows from the top.
            let s = port.scroll_state(pane.clone()).await.map_err(unavailable)?;
            let all = s.max_offset_from_bottom + u64::from(s.viewport_rows);
            let lines = p
                .lines
                .unwrap_or_else(|| u32::try_from(all).unwrap_or(u32::MAX));
            let read = port
                .read(
                    pane.clone(),
                    herdr::ReadSource::RecentUnwrapped,
                    Some(lines),
                )
                .await
                .map_err(unavailable)?;
            let screen = port
                .read(pane, herdr::ReadSource::Recent, Some(lines))
                .await
                .map_err(unavailable)?;
            let scroll = ScrollState {
                offset_from_bottom: s.offset_from_bottom,
                max_offset_from_bottom: s.max_offset_from_bottom,
                viewport_rows: s.viewport_rows,
            };
            let mut matches = search(&read.text, &p.query, p.case_sensitive);
            // Cut short, the reads start at an unknown row.
            if !read.truncated && !screen.truncated {
                let typed: Vec<&str> = read.text.lines().collect();
                let rows: Vec<&str> = screen.text.lines().collect();
                let starts = line_rows(&typed, &rows);
                for m in &mut matches {
                    m.row = starts.get(m.line).copied().flatten();
                    m.offset_from_bottom = m.row.map(|row| centre(row, &scroll));
                }
            }
            Ok(Reply::Matches {
                matches,
                truncated: read.truncated,
                scroll: Some(scroll),
            })
        }
        other => Err(Error::new(
            ErrorCode::Internal,
            format!("`{}` isn't terminal I/O", other.method()),
        )),
    }
}

/// Lines of `text` containing `query`, oldest first.
pub fn search(text: &str, query: &str, case_sensitive: bool) -> Vec<SearchMatch> {
    let needle = if case_sensitive {
        query.to_owned()
    } else {
        query.to_lowercase()
    };
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            if case_sensitive {
                line.contains(&needle)
            } else {
                line.to_lowercase().contains(&needle)
            }
        })
        .map(|(line, text)| SearchMatch {
            line,
            text: text.to_owned(),
            row: None,
            offset_from_bottom: None,
        })
        .collect()
}

/// The first buffer row of each of `lines` (soft wraps joined), found by
/// walking `rows` (as on screen) alongside them from the top. Whitespace is
/// ignored when comparing, since herdr trims it at the end of rows. From the
/// first line that doesn't line up (output arrived between the two reads),
/// the rest are `None`.
pub fn line_rows(lines: &[&str], rows: &[&str]) -> Vec<Option<u64>> {
    fn squash(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }
    let mut out = Vec::with_capacity(lines.len());
    let mut next = 0usize;
    for line in lines {
        let want = squash(line);
        let start = next;
        let mut have = String::new();
        let lined_up = loop {
            let Some(row) = rows.get(next) else {
                break false;
            };
            have.push_str(&squash(row));
            next += 1;
            if have == want {
                break true;
            }
            if !want.starts_with(&have) {
                break false;
            }
        };
        if !lined_up {
            out.resize(lines.len(), None);
            return out;
        }
        out.push(Some(start as u64));
    }
    out
}

/// The `terminal.scroll` offset that puts buffer row `row` mid-view.
pub fn centre(row: u64, view: &ScrollState) -> u64 {
    let top = row.saturating_sub(u64::from(view.viewport_rows / 2));
    view.max_offset_from_bottom.saturating_sub(top)
}

fn source(s: ReadSource) -> herdr::ReadSource {
    match s {
        ReadSource::Visible => herdr::ReadSource::Visible,
        ReadSource::Recent => herdr::ReadSource::Recent,
        ReadSource::RecentUnwrapped => herdr::ReadSource::RecentUnwrapped,
    }
}

fn unavailable(e: String) -> Error {
    Error::new(ErrorCode::Unavailable, format!("herdr: {e}"))
}
