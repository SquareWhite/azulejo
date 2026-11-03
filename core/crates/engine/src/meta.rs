//! What runs in terminal tiles, and the header every tile shows (F28).
//!
//! herdr reports a pane's title and working directory in `pane_updated`
//! events; what runs in its foreground has no event, so the core polls
//! `pane.process_info` for bound panes ([`follow_processes`]). From those the
//! core works out each tile's header text, so every shell shows the same.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use azulejo_herdr as herdr;
use azulejo_model::{PaneMeta, Session, TileContent, TileId};

use crate::link::{HerdrPort, Internal, Machine};

/// How often foreground processes are polled.
pub(crate) const PROCESS_POLL: Duration = Duration::from_millis(1000);

/// Longest command line a header shows.
const MAX_COMMAND: usize = 120;

/// The home folder, for `~` in headers.
pub(crate) fn home() -> Option<String> {
    std::env::var("HOME").ok().filter(|h| !h.is_empty())
}

/// Brings every tile's header up to date. Returns whether any changed.
pub(crate) fn refresh_headers(session: &mut Session) -> bool {
    let home = home();
    let mut changed = false;
    for window in &mut session.windows {
        for ws in &mut window.workspaces {
            for tab in &mut ws.tabs {
                for tile in &mut tab.tiles {
                    let header = tile.compute_header(home.as_deref());
                    if tile.header.as_ref() != Some(&header) {
                        tile.header = Some(header);
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

/// Terminal tiles bound to `pane_id` on `machine` (pane IDs repeat across
/// machines; ADR 0011).
fn tiles_on(session: &Session, machine: &Machine, pane_id: &str) -> Vec<TileId> {
    session
        .tiles()
        .filter_map(|(_, tile)| match &tile.content {
            TileContent::Terminal { herdr: Some(h), .. }
                if h.machine == *machine && h.pane_id == pane_id =>
            {
                Some(tile.id)
            }
            _ => None,
        })
        .collect()
}

/// Applies a change to the pane metadata of every tile on `pane_id`.
/// Returns whether anything changed.
fn update(
    session: &mut Session,
    machine: &Machine,
    pane_id: &str,
    f: impl Fn(&mut PaneMeta),
) -> bool {
    let mut changed = false;
    for id in tiles_on(session, machine, pane_id) {
        if let Ok(tile) = session.tile_mut(id) {
            let mut meta = tile.pane.clone().unwrap_or_default();
            f(&mut meta);
            if tile.pane.as_ref() != Some(&meta) {
                tile.pane = Some(meta);
                changed = true;
            }
        }
    }
    changed
}

/// herdr's title and working directory for a pane (`pane_updated`, surveys).
pub(crate) fn apply_info(
    session: &mut Session,
    machine: &Machine,
    pane_id: &str,
    title: Option<String>,
    cwd: Option<String>,
) -> bool {
    let title = title.filter(|t| !t.trim().is_empty());
    update(session, machine, pane_id, |meta| {
        meta.title = title.clone();
        if cwd.is_some() {
            meta.cwd = cwd.clone();
        }
    })
}

/// What runs in a pane's foreground: the shell (`command: None`) or a
/// command. `shell: None` keeps the shell name already known.
pub(crate) fn apply_process(
    session: &mut Session,
    machine: &Machine,
    pane_id: &str,
    shell: Option<String>,
    command: Option<String>,
    cwd: Option<String>,
) -> bool {
    update(session, machine, pane_id, |meta| {
        // Seen in the foreground beats what's known, which beats `$SHELL`.
        meta.shell = shell
            .clone()
            .or_else(|| meta.shell.clone())
            .or_else(default_shell);
        if cwd.is_some() {
            meta.cwd = cwd.clone();
        }
        meta.command = command.clone();
    })
}

/// Title and cwd of every bound pane in a survey.
///
/// `fresher(pane)`: that pane's title or cwd came in (herdr's stream, the
/// process poller) after the survey was requested, so the survey's snapshot
/// may predate it; such panes keep what they have.
pub(crate) fn refresh_from_survey(
    session: &mut Session,
    machine: &Machine,
    survey: &herdr::Survey,
    fresher: impl Fn(&str) -> bool,
) -> bool {
    let mut changed = false;
    for pane in survey.panes.iter().filter(|p| !fresher(p.pane_id.as_str())) {
        changed |= apply_info(
            session,
            machine,
            pane.pane_id.as_str(),
            pane.terminal_title.clone(),
            pane.foreground_cwd.clone().or_else(|| pane.cwd.clone()),
        );
    }
    changed
}

/// A process's name without a login shell's leading dash (`-fish` → `fish`).
fn process_name(entry: &herdr::ProcessEntry) -> String {
    let name = if entry.name.is_empty() {
        entry.cmdline.split_whitespace().next().unwrap_or("")
    } else {
        &entry.name
    };
    let name = name.rsplit('/').next().unwrap_or(name);
    name.trim_start_matches('-').to_owned()
}

/// A command line as a header shows it: the program's name without its
/// folder, then its arguments, cut to a sensible length.
fn command_line(entry: &herdr::ProcessEntry) -> String {
    let line = entry.cmdline.trim();
    let line = if line.is_empty() { &entry.name } else { line };
    let (program, rest) = match line.split_once(' ') {
        Some((p, r)) => (p, Some(r)),
        None => (line, None),
    };
    let program = program.rsplit('/').next().unwrap_or(program);
    let mut out = match rest {
        Some(rest) => format!("{program} {}", rest.trim()),
        None => program.to_owned(),
    };
    if out.chars().count() > MAX_COMMAND {
        out = out.chars().take(MAX_COMMAND - 1).collect::<String>() + "…";
    }
    out
}

/// What a pane's process info means for its header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Described {
    /// The shell's name, when the shell is in the foreground.
    pub shell: Option<String>,
    /// The command line, when something else is.
    pub command: Option<String>,
    /// The foreground process's working directory.
    pub cwd: Option<String>,
}

pub(crate) fn describe(info: &herdr::ProcessInfo) -> Described {
    match info.foreground() {
        None => Described::default(),
        Some(fg) if info.shell_in_foreground() => Described {
            shell: Some(process_name(fg)),
            command: None,
            cwd: fg.cwd.clone(),
        },
        Some(fg) => Described {
            shell: None,
            command: Some(command_line(fg)),
            cwd: fg.cwd.clone(),
        },
    }
}

/// The user's shell from `$SHELL`, as a fallback until a pane's shell has
/// been seen in the foreground.
fn default_shell() -> Option<String> {
    std::env::var("SHELL")
        .ok()
        .and_then(|s| s.rsplit('/').next().map(str::to_owned))
        .filter(|s| !s.is_empty())
}

/// Polls `pane.process_info` for the bound panes in `want` and reports
/// changes as [`Internal::PaneProcess`]. Stops when `want` closes or the port
/// can't report processes at all.
pub(crate) async fn follow_processes<P: HerdrPort>(
    machine: Machine,
    port: Arc<P>,
    mut want: tokio::sync::watch::Receiver<Vec<String>>,
    // Poll now rather than at the next tick: herdr said a pane changed (a
    // shell retitles itself when a command starts).
    poke: Arc<tokio::sync::Notify>,
    out: impl Fn(Internal) + Send + 'static,
) {
    // What each pane last reported.
    let mut last: HashMap<String, Described> = HashMap::new();
    let mut ticker = tokio::time::interval(PROCESS_POLL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = want.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            _ = ticker.tick() => {}
            _ = poke.notified() => {
                // Let the process the title announced get going.
                tokio::time::sleep(Duration::from_millis(30)).await;
                ticker.reset();
            }
        }
        let panes = want.borrow_and_update().clone();
        last.retain(|pane, _| panes.contains(pane));
        let mut unsupported = !panes.is_empty();
        for pane in panes {
            let info = match port.process_info(pane.clone()).await {
                Ok(info) => {
                    unsupported = false;
                    info
                }
                Err(e) => {
                    unsupported &= e == UNSUPPORTED;
                    continue;
                }
            };
            let mut now = describe(&info);
            // Unknown stays unknown here: the engine keeps the shell the tile
            // already knows (e.g. from before a relaunch) and only then falls
            // back to `$SHELL`.
            let known = last.get(&pane).and_then(|d| d.shell.clone());
            now.shell = now.shell.or(known);
            if last.get(&pane) != Some(&now) {
                last.insert(pane.clone(), now.clone());
                out(Internal::PaneProcess {
                    machine: machine.clone(),
                    pane_id: pane,
                    shell: now.shell,
                    command: now.command,
                    cwd: now.cwd,
                });
            }
        }
        if unsupported {
            return;
        }
    }
}

/// What a [`HerdrPort`] without process info answers.
pub(crate) const UNSUPPORTED: &str = "process info isn't supported";

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pid: u32, name: &str, cmdline: &str) -> herdr::ProcessEntry {
        herdr::ProcessEntry {
            pid,
            name: name.into(),
            cmdline: cmdline.into(),
            cwd: None,
        }
    }

    #[test]
    fn describes_shell_and_command() {
        let idle = herdr::ProcessInfo {
            shell_pid: Some(10),
            foreground_process_group_id: Some(10),
            foreground_processes: vec![entry(10, "fish", "-fish")],
        };
        assert_eq!(describe(&idle).shell.as_deref(), Some("fish"));
        assert_eq!(describe(&idle).command, None);

        let busy = herdr::ProcessInfo {
            shell_pid: Some(10),
            foreground_process_group_id: Some(20),
            foreground_processes: vec![
                entry(21, "cargo", "cargo build"),
                entry(20, "just", "/opt/homebrew/bin/just dogfood"),
            ],
        };
        assert_eq!(describe(&busy).shell, None);
        assert_eq!(describe(&busy).command.as_deref(), Some("just dogfood"));
    }

    #[test]
    fn a_busy_pane_keeps_the_shell_it_had() {
        use azulejo_model::{HerdrRef, Tile};
        let mut session = Session::new();
        let (_, ws) = session.create_window("w", None);
        let mut tile = Tile::new(TileContent::terminal());
        if let TileContent::Terminal { herdr, .. } = &mut tile.content {
            *herdr = Some(HerdrRef {
                pane_id: "w1:p1".into(),
                terminal_id: "term_1".into(),
                tab_id: "w1:t1".into(),
                machine: None,
            });
        }
        tile.pane = Some(PaneMeta {
            shell: Some("fish".into()),
            ..PaneMeta::default()
        });
        let id = tile.id;
        session.create_tab(ws, tile, None).unwrap();

        // After a relaunch the poller hasn't seen the shell yet.
        // The same pane ID on another machine isn't this tile (ADR 0011).
        assert!(!apply_process(
            &mut session,
            &Some("remote".into()),
            "w1:p1",
            None,
            Some("vim".into()),
            None
        ));
        assert!(apply_process(
            &mut session,
            &None,
            "w1:p1",
            None,
            Some("cargo test".into()),
            None
        ));
        let meta = session.tile(id).unwrap().pane.clone().unwrap();
        assert_eq!(meta.shell.as_deref(), Some("fish"));
        assert_eq!(meta.command.as_deref(), Some("cargo test"));
    }

    #[test]
    fn long_commands_are_cut() {
        let long = "x".repeat(400);
        let e = entry(1, "echo", &format!("echo {long}"));
        let shown = command_line(&e);
        assert_eq!(shown.chars().count(), MAX_COMMAND);
        assert!(shown.ends_with('…'));
    }
}
