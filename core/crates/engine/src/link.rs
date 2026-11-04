//! Keeps terminal tiles and herdr in step (ADR 0002).
//!
//! The engine stays pure: [`effects`] turns events into herdr work, a worker
//! runs that work through a [`HerdrPort`] one item at a time, and the results
//! come back as [`Internal`] commands applied on the engine's thread.
//! [`reconcile`] does the same for events herdr sends us.
//!
//! Every herdr machine (Local and herdr's saved machines, ADR 0011) has its
//! own worker and event stream; herdr IDs are only unique per server, so
//! every match between a tile and herdr compares the machine first. A
//! machine is `None` for Local, else herdr's saved-machine ID.
//!
//! herdr workspaces are created on demand, with the first terminal that needs
//! one: herdr always gives a new workspace a first pane, and that pane becomes
//! the terminal. Creating workspaces eagerly would leave a stray pane that
//! can't be killed without herdr deleting the (otherwise empty) workspace.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use azulejo_herdr as herdr;
use azulejo_model::{
    AgentStatus, HerdrRef, OnExit, Session, Tile, TileContent, TileId, WorkspaceId,
};
use azulejo_protocol::{Command, Event, TerminalBind, WorkspaceBind};
use tokio::sync::mpsc;

use crate::{Engine, adopt, agent, diff};

