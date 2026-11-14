//! Terminal I/O (F20). The engine only finds the herdr pane behind a tile
//! ([`crate::Engine::terminal_pane`]); [`run`] then talks to herdr off the
//! engine's thread, so a slow read never blocks layout commands.
//!
//! Search runs here, over `recent_unwrapped` output: herdr's own
//! `pane.copy_search` drives its interactive copy mode (a cursor and content
//! revision per step), which suits an in-tile find bar later, not one-shot API
//! calls.

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
            let read = port
                .read(pane, herdr::ReadSource::RecentUnwrapped, p.lines)
                .await
                .map_err(unavailable)?;
            Ok(Reply::Matches {
                matches: search(&read.text, &p.query, p.case_sensitive),
                truncated: read.truncated,
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
        })
        .collect()
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
