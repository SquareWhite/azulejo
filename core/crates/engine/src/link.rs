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
use azulejo_model::{HerdrRef, Session, Tile, TileContent, TileId, WorkspaceId};
use azulejo_protocol::{Command, Event, TerminalBind, WorkspaceBind};
use tokio::sync::mpsc;

use crate::{Engine, diff};

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

    /// herdr's lifecycle events. Called once, inside the runtime.
    fn events(&self) -> mpsc::Receiver<herdr::StreamItem>;

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
        let mut stream = herdr::Client::subscribe(self, herdr::Subscription::lifecycle());
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
    },
    /// Kill a closed tile's pane (F14: only when the user asked to).
    KillTerminal { pane_id: String },
    /// Check what became of a stored pane (relaunch, reconnect).
    Reresolve {
        tile: TileId,
        stored: HerdrRef,
        herdr_workspace: Option<String>,
    },
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

fn create_effect(session: &Session, tile: &Tile) -> Option<Effect> {
    let TileContent::Terminal {
        cwd,
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
    })
}

/// What to do about something herdr reported.
#[derive(Debug, Default, PartialEq)]
pub struct Reaction {
    pub internals: Vec<Internal>,
    pub effects: Vec<Effect>,
}

/// Matches herdr events to bound tiles. Panes herdr creates on its own
/// (phone, CLI, agents) aren't adopted yet: that's PLAN step 2.7.
pub fn reconcile(item: &herdr::StreamItem, session: &Session) -> Reaction {
    use herdr::HerdrEvent as E;
    let mut reaction = Reaction::default();
    let bound = || {
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
    };
    let end_where = |reaction: &mut Reaction, pred: &dyn Fn(&HerdrRef) -> bool| {
        for (_, tile, h) in bound() {
            if pred(h) {
                reaction.internals.push(Internal::MarkEnded { tile });
            }
        }
    };
    match item {
        herdr::StreamItem::Resync => {
            reaction.effects = restore_effects(session)
                .into_iter()
                .filter(|e| matches!(e, Effect::Reresolve { .. }))
                .collect();
        }
        herdr::StreamItem::Event(event) => match event {
            E::PaneClosed { pane_id, .. } | E::PaneExited { pane_id, .. } => {
                end_where(&mut reaction, &|h| h.pane_id == pane_id.as_str());
            }
            // Closing a tab reports only the tab, not its panes.
            E::TabClosed { tab_id, .. } => {
                end_where(&mut reaction, &|h| h.tab_id == tab_id.as_str());
            }
            E::WorkspaceClosed { workspace_id } => {
                for (path, tile, _) in bound() {
                    let (_, workspace, _) = session.ids_at(path);
                    if herdr_workspace(session, workspace).as_deref() == Some(workspace_id.as_str())
                    {
                        reaction.internals.push(Internal::MarkEnded { tile });
                    }
                }
            }
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
            } => {
                let env = herdr::TileEnv {
                    socket: api_socket.clone(),
                    tile_id: tile.to_string(),
                    workspace_id: workspace.to_string(),
                }
                .to_pairs();
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
                    Ok(t) => out(Internal::BindTerminal {
                        tile,
                        herdr: to_ref(&t),
                    }),
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
        }
    }
}