/// How many closed-in-herdr panes the stale-survey check remembers.
const MAX_CLOSED_REPORTED: usize = 1024;

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

    /// [`HerdrPort::reresolve`] for several terminals at once: one answer
    /// per stored ref, in order. [`herdr::Client`] reads herdr's panes once.
    fn reresolve_many(
        &self,
        stored: Vec<herdr::TerminalRef>,
    ) -> impl Future<Output = Result<Vec<herdr::Resolved>, String>> + Send {
        async move {
            let mut out = Vec::with_capacity(stored.len());
            for s in stored {
                out.push(self.reresolve(s).await?);
            }
            Ok(out)
        }
    }

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

    /// Where a pane's view sits, without moving it (`terminal.search`
    /// with `locate`).
    fn scroll_state(
        &self,
        pane_id: String,
    ) -> impl Future<Output = Result<herdr::ScrollInfo, String>> + Send;

    /// What runs in a pane's foreground (F28). herdr has no event for it, so
    /// the core polls. A port without it answers [`crate::meta::UNSUPPORTED`].
    fn process_info(
        &self,
        pane_id: String,
    ) -> impl Future<Output = Result<herdr::ProcessInfo, String>> + Send {
        let _ = pane_id;
        async { Err(crate::meta::UNSUPPORTED.to_owned()) }
    }

    /// The herdr session socket this port talks to, if it has one.
    fn socket(&self) -> Option<std::path::PathBuf> {
        None
    }

    // ----- layout mirroring (ADR 0012) -----

    /// A terminal split off a pane of an existing tab. A port without it
    /// fails, and the caller makes a tab instead.
    fn split_terminal(
        &self,
        spec: herdr::SplitSpec,
    ) -> impl Future<Output = Result<herdr::TerminalRef, String>> + Send {
        let _ = spec;
        async { Err(crate::meta::UNSUPPORTED.to_owned()) }
    }

    /// A herdr tab's split tree.
    fn layout_export(
        &self,
        tab: String,
    ) -> impl Future<Output = Result<herdr::TabLayout, String>> + Send {
        let _ = tab;
        async { Err(crate::meta::UNSUPPORTED.to_owned()) }
    }

    /// Makes a herdr tab look like `target`, never closing a pane.
    fn mirror(
        &self,
        target: herdr::MirrorTarget,
    ) -> impl Future<Output = Result<herdr::MirrorOutcome, herdr::MirrorError>> + Send {
        let _ = target;
        async {
            Err(herdr::MirrorError::Herdr(herdr::Error::Protocol(
                crate::meta::UNSUPPORTED.to_owned(),
            )))
        }
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

    async fn reresolve_many(
        &self,
        stored: Vec<herdr::TerminalRef>,
    ) -> Result<Vec<herdr::Resolved>, String> {
        herdr::Client::reresolve_many(self, &stored)
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

    async fn scroll_state(&self, pane_id: String) -> Result<herdr::ScrollInfo, String> {
        match herdr::Client::pane(self, &herdr::PaneId(pane_id.clone())).await {
            Ok(Some(pane)) => Ok(pane.scroll.unwrap_or_default()),
            Ok(None) => Err(format!("no pane {pane_id}")),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn process_info(&self, pane_id: String) -> Result<herdr::ProcessInfo, String> {
        herdr::Client::process_info(self, &herdr::PaneId(pane_id))
            .await
            .map_err(|e| e.to_string())
    }

    fn socket(&self) -> Option<std::path::PathBuf> {
        self.socket_path().map(std::path::Path::to_path_buf)
    }

    async fn split_terminal(&self, spec: herdr::SplitSpec) -> Result<herdr::TerminalRef, String> {
        herdr::Client::split_terminal(self, &spec)
            .await
            .map_err(|e| e.to_string())
    }

    async fn layout_export(&self, tab: String) -> Result<herdr::TabLayout, String> {
        herdr::Client::layout_export(self, &herdr::TabId(tab))
            .await
            .map_err(|e| e.to_string())
    }

    async fn mirror(
        &self,
        target: herdr::MirrorTarget,
    ) -> Result<herdr::MirrorOutcome, herdr::MirrorError> {
        herdr::Client::mirror(self, &target).await
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

/// A herdr machine: `None` for Local, else herdr's saved-machine ID.
pub type Machine = Option<String>;

/// herdr work the engine asks for, on one machine's worker.
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
        /// Where to split the pane off while mirroring the layout (ADR
        /// 0012): the pane beside the tile in its tab. `None` (or mirroring
        /// off) gives the terminal a herdr tab of its own.
        split: Option<herdr::SplitSpec>,
    },
    /// Kill a closed tile's pane (F14: only when the user asked to).
    KillTerminal { pane_id: String },
    /// Check what became of a stored pane (relaunch, reconnect).
    Reresolve {
        tile: TileId,
        stored: HerdrRef,
        herdr_workspace: Option<String>,
    },
    /// A herdr tab a tile's pane was in closed. herdr reports that before
    /// the `pane_moved` of a pane moved out of it (ADR 0012), so check: a
    /// pane that's gone closes its tile, a moved one rebinds it.
    ConfirmClosed { tile: TileId, stored: HerdrRef },
    /// Read what herdr runs, to adopt tabs nobody here knows (ADR 0008).
    Survey,
    /// Read every pane's agent status, and nothing else: herdr's agent
    /// subscription just went live, and statuses that changed before herdr
    /// accepted it never streamed.
    AgentStatuses,
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
        machine: Machine,
        herdr: String,
    },
    /// herdr reported the agent in a pane (F16).
    AgentStatus {
        machine: Machine,
        pane_id: String,
        agent: Option<String>,
        status: AgentStatus,
        at_ms: u64,
    },
    /// What a machine's herdr runs; adopt what's new. The `u64` is when the
    /// survey was requested (ms since the epoch): streamed agent statuses
    /// that arrived after that are newer than whatever the survey says.
    Surveyed(Machine, Box<herdr::Survey>, u64),
    /// A survey read only for its agent statuses ([`Effect::AgentStatuses`]):
    /// adopts nothing, closes nothing. The `u64` is as for `Surveyed`.
    AgentStatuses(Machine, Box<herdr::Survey>, u64),
    /// herdr's title and working directory for a pane (F28).
    PaneInfo {
        machine: Machine,
        pane_id: String,
        title: Option<String>,
        cwd: Option<String>,
    },
    /// What runs in a pane's foreground (F28): `command` is `None` when the
    /// shell is; `shell` is `None` when it isn't known.
    PaneProcess {
        machine: Machine,
        pane_id: String,
        shell: Option<String>,
        command: Option<String>,
        /// The foreground process's working directory, when herdr knows it.
        cwd: Option<String>,
    },
    /// What the core read about a workspace's repository (F26).
    WorkspaceGit {
        workspace: WorkspaceId,
        git: Option<azulejo_model::GitInfo>,
    },
    /// The survey failed (logged); nothing to adopt this time.
    SurveyFailed(Machine),
    /// `machine`'s event stream (re)connected and its bound tiles have been
    /// checked ([`check_bound`]): shells may attach now, to current panes.
    /// The `u64` is the link's connection generation when the check started;
    /// a newer one means the news is stale.
    Announce(Machine, u64),
    /// Someone changed a herdr tab's layout (the TUI, a phone): the tab
    /// showing it follows (ADR 0012).
    MirrorPulled {
        machine: Machine,
        layout: Box<herdr::TabLayout>,
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
                // The tile (or its whole workspace) closed while herdr made
                // its pane: that pane is closed here too, so adoption leaves
                // it alone (F14, ADR 0008).
                if self.session.tile(tile).is_err() {
                    if !self.session.detached_herdr.contains(&herdr) {
                        self.session.detached_herdr.push(herdr);
                    }
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
                self.changes_since(&before)
            }
            Internal::ClosedElsewhere { tile } => {
                let before = self.session.clone();
                if self.session.close_tile(tile).is_err() {
                    return Vec::new();
                }
                let events = self.changes_since(&before);
                self.remember_closed_in_herdr(&events);
                events
            }
            Internal::HerdrWorkspaceClosed { machine, herdr } => {
                let before = self.session.clone();
                close_herdr_workspace(&mut self.session, &machine, &herdr);
                debug_assert_eq!(self.session.check_invariants(), Ok(()));
                let events = self.changes_since(&before);
                self.remember_closed_in_herdr(&events);
                events
            }
            Internal::AgentStatus {
                machine,
                pane_id,
                agent,
                status,
                at_ms,
            } => {
                let before = self.session.clone();
                let tiles: Vec<TileId> = bound_tiles(&self.session, &machine)
                    .filter(|(_, _, h)| h.pane_id == pane_id)
                    .map(|(_, id, _)| id)
                    .collect();
                self.agent_reported
                    .insert((machine.clone(), pane_id.clone()), at_ms);
                let mut changed = false;
                for id in tiles {
                    if let Ok(tile) = self.session.tile_mut(id) {
                        changed |= agent::update(tile, agent.clone(), status, at_ms);
                    }
                }
                if !changed {
                    return Vec::new();
                }
                self.changes_since(&before)
            }
            Internal::Surveyed(machine, survey, requested) => {
                let before = self.session.clone();
                // Only Local's survey may drop the first-launch placeholder
                // (ADR 0008).
                let placeholder = if machine.is_none() {
                    self.placeholder.take()
                } else {
                    None
                };
                // A survey requested after a close reflects it; one requested
                // before still lists the pane and mustn't bring it back.
                self.closed_reported
                    .retain(|(r, at)| r.machine != machine || *at >= requested);
                let closed: Vec<HerdrRef> = self
                    .closed_reported
                    .iter()
                    .filter(|(r, _)| r.machine == machine)
                    .map(|(r, _)| r.clone())
                    .collect();
                let mut changed =
                    adopt::adopt(&mut self.session, &machine, &survey, placeholder, &closed);
                changed |= self.refresh_agents(&machine, &survey, requested);
                // Likewise for titles and working directories.
                let info = &mut self.info_reported;
                info.retain(|(m, pane), _| {
                    *m != machine || survey.panes.iter().any(|p| p.pane_id.as_str() == pane)
                });
                let fresher_info = |pane: &str| {
                    info.get(&(machine.clone(), pane.to_string()))
                        .is_some_and(|&at| at >= requested)
                };
                changed |= crate::meta::refresh_from_survey(
                    &mut self.session,
                    &machine,
                    &survey,
                    fresher_info,
                );
                if !changed {
                    return Vec::new();
                }
                debug_assert_eq!(self.session.check_invariants(), Ok(()));
                self.changes_since(&before)
            }
            Internal::AgentStatuses(machine, survey, requested) => {
                let before = self.session.clone();
                if !self.refresh_agents(&machine, &survey, requested) {
                    return Vec::new();
                }
                self.changes_since(&before)
            }
            Internal::SurveyFailed(machine) => {
                if machine.is_none() {
                    self.placeholder = None;
                }
                Vec::new()
            }
            // The driver handles it; nothing in the session changes.
            Internal::Announce(..) => Vec::new(),
            Internal::MirrorPulled { machine, layout } => {
                let before = self.session.clone();
                if !crate::mirror::absorb(&mut self.session, &machine, &layout) {
                    return Vec::new();
                }
                debug_assert_eq!(self.session.check_invariants(), Ok(()));
                self.changes_since(&before)
            }
            Internal::PaneInfo {
                machine,
                pane_id,
                title,
                cwd,
            } => {
                self.info_reported
                    .insert((machine.clone(), pane_id.clone()), agent::now_ms());
                let before = self.session.clone();
                if !crate::meta::apply_info(&mut self.session, &machine, &pane_id, title, cwd) {
                    return Vec::new();
                }
                self.changes_since(&before)
            }
            Internal::PaneProcess {
                machine,
                pane_id,
                shell,
                command,
                cwd,
            } => {
                // The survey only touches title and cwd; a polled cwd is
                // newer than a survey requested before it.
                if cwd.is_some() {
                    self.info_reported
                        .insert((machine.clone(), pane_id.clone()), agent::now_ms());
                }
                let before = self.session.clone();
                if !crate::meta::apply_process(
                    &mut self.session,
                    &machine,
                    &pane_id,
                    shell,
                    command,
                    cwd,
                ) {
                    return Vec::new();
                }
                self.changes_since(&before)
            }
            Internal::WorkspaceGit { workspace, git } => {
                let unchanged = self
                    .session
                    .workspace(workspace)
                    .map(|w| w.git == git)
                    .unwrap_or(true);
                if unchanged {
                    return Vec::new();
                }
                let before = self.session.clone();
                let _ = self.session.set_workspace_git(workspace, git);
                self.changes_since(&before)
            }
        }
    }

    /// Events for what changed since `before`, with headers brought up to
    /// date first (F28).
    /// Notes the herdr panes of tiles that closed because herdr closed them,
    /// for [`Internal::Surveyed`]'s stale-survey check.
    fn remember_closed_in_herdr(&mut self, events: &[Event]) {
        let now = agent::now_ms();
        for event in events {
            if let Event::TileClosed { tile, .. } = event
                && let TileContent::Terminal { herdr: Some(h), .. } = &tile.content
            {
                self.closed_reported.retain(|(r, _)| r != h);
                self.closed_reported.push((h.clone(), now));
            }
        }
        let len = self.closed_reported.len();
        if len > MAX_CLOSED_REPORTED {
            self.closed_reported.drain(..len - MAX_CLOSED_REPORTED);
        }
    }

    pub(crate) fn changes_since(&mut self, before: &Session) -> Vec<Event> {
        crate::meta::refresh_headers(&mut self.session);
        diff(before, &self.session, false)
    }
}

/// herdr work implied by `events`, given the session after them, with the
/// machine each piece is for.
pub fn effects(events: &[Event], session: &Session) -> Vec<(Machine, Effect)> {
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
                    out.push((
                        h.machine.clone(),
                        Effect::KillTerminal {
                            pane_id: h.pane_id.clone(),
                        },
                    ));
                }
            }
            _ => {}
        }
    }
    out
}

