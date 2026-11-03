//! Keeps terminal tiles and herdr in step (ADR 0002).
//!
//! The engine stays pure: [`effects`] turns events into herdr work, a worker
//! runs that work through a [`HerdrPort`] one item at a time, and the results
//! come back as [`Internal`] commands applied on the engine's thread.
//! [`reconcile`] does the same for events herdr sends us.
//!
//! herdr workspaces are created on demand, with the first terminal that needs
//! one: herdr always gives a new workspace a first pane, and that pane becomes
//! the terminal. Creating workspaces eagerly would leave a stray pane that
//! can't be killed without herdr deleting the (otherwise empty) workspace.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use azulejo_herdr as herdr;
use azulejo_model::{
    AgentStatus, HerdrRef, OnExit, Session, Tile, TileContent, TileId, WorkspaceId,
};
use azulejo_protocol::{Command, Event, TerminalBind, WorkspaceBind};
use tokio::sync::mpsc;

use crate::{Engine, adopt, agent, diff};

/// What the engine needs from herdr. [`herdr::Client`] implements it; tests
/// use a fake.
pub trait HerdrPort: Send + Sync + 'static {
    fn ensure_workspace(
        &self,
        existing: Option<String>,
        spec: herdr::WorkspaceSpec,
    ) -> impl Future<Output = Result<herdr::EnsuredWorkspace, String>> + Send;

    fn create_terminal(
        &self,
        spec: herdr::TerminalSpec,
    ) -> impl Future<Output = Result<herdr::TerminalRef, String>> + Send;

    fn kill_terminal(&self, pane_id: String) -> impl Future<Output = Result<(), String>> + Send;

    fn reresolve(
        &self,
        stored: herdr::TerminalRef,
    ) -> impl Future<Output = Result<herdr::Resolved, String>> + Send;

    /// What herdr is running, for adoption (ADR 0008). Must only read.
    fn survey(&self) -> impl Future<Output = Result<herdr::Survey, String>> + Send;

    /// herdr's lifecycle events. Called once, inside the runtime.
    fn events(&self) -> mpsc::Receiver<herdr::StreamItem>;

    /// Agent status changes for `panes` (herdr wants one subscription per
    /// pane). Called inside the runtime whenever the set of bound panes
    /// changes; dropping the receiver ends the subscription. A port without
    /// agent status returns a closed channel.
    fn agent_events(&self, panes: Vec<String>) -> mpsc::Receiver<herdr::StreamItem> {
        let _ = panes;
        mpsc::channel(1).1
    }

    // ----- terminal I/O (F20) -----

    fn send_text(
        &self,
        pane_id: String,
        text: String,
    ) -> impl Future<Output = Result<(), String>> + Send;

    fn send_keys(
        &self,
        pane_id: String,
        keys: Vec<String>,
    ) -> impl Future<Output = Result<(), String>> + Send;

    fn read(
        &self,
        pane_id: String,
        source: herdr::ReadSource,
        lines: Option<u32>,
    ) -> impl Future<Output = Result<herdr::ReadResult, String>> + Send;

    fn scroll(
        &self,
        pane_id: String,
        offset_from_bottom: u64,
    ) -> impl Future<Output = Result<herdr::ScrollInfo, String>> + Send;

    /// The herdr session socket this port talks to, if it has one.
    fn socket(&self) -> Option<std::path::PathBuf> {
        None
    }
}

impl HerdrPort for herdr::Client {
    async fn ensure_workspace(
        &self,
        existing: Option<String>,
        spec: herdr::WorkspaceSpec,
    ) -> Result<herdr::EnsuredWorkspace, String> {
        let existing = existing.map(herdr::WorkspaceId);
        herdr::Client::ensure_workspace(self, existing.as_ref(), &spec)
            .await
            .map_err(|e| e.to_string())
    }

    async fn create_terminal(
        &self,
        spec: herdr::TerminalSpec,
    ) -> Result<herdr::TerminalRef, String> {
        herdr::Client::create_terminal(self, &spec)
            .await
            .map_err(|e| e.to_string())
    }

    async fn kill_terminal(&self, pane_id: String) -> Result<(), String> {
        herdr::Client::kill_terminal(self, &herdr::PaneId(pane_id))
            .await
            .map_err(|e| e.to_string())
    }

    async fn reresolve(&self, stored: herdr::TerminalRef) -> Result<herdr::Resolved, String> {
        herdr::Client::reresolve(self, &stored)
            .await
            .map_err(|e| e.to_string())
    }

    async fn survey(&self) -> Result<herdr::Survey, String> {
        herdr::Client::survey(self).await.map_err(|e| e.to_string())
    }

    async fn send_text(&self, pane_id: String, text: String) -> Result<(), String> {
        herdr::Client::send_text(self, &herdr::PaneId(pane_id), &text)
            .await
            .map_err(|e| e.to_string())
    }

    async fn send_keys(&self, pane_id: String, keys: Vec<String>) -> Result<(), String> {
        herdr::Client::send_keys(self, &herdr::PaneId(pane_id), &keys)
            .await
            .map_err(|e| e.to_string())
    }

    async fn read(
        &self,
        pane_id: String,
        source: herdr::ReadSource,
        lines: Option<u32>,
    ) -> Result<herdr::ReadResult, String> {
        herdr::Client::read(self, &herdr::PaneId(pane_id), source, lines)
            .await
            .map_err(|e| e.to_string())
    }

    async fn scroll(
        &self,
        pane_id: String,
        offset_from_bottom: u64,
    ) -> Result<herdr::ScrollInfo, String> {
        herdr::Client::scroll(self, &herdr::PaneId(pane_id), offset_from_bottom)
            .await
            .map(|pane| pane.scroll.unwrap_or_default())
            .map_err(|e| e.to_string())
    }

    fn socket(&self) -> Option<std::path::PathBuf> {
        Some(self.socket_path().to_path_buf())
    }

    fn events(&self) -> mpsc::Receiver<herdr::StreamItem> {
        forward(herdr::Client::subscribe(
            self,
            herdr::Subscription::lifecycle(),
        ))
    }

    fn agent_events(&self, panes: Vec<String>) -> mpsc::Receiver<herdr::StreamItem> {
        forward(herdr::Client::subscribe(
            self,
            panes
                .into_iter()
                .map(|p| herdr::Subscription::AgentStatus(herdr::PaneId(p)))
                .collect(),
        ))
    }
}

/// Pumps an event stream into a channel; the stream closes with the channel.
fn forward(mut stream: herdr::EventStream) -> mpsc::Receiver<herdr::StreamItem> {
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        while let Some(item) = stream.recv().await {
            if tx.send(item).await.is_err() {
                break;
            }
        }
    });
    rx
}

/// herdr work the engine asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Give a terminal tile a herdr pane (and its workspace one if needed).
    CreateTerminal {
        tile: TileId,
        workspace: WorkspaceId,
        herdr_workspace: Option<String>,
        workspace_name: String,
        cwd: Option<String>,
        label: Option<String>,
        /// Extra environment from the tile, on top of `AZULEJO_*`.
        env: Vec<(String, String)>,
        /// A line typed into the new pane's shell (a layout file's `command`).
        command: Option<String>,
    },
    /// Kill a closed tile's pane (F14: only when the user asked to).
    KillTerminal { pane_id: String },
    /// Check what became of a stored pane (relaunch, reconnect).
    Reresolve {
        tile: TileId,
        stored: HerdrRef,
        herdr_workspace: Option<String>,
    },
    /// Read what herdr runs, to adopt tabs nobody here knows (ADR 0008).
    Survey,
}

/// Results fed back to the engine. Not part of the public API.
#[derive(Debug, Clone, PartialEq)]
pub enum Internal {
    BindWorkspace {
        workspace: WorkspaceId,
        herdr: String,
    },
    BindTerminal {
        tile: TileId,
        herdr: HerdrRef,
    },
    /// The tile's pane is gone; the shell offers Respawn or Close.
    MarkEnded {
        tile: TileId,
    },
    /// The tile's pane was closed outside Azulejo (herdr TUI, phone, CLI):
    /// the tile goes too (F15).
    ClosedElsewhere {
        tile: TileId,
    },
    /// A herdr workspace closed: its tiles go and the binding is dropped, so
    /// a later workspace reusing the ID isn't mistaken for it.
    HerdrWorkspaceClosed {
        herdr: String,
    },
    /// herdr reported the agent in a pane (F16).
    AgentStatus {
        pane_id: String,
        agent: Option<String>,
        status: AgentStatus,
        at_ms: u64,
    },
    /// What herdr runs; adopt what's new.
    Surveyed(Box<herdr::Survey>),
    /// The survey failed (logged); nothing to adopt this time.
    SurveyFailed,
}