/// Work to bring `machine`'s terminal tiles in line with herdr: when it
/// connects, and after its event stream lost events.
pub fn restore_effects(session: &Session, machine: &Machine) -> Vec<Effect> {
    session
        .tiles()
        .filter_map(|(path, tile)| match &tile.content {
            TileContent::Terminal {
                herdr: Some(stored),
                ended: false,
                ..
            } if stored.machine == *machine => {
                let (_, workspace, _) = session.ids_at(path);
                Some(Effect::Reresolve {
                    tile: tile.id,
                    stored: stored.clone(),
                    // A workspace binding only means something on the
                    // workspace's own machine.
                    herdr_workspace: session
                        .workspace(workspace)
                        .ok()
                        .filter(|w| w.machine == *machine)
                        .and_then(|w| w.herdr_workspace.clone()),
                })
            }
            TileContent::Terminal { herdr: Some(_), .. } => None,
            _ => create_effect(session, tile)
                .filter(|(m, _)| m == machine)
                .map(|(_, e)| e),
        })
        .collect()
}

/// Work to give a respawned tile (`terminal.respawn`) a fresh pane.
pub fn respawn_effect(session: &Session, tile: TileId) -> Option<(Machine, Effect)> {
    create_effect(session, session.tile(tile).ok()?)
}