impl Engine {
    /// Applies a result from the herdr worker. The target may have closed in
    /// the meantime; that's not an error, nothing happens.
    pub fn apply_internal(&mut self, internal: Internal) -> Vec<Event> {
        match internal {
            Internal::BindWorkspace { workspace, herdr } => self
                .execute(Command::WorkspaceBind(WorkspaceBind {
                    workspace,
                    herdr_workspace: herdr,
                }))
                .map(|o| o.events)
                .unwrap_or_default(),
            Internal::BindTerminal { tile, herdr } => {
                let unchanged = matches!(
                    self.session.tile(tile).map(|t| &t.content),
                    Ok(TileContent::Terminal { herdr: Some(h), ended: false, .. }) if *h == herdr
                );
                if unchanged {
                    return Vec::new();
                }
                self.execute(Command::TerminalBind(TerminalBind { tile, herdr }))
                    .map(|o| o.events)
                    .unwrap_or_default()
            }
            Internal::MarkEnded { tile } => {
                let before = self.session.clone();
                match self.session.tile_mut(tile).map(|t| &mut t.content) {
                    Ok(TileContent::Terminal { ended, .. }) if !*ended => *ended = true,
                    _ => return Vec::new(),
                }
                diff(&before, &self.session, false)
            }
            Internal::ClosedElsewhere { tile } => {
                let before = self.session.clone();
                if self.session.close_tile(tile).is_err() {
                    return Vec::new();
                }
                diff(&before, &self.session, false)
            }
            Internal::HerdrWorkspaceClosed { herdr } => {
                let before = self.session.clone();
                close_herdr_workspace(&mut self.session, &herdr);
                debug_assert_eq!(self.session.check_invariants(), Ok(()));
                diff(&before, &self.session, false)
            }
            Internal::AgentStatus {
                pane_id,
                agent,
                status,
                at_ms,
            } => {
                let before = self.session.clone();
                let tiles: Vec<TileId> = bound_tiles(&self.session)
                    .filter(|(_, _, h)| h.pane_id == pane_id)
                    .map(|(_, id, _)| id)
                    .collect();
                let mut changed = false;
                for id in tiles {
                    if let Ok(tile) = self.session.tile_mut(id) {
                        changed |= agent::update(tile, agent.clone(), status, at_ms);
                    }
                }
                if !changed {
                    return Vec::new();
                }
                diff(&before, &self.session, false)
            }
            Internal::Surveyed(survey) => {
                let before = self.session.clone();
                let placeholder = self.placeholder.take();
                let mut changed = adopt::adopt(&mut self.session, &survey, placeholder);
                changed |= agent::refresh(&mut self.session, &survey, agent::now_ms());
                if !changed {
                    return Vec::new();
                }
                debug_assert_eq!(self.session.check_invariants(), Ok(()));
                diff(&before, &self.session, false)
            }
            Internal::SurveyFailed => {
                self.placeholder = None;
                Vec::new()
            }
        }
    }
}

/// herdr work implied by `events`, given the session after them.
pub fn effects(events: &[Event], session: &Session) -> Vec<Effect> {
    let mut out = Vec::new();
    for event in events {
        match event {
            Event::TileCreated { tile, .. } => {
                if let Some(effect) = create_effect(session, tile) {
                    out.push(effect);
                }
            }
            Event::TileClosed {
                tile, kill: true, ..
            } => {
                if let TileContent::Terminal {
                    herdr: Some(h),
                    ended: false,
                    ..
                } = &tile.content
                {
                    out.push(Effect::KillTerminal {
                        pane_id: h.pane_id.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// Work to bring every terminal tile in line with herdr: on startup, and
/// after the event stream lost events.
pub fn restore_effects(session: &Session) -> Vec<Effect> {
    session
        .tiles()
        .filter_map(|(path, tile)| match &tile.content {
            TileContent::Terminal {
                herdr: Some(stored),
                ended: false,
                ..
            } => {
                let (_, workspace, _) = session.ids_at(path);
                Some(Effect::Reresolve {
                    tile: tile.id,
                    stored: stored.clone(),
                    herdr_workspace: herdr_workspace(session, workspace),
                })
            }
            _ => create_effect(session, tile),
        })
        .collect()
}

fn herdr_workspace(session: &Session, workspace: WorkspaceId) -> Option<String> {
    session
        .workspace(workspace)
        .ok()
        .and_then(|w| w.herdr_workspace.clone())
}

/// Work to give a respawned tile (`terminal.respawn`) a fresh pane.
pub fn respawn_effect(session: &Session, tile: TileId) -> Option<Effect> {
    create_effect(session, session.tile(tile).ok()?)
}

fn create_effect(session: &Session, tile: &Tile) -> Option<Effect> {
    let TileContent::Terminal {
        cwd,
        command,
        env,
        herdr: None,
        ended: false,
        ..
    } = &tile.content
    else {
        return None;
    };
    let path = session.locate(tile.id).ok()?;
    let (_, workspace_id, _) = session.ids_at(path);
    let workspace = session.workspace(workspace_id).ok()?;
    Some(Effect::CreateTerminal {
        tile: tile.id,
        workspace: workspace_id,
        herdr_workspace: workspace.herdr_workspace.clone(),
        workspace_name: workspace.name.clone(),
        cwd: cwd.clone().or_else(|| workspace.cwd.clone()),
        label: tile.title.clone(),
        env: env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        command: command
            .as_deref()
            .and_then(crate::layout_file::command_line),
    })
}

/// What to do about something herdr reported.
#[derive(Debug, Default, PartialEq)]
pub struct Reaction {
    pub internals: Vec<Internal>,
    pub effects: Vec<Effect>,
    /// herdr has new or possibly missed tabs: survey and adopt (ADR 0008).
    pub survey: bool,
}

/// Matches herdr events to tiles (ADR 0009):
/// - `pane_exited`: the process ended (`exit`, ctrl+d, a finished command).
///   The tile closes, or with `on_exit = "keep"` (the tile's own, else
///   `default_on_exit` from config) stays ended and respawnable.
/// - `pane_closed`, `tab_closed`, `workspace_closed`: someone closed it in
///   herdr; the tile goes too (F15).
/// - New workspaces, tabs and panes nobody here knows ask for a survey, which
///   adopts them (ADR 0008; split-off panes land beside their sibling).
/// - Agent status updates the tile's agent (F16).
pub fn reconcile(item: &herdr::StreamItem, session: &Session, default_on_exit: OnExit) -> Reaction {
    use herdr::HerdrEvent as E;
    let mut reaction = Reaction::default();
    let bound = || bound_tiles(session);
    let close_where = |reaction: &mut Reaction, pred: &dyn Fn(&HerdrRef) -> bool| {
        for (_, tile, h) in bound() {
            if pred(h) {
                reaction.internals.push(Internal::ClosedElsewhere { tile });
            }
        }
    };
    match item {
        // Events before the stream came up are lost (herdr doesn't replay),
        // so check everything then too, not only after a reconnect.
        herdr::StreamItem::Connected | herdr::StreamItem::Resync => {
            reaction.effects = restore_effects(session)
                .into_iter()
                .filter(|e| matches!(e, Effect::Reresolve { .. }))
                .collect();
            reaction.survey = true;
        }
        herdr::StreamItem::Event(event) => match event {
            E::WorkspaceCreated(_) | E::TabCreated(_) => reaction.survey = true,
            // A pane split off elsewhere (F15); ours are bound already, or
            // will be before the survey runs (the worker goes in order).
            E::PaneCreated(pane) => {
                reaction.survey |= !bound().any(|(_, _, h)| {
                    h.terminal_id == pane.terminal_id.as_str()
                        || (h.pane_id == pane.pane_id.as_str() && h.tab_id == pane.tab_id.as_str())
                });
            }
            // The process ended (`exit`, ctrl+d, a crash): herdr drops the
            // pane (and its tab, if it was the only one) and says
            // `pane_exited` only. Like any terminal, the tile closes; with
            // `on_exit = "keep"` it stays, ended and respawnable.
            E::PaneExited { pane_id, .. } => {
                for (_, tile, h) in bound() {
                    if h.pane_id != pane_id.as_str() {
                        continue;
                    }
                    let on_exit = match session.tile(tile).map(|t| &t.content) {
                        Ok(TileContent::Terminal {
                            on_exit: Some(own), ..
                        }) => *own,
                        _ => default_on_exit,
                    };
                    reaction.internals.push(match on_exit {
                        OnExit::Close => Internal::ClosedElsewhere { tile },
                        OnExit::Keep => Internal::MarkEnded { tile },
                    });
                }
            }
            // Someone closed the pane, tab or workspace in herdr (its TUI, a
            // phone, the CLI, an agent): the tile goes with it.
            E::PaneClosed { pane_id, .. } => {
                close_where(&mut reaction, &|h| h.pane_id == pane_id.as_str());
            }
            // Closing a tab reports only the tab, not its panes.
            E::TabClosed { tab_id, .. } => {
                close_where(&mut reaction, &|h| h.tab_id == tab_id.as_str());
            }
            E::WorkspaceClosed { workspace_id } => {
                reaction.internals.push(Internal::HerdrWorkspaceClosed {
                    herdr: workspace_id.0.clone(),
                });
            }
            E::AgentStatusChanged {
                pane_id,
                agent,
                status,
                ..
            } => reaction.internals.push(Internal::AgentStatus {
                pane_id: pane_id.0.clone(),
                agent: agent.clone(),
                status: agent::from_herdr(*status),
                at_ms: agent::now_ms(),
            }),
            E::PaneMoved {
                previous_pane_id,
                pane,
                ..
            } => {
                for (_, tile, h) in bound() {
                    if h.pane_id == previous_pane_id.as_str()
                        || h.terminal_id == pane.terminal_id.as_str()
                    {
                        reaction.internals.push(Internal::BindTerminal {
                            tile,
                            herdr: to_ref(&pane.terminal_ref()),
                        });
                    }
                }
            }
            _ => {}
        },
        _ => {}
    }
    reaction
}

/// Terminal tiles bound to a live pane: path, tile and pane.
fn bound_tiles(
    session: &Session,
) -> impl Iterator<Item = (azulejo_model::TilePath, TileId, &HerdrRef)> {
    session
        .tiles()
        .filter_map(|(path, tile)| match &tile.content {
            TileContent::Terminal {
                herdr: Some(h),
                ended: false,
                ..
            } => Some((path, tile.id, h)),
            _ => None,
        })
}

/// The panes whose agent status the core follows: every bound pane, sorted.
pub fn agent_panes(session: &Session) -> Vec<String> {
    let mut panes: Vec<String> = bound_tiles(session)
        .map(|(_, _, h)| h.pane_id.clone())
        .collect();
    panes.sort_unstable();
    panes.dedup();
    panes
}

/// A herdr workspace closed: drops the tiles still bound there and the
/// binding. An Azulejo workspace left without tabs closes too, unless it's its
/// window's last; herdr deletes a workspace with its last tab, so mirroring
/// that keeps the two in step.
fn close_herdr_workspace(session: &mut Session, herdr: &str) {
    let bound: Vec<WorkspaceId> = session
        .windows
        .iter()
        .flat_map(|w| &w.workspaces)
        .filter(|w| w.herdr_workspace.as_deref() == Some(herdr))
        .map(|w| w.id)
        .collect();
    for id in bound {
        let tiles: Vec<TileId> = bound_tiles(session)
            .filter(|(path, _, _)| session.ids_at(*path).1 == id)
            .map(|(_, tile, _)| tile)
            .collect();
        for tile in tiles {
            let _ = session.close_tile(tile);
        }
        let mut emptied = false;
        let mut siblings = 0;
        for window in &mut session.windows {
            if let Some(ws) = window.workspaces.iter_mut().find(|w| w.id == id) {
                ws.herdr_workspace = None;
                emptied = ws.tabs.is_empty();
                siblings = window.workspaces.len() - 1;
            }
        }
        if emptied && siblings > 0 {
            let _ = session.close_workspace(id);
        }
    }
}

fn to_ref(t: &herdr::TerminalRef) -> HerdrRef {
    HerdrRef {
        pane_id: t.pane_id.0.clone(),
        terminal_id: t.terminal_id.0.clone(),
        tab_id: t.tab_id.0.clone(),
    }
}

/// Runs effects one at a time, so a terminal never races its workspace's
/// creation. `out` delivers results to the engine's thread; `log` reports
/// herdr failures.
pub(crate) async fn run_worker<P: HerdrPort>(
    port: Arc<P>,
    mut effects: mpsc::UnboundedReceiver<Effect>,
    api_socket: String,
    out: impl Fn(Internal),
    log: impl Fn(String),
) {
    // App workspace → herdr workspace, ahead of the engine's own copy.
    let mut workspaces: HashMap<WorkspaceId, String> = HashMap::new();
    while let Some(effect) = effects.recv().await {
        match effect {
            Effect::CreateTerminal {
                tile,
                workspace,
                herdr_workspace,
                workspace_name,
                cwd,
                label,
                env: extra_env,
                command,
            } => {
                let mut env = extra_env;
                env.extend(
                    herdr::TileEnv {
                        socket: api_socket.clone(),
                        tile_id: tile.to_string(),
                        workspace_id: workspace.to_string(),
                    }
                    .to_pairs(),
                );
                let known = workspaces.get(&workspace).cloned().or(herdr_workspace);
                let ensured = match port
                    .ensure_workspace(
                        known.clone(),
                        herdr::WorkspaceSpec {
                            label: Some(workspace_name),
                            cwd: cwd.clone(),
                            env: env.clone(),
                        },
                    )
                    .await
                {
                    Ok(ensured) => ensured,
                    Err(e) => {
                        log(format!("herdr: no workspace for tile {tile}: {e}"));
                        out(Internal::MarkEnded { tile });
                        continue;
                    }
                };
                let herdr_ws = ensured.workspace.workspace_id.0.clone();
                workspaces.insert(workspace, herdr_ws.clone());
                if known.as_deref() != Some(herdr_ws.as_str()) {
                    out(Internal::BindWorkspace {
                        workspace,
                        herdr: herdr_ws.clone(),
                    });
                }
                let terminal = match ensured.created_terminal {
                    // A new workspace's first pane already carries this tile's env.
                    Some(terminal) => Ok(terminal),
                    None => {
                        port.create_terminal(herdr::TerminalSpec {
                            workspace_id: herdr::WorkspaceId(herdr_ws),
                            label,
                            cwd,
                            env,
                        })
                        .await
                    }
                };
                match terminal {
                    Ok(t) => {
                        // herdr can't replace the shell, so the command is
                        // typed in; the PTY holds it until the shell reads.
                        if let Some(line) = command {
                            let pane = t.pane_id.0.clone();
                            let typed = match port.send_text(pane.clone(), line).await {
                                Ok(()) => port.send_keys(pane, vec!["enter".into()]).await,
                                Err(e) => Err(e),
                            };
                            if let Err(e) = typed {
                                log(format!("herdr: running tile {tile}'s command: {e}"));
                            }
                        }
                        out(Internal::BindTerminal {
                            tile,
                            herdr: to_ref(&t),
                        })
                    }
                    Err(e) => {
                        log(format!("herdr: no pane for tile {tile}: {e}"));
                        out(Internal::MarkEnded { tile });
                    }
                }
            }
            Effect::KillTerminal { pane_id } => {
                if let Err(e) = port.kill_terminal(pane_id.clone()).await {
                    log(format!("herdr: killing pane {pane_id}: {e}"));
                }
            }
            Effect::Reresolve {
                tile,
                stored,
                herdr_workspace,
            } => {
                let query = herdr::TerminalRef {
                    workspace_id: herdr::WorkspaceId(herdr_workspace.unwrap_or_default()),
                    tab_id: herdr::TabId(stored.tab_id.clone()),
                    pane_id: herdr::PaneId(stored.pane_id.clone()),
                    terminal_id: herdr::TerminalId(stored.terminal_id.clone()),
                };
                match port.reresolve(query).await {
                    Ok(herdr::Resolved::Same(_)) => {}
                    // Moved: same process, new IDs. Respawned: herdr restarted;
                    // the slot has a fresh shell (without the tile's env vars).
                    Ok(herdr::Resolved::Moved(p) | herdr::Resolved::Respawned(p)) => {
                        out(Internal::BindTerminal {
                            tile,
                            herdr: to_ref(&p.terminal_ref()),
                        });
                    }
                    Ok(herdr::Resolved::Gone) => out(Internal::MarkEnded { tile }),
                    Err(e) => log(format!("herdr: checking tile {tile}: {e}")),
                }
            }
            Effect::Survey => match port.survey().await {
                Ok(survey) => out(Internal::Surveyed(Box::new(survey))),
                Err(e) => {
                    log(format!("herdr: reading its workspaces: {e}"));
                    out(Internal::SurveyFailed);
                }
            },
        }
    }
}