/// A new pane for an unbound terminal tile, on its workspace's machine.
fn create_effect(session: &Session, tile: &Tile) -> Option<(Machine, Effect)> {
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
    Some((
        workspace.machine.clone(),
        Effect::CreateTerminal {
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
            split: crate::mirror::split_for(session, tile.id),
        },
    ))
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
///
/// Everything here is about `machine`'s herdr only.
pub fn reconcile(
    machine: &Machine,
    item: &herdr::StreamItem,
    session: &Session,
    default_on_exit: OnExit,
) -> Reaction {
    use herdr::HerdrEvent as E;
    let mut reaction = Reaction::default();
    let bound = || bound_tiles(session, machine);
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
            reaction.effects = restore_effects(session, machine)
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
                reaction.internals.push(pane_info(machine, pane));
            }
            // Title or cwd changed (F28); agent status comes per pane.
            E::PaneUpdated(pane) => {
                if bound().any(|(_, _, h)| h.pane_id == pane.pane_id.as_str()) {
                    reaction.internals.push(pane_info(machine, pane));
                }
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
            // Closing a tab reports only the tab, not its panes. A tab also
            // closes when its last pane moves to another, and herdr says
            // so before `pane_moved`: check each pane before closing.
            E::TabClosed { tab_id, .. } => {
                for (_, tile, h) in bound() {
                    if h.tab_id == tab_id.as_str() {
                        reaction.effects.push(Effect::ConfirmClosed {
                            tile,
                            stored: h.clone(),
                        });
                    }
                }
            }
            E::WorkspaceClosed { workspace_id } => {
                reaction.internals.push(Internal::HerdrWorkspaceClosed {
                    machine: machine.clone(),
                    herdr: workspace_id.0.clone(),
                });
            }
            E::AgentStatusChanged {
                pane_id,
                agent,
                status,
                ..
            } => reaction.internals.push(Internal::AgentStatus {
                machine: machine.clone(),
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
                            herdr: to_ref(machine, &pane.terminal_ref()),
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

/// A pane's title and working directory, for its tiles' headers (F28).
fn pane_info(machine: &Machine, pane: &herdr::PaneInfo) -> Internal {
    Internal::PaneInfo {
        machine: machine.clone(),
        pane_id: pane.pane_id.0.clone(),
        title: pane.terminal_title.clone(),
        cwd: pane.foreground_cwd.clone().or_else(|| pane.cwd.clone()),
    }
}

/// Terminal tiles bound to a live pane on `machine`: path, tile and pane.
fn bound_tiles<'a>(
    session: &'a Session,
    machine: &'a Machine,
) -> impl Iterator<Item = (azulejo_model::TilePath, TileId, &'a HerdrRef)> {
    session
        .tiles()
        .filter_map(move |(path, tile)| match &tile.content {
            TileContent::Terminal {
                herdr: Some(h),
                ended: false,
                ..
            } if h.machine == *machine => Some((path, tile.id, h)),
            _ => None,
        })
}

/// The panes on `machine` whose agent status the core follows: every bound
/// pane, sorted.
pub fn agent_panes(session: &Session, machine: &Machine) -> Vec<String> {
    let mut panes: Vec<String> = bound_tiles(session, machine)
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
fn close_herdr_workspace(session: &mut Session, machine: &Machine, herdr: &str) {
    let bound: Vec<WorkspaceId> = session
        .windows
        .iter()
        .flat_map(|w| &w.workspaces)
        .filter(|w| w.machine == *machine && w.herdr_workspace.as_deref() == Some(herdr))
        .map(|w| w.id)
        .collect();
    for id in bound {
        let tiles: Vec<TileId> = bound_tiles(session, machine)
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

fn to_ref(machine: &Machine, t: &herdr::TerminalRef) -> HerdrRef {
    HerdrRef {
        pane_id: t.pane_id.0.clone(),
        terminal_id: t.terminal_id.0.clone(),
        tab_id: t.tab_id.0.clone(),
        machine: machine.clone(),
    }
}

impl Engine {
    /// Brings bound tiles' agent statuses in line with a survey requested at
    /// `requested` (ms since the epoch). A status streamed in after that is
    /// newer than the survey's, so those panes keep it.
    fn refresh_agents(
        &mut self,
        machine: &Machine,
        survey: &herdr::Survey,
        requested: u64,
    ) -> bool {
        let reported = &mut self.agent_reported;
        reported.retain(|(m, pane), _| {
            m != machine || survey.panes.iter().any(|p| p.pane_id.as_str() == pane)
        });
        let fresher = |pane: &str| {
            reported
                .get(&(machine.clone(), pane.to_string()))
                .is_some_and(|&at| at >= requested)
        };
        agent::refresh(&mut self.session, machine, survey, agent::now_ms(), fresher)
    }
}

/// The herdr query for a stored pane.
fn reresolve_query(stored: &HerdrRef, herdr_workspace: Option<String>) -> herdr::TerminalRef {
    herdr::TerminalRef {
        workspace_id: herdr::WorkspaceId(herdr_workspace.unwrap_or_default()),
        tab_id: herdr::TabId(stored.tab_id.clone()),
        pane_id: herdr::PaneId(stored.pane_id.clone()),
        terminal_id: herdr::TerminalId(stored.terminal_id.clone()),
    }
}

/// What to tell the engine about a re-resolved tile, if anything changed.
fn resolved_internal(
    machine: &Machine,
    tile: TileId,
    resolved: herdr::Resolved,
) -> Option<Internal> {
    match resolved {
        herdr::Resolved::Same(_) => None,
        // Moved: same process, new IDs. Respawned: herdr restarted; the slot
        // has a fresh shell (without the tile's env vars).
        herdr::Resolved::Moved(p) | herdr::Resolved::Respawned(p) => Some(Internal::BindTerminal {
            tile,
            herdr: to_ref(machine, &p.terminal_ref()),
        }),
        herdr::Resolved::Gone => Some(Internal::MarkEnded { tile }),
    }
}

/// When [`check_bound`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Check {
    /// The machine was just added: herdr may not be reachable yet. herdr's
    /// panes are read even with no tile to check, and only an answer
    /// announces the machine.
    Added,
    /// Its event stream just (re)connected, which already proves herdr is
    /// there.
    Connected,
}

/// Re-checks `machine`'s bound tiles (the [`Effect::Reresolve`]s in
/// `checks`) with one read of herdr's panes, then sends
/// [`Internal::Announce`]: when the machine is added, and whenever its event
/// stream (re)connects.
///
/// Runs beside the worker, not in its queue: it only reads, so it can't race
/// pane creation, and a slow survey queued ahead (a herdr that just started
/// can take 100 ms over one) doesn't hold it up. Shells hear that the machine
/// is connected only after this, so they attach to the panes herdr has now,
/// not to terminals a herdr restart replaced.
pub(crate) async fn check_bound<P: HerdrPort>(
    machine: Machine,
    port: Arc<P>,
    checks: Vec<Effect>,
    (when, generation): (Check, u64),
    out: impl Fn(Internal),
    log: impl Fn(String),
) {
    let (tiles, queries): (Vec<TileId>, Vec<herdr::TerminalRef>) = checks
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Reresolve {
                tile,
                stored,
                herdr_workspace,
            } => Some((tile, reresolve_query(&stored, herdr_workspace))),
            _ => None,
        })
        .unzip();
    if !queries.is_empty() || when == Check::Added {
        match port.reresolve_many(queries).await {
            Ok(resolved) => {
                for (tile, resolved) in tiles.into_iter().zip(resolved) {
                    if let Some(internal) = resolved_internal(&machine, tile, resolved) {
                        out(internal);
                    }
                }
            }
            // Not there yet: the event stream says when it is.
            Err(_) if when == Check::Added => return,
            Err(e) => log(format!("herdr: checking terminal tiles: {e}")),
        }
    }
    out(Internal::Announce(machine, generation));
}

/// Runs effects one at a time, so a terminal never races its workspace's
/// creation. `out` delivers results to the engine's thread; `log` reports
/// herdr failures.
pub(crate) async fn run_worker<P: HerdrPort>(
    machine: Machine,
    port: Arc<P>,
    mut effects: mpsc::UnboundedReceiver<Effect>,
    (api_socket, mirror): (String, Arc<AtomicBool>),
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
                split,
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
                        // Mirroring (ADR 0012): split off the pane beside it,
                        // so it's born in place. If that fails (the pane
                        // just went), a tab of its own; the next push moves
                        // it in.
                        let split = split.filter(|_| mirror.load(Ordering::Relaxed));
                        let made = match split {
                            Some(spec) => port
                                .split_terminal(herdr::SplitSpec {
                                    cwd: cwd.clone(),
                                    env: env.clone(),
                                    ..spec
                                })
                                .await
                                .ok(),
                            None => None,
                        };
                        match made {
                            Some(t) => Ok(t),
                            None => {
                                port.create_terminal(herdr::TerminalSpec {
                                    workspace_id: herdr::WorkspaceId(herdr_ws),
                                    label,
                                    cwd,
                                    env,
                                })
                                .await
                            }
                        }
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
                            herdr: to_ref(&machine, &t),
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
                let query = reresolve_query(&stored, herdr_workspace);
                match port.reresolve(query).await {
                    Ok(resolved) => {
                        if let Some(internal) = resolved_internal(&machine, tile, resolved) {
                            out(internal);
                        }
                    }
                    Err(e) => log(format!("herdr: checking tile {tile}: {e}")),
                }
            }
            Effect::ConfirmClosed { tile, stored } => {
                match port.reresolve(reresolve_query(&stored, None)).await {
                    Ok(herdr::Resolved::Gone) => out(Internal::ClosedElsewhere { tile }),
                    Ok(resolved) => {
                        if let Some(internal) = resolved_internal(&machine, tile, resolved) {
                            out(internal);
                        }
                    }
                    Err(e) => log(format!("herdr: checking tile {tile}: {e}")),
                }
            }
            Effect::Survey => {
                let requested = agent::now_ms();
                match port.survey().await {
                    Ok(survey) => out(Internal::Surveyed(
                        machine.clone(),
                        Box::new(survey),
                        requested,
                    )),
                    Err(e) => {
                        log(format!("herdr: reading its workspaces: {e}"));
                        out(Internal::SurveyFailed(machine.clone()));
                    }
                }
            }
            Effect::AgentStatuses => {
                let requested = agent::now_ms();
                match port.survey().await {
                    Ok(survey) => out(Internal::AgentStatuses(
                        machine.clone(),
                        Box::new(survey),
                        requested,
                    )),
                    Err(e) => log(format!("herdr: reading agent statuses: {e}")),
                }
            }
        }
    }
}
