//! The running core with a fake herdr.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use azulejo_herdr as herdr;
use azulejo_model::{HerdrRef, OnExit, Session, TileContent, TileId};
use azulejo_protocol::{Command, Direction, Event, LayoutSplit, TileClose};
use serde_json::json;
use tokio::sync::mpsc as tmpsc;

use super::*;

#[derive(Default)]
struct FakeState {
    next: u32,
    workspaces: Vec<String>,
    panes: Vec<herdr::TerminalRef>,
    created_workspaces: u32,
    created_terminals: Vec<herdr::TerminalSpec>,
    workspace_specs: Vec<herdr::WorkspaceSpec>,
    killed: Vec<String>,
    /// Overrides for `reresolve`, by terminal ID.
    resolve: HashMap<String, herdr::Resolved>,
    events: Option<tmpsc::Sender<herdr::StreamItem>>,
    /// Text typed into each pane, by pane ID; `read` returns it.
    typed: HashMap<String, String>,
    keys: Vec<(String, Vec<String>)>,
    reads: Vec<(String, herdr::ReadSource, Option<u32>)>,
    scrolled: Vec<(String, u64)>,
    /// Makes every terminal I/O call fail, as when herdr is down.
    down: bool,
    /// What the user already runs in herdr before Azulejo connects.
    existing: herdr::Survey,
    /// Every herdr call, by name, in order.
    calls: Vec<&'static str>,
    /// Pane sets the core subscribed to for agent status, in order, and the
    /// latest subscription's sender.
    agent_subs: Vec<Vec<String>>,
    agent_tx: Option<tmpsc::Sender<herdr::StreamItem>>,
    /// Agent subscriptions stay unconfirmed until the test sends
    /// `Connected` on `agent_tx`, as when herdr is slow to accept them.
    agent_unconfirmed: bool,
    /// Agent statuses herdr holds (and surveys report), by pane ID.
    agent_statuses: HashMap<String, herdr::AgentStatus>,
    /// What runs in each pane, by pane ID. `None`: process info isn't
    /// supported (the poller stops).
    processes: Option<HashMap<String, herdr::ProcessInfo>>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<FakeState>>);

impl Fake {
    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.0.lock().unwrap()
    }

    fn send(&self, item: herdr::StreamItem) {
        let tx = self.state().events.clone().expect("subscribed");
        tx.blocking_send(item).unwrap();
    }

    /// Sends on the latest agent-status subscription.
    fn send_agent(&self, pane: &str, status: herdr::AgentStatus) {
        let tx = self.state().agent_tx.clone().expect("agent subscription");
        tx.blocking_send(herdr::StreamItem::Event(
            herdr::HerdrEvent::AgentStatusChanged {
                pane_id: herdr::PaneId(pane.into()),
                workspace_id: herdr::WorkspaceId("w1".into()),
                agent: Some("claude".into()),
                status,
            },
        ))
        .unwrap();
    }

    /// A herdr that already runs `existing`.
    fn with_existing(existing: herdr::Survey) -> Fake {
        let fake = Fake::default();
        {
            let mut s = fake.state();
            for p in &existing.panes {
                s.panes.push(p.terminal_ref());
            }
            for w in &existing.workspaces {
                s.workspaces.push(w.workspace_id.0.clone());
            }
            // Keep IDs Azulejo makes clear of the existing ones.
            s.next = 100;
            s.created_workspaces = 100;
            s.existing = existing;
        }
        fake
    }

    /// herdr calls that change something.
    fn mutations(&self) -> Vec<&'static str> {
        let s = self.state();
        let mut out: Vec<&'static str> = s
            .calls
            .iter()
            .copied()
            .filter(|c| !matches!(*c, "survey" | "reresolve" | "ensure_workspace(existing)"))
            .collect();
        out.dedup();
        out
    }
}

fn pane_info(t: &herdr::TerminalRef) -> herdr::PaneInfo {
    serde_json::from_value(json!({
        "pane_id": t.pane_id,
        "terminal_id": t.terminal_id,
        "workspace_id": t.workspace_id,
        "tab_id": t.tab_id,
    }))
    .unwrap()
}

fn new_pane(s: &mut FakeState, workspace: &str) -> herdr::TerminalRef {
    s.next += 1;
    let n = s.next;
    let t = herdr::TerminalRef {
        workspace_id: herdr::WorkspaceId(workspace.into()),
        tab_id: herdr::TabId(format!("{workspace}:t{n}")),
        pane_id: herdr::PaneId(format!("{workspace}:p{n}")),
        terminal_id: herdr::TerminalId(format!("term_{n}")),
    };
    s.panes.push(t.clone());
    t
}

impl HerdrPort for Fake {
    async fn ensure_workspace(
        &self,
        existing: Option<String>,
        spec: herdr::WorkspaceSpec,
    ) -> Result<herdr::EnsuredWorkspace, String> {
        let mut s = self.state();
        let info = |id: &str| -> herdr::WorkspaceInfo {
            serde_json::from_value(json!({ "workspace_id": id })).unwrap()
        };
        if let Some(id) = existing
            && s.workspaces.contains(&id)
        {
            s.calls.push("ensure_workspace(existing)");
            return Ok(herdr::EnsuredWorkspace {
                workspace: info(&id),
                created_terminal: None,
            });
        }
        s.calls.push("ensure_workspace(create)");
        s.created_workspaces += 1;
        let id = format!("w{}", s.created_workspaces);
        s.workspaces.push(id.clone());
        s.workspace_specs.push(spec);
        let terminal = new_pane(&mut s, &id);
        Ok(herdr::EnsuredWorkspace {
            workspace: info(&id),
            created_terminal: Some(terminal),
        })
    }

    async fn create_terminal(
        &self,
        spec: herdr::TerminalSpec,
    ) -> Result<herdr::TerminalRef, String> {
        let mut s = self.state();
        s.calls.push("create_terminal");
        if !s.workspaces.contains(&spec.workspace_id.0) {
            return Err(format!("no workspace {}", spec.workspace_id));
        }
        let t = new_pane(&mut s, &spec.workspace_id.0.clone());
        s.created_terminals.push(spec);
        Ok(t)
    }

    async fn kill_terminal(&self, pane_id: String) -> Result<(), String> {
        let mut s = self.state();
        s.calls.push("kill_terminal");
        s.panes.retain(|p| p.pane_id.0 != pane_id);
        s.killed.push(pane_id);
        Ok(())
    }

    async fn reresolve(&self, stored: herdr::TerminalRef) -> Result<herdr::Resolved, String> {
        let mut s = self.state();
        s.calls.push("reresolve");
        if let Some(r) = s.resolve.get(&stored.terminal_id.0) {
            return Ok(r.clone());
        }
        Ok(
            match s.panes.iter().find(|p| p.terminal_id == stored.terminal_id) {
                Some(p) => herdr::Resolved::Same(pane_info(p)),
                None => herdr::Resolved::Gone,
            },
        )
    }

    async fn survey(&self) -> Result<herdr::Survey, String> {
        let mut s = self.state();
        s.calls.push("survey");
        let mut survey = s.existing.clone();
        survey
            .panes
            .retain(|p| s.panes.iter().any(|t| t.terminal_id == p.terminal_id));
        for t in &s.panes {
            if survey.panes.iter().any(|p| p.terminal_id == t.terminal_id) {
                continue;
            }
            survey.panes.push(pane_info(t));
            if !survey.tabs.iter().any(|tab| tab.tab_id == t.tab_id) {
                survey.tabs.push(
                    serde_json::from_value(json!({
                        "tab_id": t.tab_id,
                        "workspace_id": t.workspace_id,
                    }))
                    .unwrap(),
                );
            }
            if !survey
                .workspaces
                .iter()
                .any(|w| w.workspace_id == t.workspace_id)
            {
                survey.workspaces.push(
                    serde_json::from_value(json!({ "workspace_id": t.workspace_id })).unwrap(),
                );
            }
        }
        survey
            .tabs
            .retain(|tab| survey.panes.iter().any(|p| p.tab_id == tab.tab_id));
        for p in &mut survey.panes {
            if let Some(status) = s.agent_statuses.get(p.pane_id.as_str()) {
                p.agent = Some("claude".into());
                p.agent_status = *status;
            }
        }
        Ok(survey)
    }

    fn events(&self) -> tmpsc::Receiver<herdr::StreamItem> {
        let (tx, rx) = tmpsc::channel(16);
        self.state().events = Some(tx);
        rx
    }

    fn agent_events(&self, panes: Vec<String>) -> tmpsc::Receiver<herdr::StreamItem> {
        let (tx, rx) = tmpsc::channel(16);
        let mut s = self.state();
        if !s.agent_unconfirmed {
            tx.try_send(herdr::StreamItem::Connected).unwrap();
        }
        s.agent_subs.push(panes);
        s.agent_tx = Some(tx);
        rx
    }

    async fn send_text(&self, pane_id: String, text: String) -> Result<(), String> {
        let mut s = self.state();
        if s.down {
            return Err("connection refused".into());
        }
        s.typed.entry(pane_id).or_default().push_str(&text);
        Ok(())
    }

    async fn send_keys(&self, pane_id: String, keys: Vec<String>) -> Result<(), String> {
        let mut s = self.state();
        if s.down {
            return Err("connection refused".into());
        }
        s.keys.push((pane_id, keys));
        Ok(())
    }

    async fn read(
        &self,
        pane_id: String,
        source: herdr::ReadSource,
        lines: Option<u32>,
    ) -> Result<herdr::ReadResult, String> {
        let mut s = self.state();
        if s.down {
            return Err("connection refused".into());
        }
        s.reads.push((pane_id.clone(), source, lines));
        Ok(herdr::ReadResult {
            text: s.typed.get(&pane_id).cloned().unwrap_or_default(),
            revision: 1,
            truncated: lines.is_some(),
        })
    }

    async fn scroll(
        &self,
        pane_id: String,
        offset_from_bottom: u64,
    ) -> Result<herdr::ScrollInfo, String> {
        let mut s = self.state();
        if s.down {
            return Err("connection refused".into());
        }
        s.scrolled.push((pane_id, offset_from_bottom));
        Ok(herdr::ScrollInfo {
            offset_from_bottom: offset_from_bottom.min(100),
            max_offset_from_bottom: 100,
            viewport_rows: 40,
        })
    }

    async fn process_info(&self, pane_id: String) -> Result<herdr::ProcessInfo, String> {
        let s = self.state();
        match &s.processes {
            None => Err(crate::meta::UNSUPPORTED.into()),
            Some(map) => Ok(map.get(&pane_id).cloned().unwrap_or_default()),
        }
    }
}

fn opts(state_path: Option<std::path::PathBuf>) -> CoreOptions {
    CoreOptions {
        state_path,
        api_socket: "/tmp/azj.sock".into(),
        log: Arc::new(|line| eprintln!("core: {line}")),
    }
}

fn terminals(session: &Session) -> Vec<(TileId, Option<HerdrRef>, bool)> {
    session
        .tiles()
        .filter_map(|(_, t)| match &t.content {
            TileContent::Terminal { herdr, ended, .. } => Some((t.id, herdr.clone(), *ended)),
            _ => None,
        })
        .collect()
}

fn wait_until(core: &Core, ok: impl Fn(&Session) -> bool) -> Session {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let session = core.session().unwrap();
        if ok(&session) {
            return session;
        }
        assert!(
            Instant::now() < deadline,
            "timed out: {:#?}",
            terminals(&session)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn all_bound(s: &Session) -> bool {
    terminals(s).iter().all(|(_, h, e)| h.is_some() && !e)
}

fn split(core: &Core) -> TileId {
    let outcome = core
        .execute(Command::LayoutSplit(LayoutSplit {
            tile: None,
            direction: Direction::Right,
            content: None,
        }))
        .unwrap();
    match outcome.reply {
        azulejo_protocol::Reply::Created(c) => c.tile.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn first_terminal_takes_the_new_workspaces_pane_and_later_ones_get_tabs() {
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    let session = wait_until(&core, all_bound);
    let first = terminals(&session)[0].0;
    {
        let s = fake.state();
        assert_eq!(s.created_workspaces, 1);
        assert!(s.created_terminals.is_empty());
        let env = &s.workspace_specs[0].env;
        assert!(env.contains(&("AZULEJO_TILE_ID".into(), first.to_string())));
        assert!(env.contains(&("AZULEJO_SOCKET".into(), "/tmp/azj.sock".into())));
    }
    assert_eq!(
        session.windows[0].workspaces[0].herdr_workspace.as_deref(),
        Some("w1")
    );

    let second = split(&core);
    wait_until(&core, |s| terminals(s).len() == 2 && all_bound(s));
    let s = fake.state();
    assert_eq!(s.created_workspaces, 1, "same herdr workspace");
    assert_eq!(s.created_terminals.len(), 1);
    assert_eq!(s.created_terminals[0].workspace_id.0, "w1");
    assert!(
        s.created_terminals[0]
            .env
            .contains(&("AZULEJO_TILE_ID".into(), second.to_string()))
    );
}

#[test]
fn close_detaches_unless_kill() {
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    let a = split(&core);
    let b = split(&core);
    let session = wait_until(&core, |s| terminals(s).len() == 3 && all_bound(s));
    let pane_of = |tile: TileId| {
        terminals(&session)
            .into_iter()
            .find(|(t, _, _)| *t == tile)
            .and_then(|(_, h, _)| h)
            .unwrap()
            .pane_id
    };
    core.execute(Command::TileClose(TileClose {
        tile: Some(a),
        kill: false,
    }))
    .unwrap();
    core.execute(Command::TileClose(TileClose {
        tile: Some(b),
        kill: true,
    }))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.state().killed.is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(fake.state().killed, [pane_of(b)]);
}

#[test]
fn restore_reresolves_every_bound_tile() {
    // Build a session with four bound terminals.
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    for _ in 0..3 {
        split(&core);
    }
    let session = wait_until(&core, |s| terminals(s).len() == 4 && all_bound(s));
    core.shutdown();
    let tiles = terminals(&session);
    let refs: Vec<HerdrRef> = tiles.iter().map(|(_, h, _)| h.clone().unwrap()).collect();

    // herdr's view on relaunch: same, moved, respawned, gone.
    let moved = herdr::TerminalRef {
        workspace_id: herdr::WorkspaceId("w1".into()),
        tab_id: herdr::TabId("w1:t9".into()),
        pane_id: herdr::PaneId("w1:p9".into()),
        terminal_id: herdr::TerminalId(refs[1].terminal_id.clone()),
    };
    let respawned = herdr::TerminalRef {
        terminal_id: herdr::TerminalId("term_new".into()),
        ..moved.clone()
    };
    {
        let mut s = fake.state();
        s.resolve.insert(
            refs[1].terminal_id.clone(),
            herdr::Resolved::Moved(pane_info(&moved)),
        );
        s.resolve.insert(
            refs[2].terminal_id.clone(),
            herdr::Resolved::Respawned(pane_info(&respawned)),
        );
        s.resolve
            .insert(refs[3].terminal_id.clone(), herdr::Resolved::Gone);
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    persist::save(&path, &session).unwrap();
    let core = Core::start_with_herdr(opts(Some(path)), fake.clone());
    wait_until(&core, |s| {
        let t = terminals(s);
        t[1].1.as_ref().unwrap().pane_id == "w1:p9"
            && t[2].1.as_ref().unwrap().terminal_id == "term_new"
            && t[3].2
    });
    let t = terminals(&core.session().unwrap());
    assert_eq!(t[0].1.as_ref(), Some(&refs[0]), "same: untouched");
    assert!(!t[0].2);
    assert_eq!(t[1].1.as_ref().unwrap().tab_id, "w1:t9", "moved: rebound");
    assert_eq!(
        t[2].1.as_ref().unwrap().terminal_id,
        "term_new",
        "respawned: rebound"
    );
    assert!(t[3].2, "gone: ended");
    assert_eq!(t[3].1.as_ref(), Some(&refs[3]), "gone keeps its last ref");
}

#[test]
fn herdr_events_end_close_or_rebind_tiles() {
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    // Exits leave tiles ended here; closing on exit has its own test.
    core.set_on_exit(OnExit::Keep);
    for _ in 0..4 {
        split(&core);
    }
    let session = wait_until(&core, |s| terminals(s).len() == 5 && all_bound(s));
    let tiles: Vec<TileId> = terminals(&session).into_iter().map(|t| t.0).collect();
    let refs: Vec<HerdrRef> = terminals(&session)
        .into_iter()
        .map(|(_, h, _)| h.unwrap())
        .collect();
    let events = core.subscribe();

    use herdr::HerdrEvent as E;
    // The process exited: the tile stays, ended.
    fake.send(herdr::StreamItem::Event(E::PaneExited {
        pane_id: herdr::PaneId(refs[0].pane_id.clone()),
        workspace_id: herdr::WorkspaceId("w1".into()),
    }));
    // Closed in herdr: tab and pane. The tiles go.
    fake.send(herdr::StreamItem::Event(E::TabClosed {
        tab_id: herdr::TabId(refs[1].tab_id.clone()),
        workspace_id: herdr::WorkspaceId("w1".into()),
    }));
    fake.send(herdr::StreamItem::Event(E::PaneClosed {
        pane_id: herdr::PaneId(refs[3].pane_id.clone()),
        workspace_id: herdr::WorkspaceId("w1".into()),
    }));
    let moved = herdr::TerminalRef {
        workspace_id: herdr::WorkspaceId("w2".into()),
        tab_id: herdr::TabId("w2:t1".into()),
        pane_id: herdr::PaneId("w2:p1".into()),
        terminal_id: herdr::TerminalId(refs[2].terminal_id.clone()),
    };
    fake.send(herdr::StreamItem::Event(E::PaneMoved {
        previous_pane_id: herdr::PaneId(refs[2].pane_id.clone()),
        previous_tab_id: herdr::TabId(refs[2].tab_id.clone()),
        previous_workspace_id: herdr::WorkspaceId("w1".into()),
        pane: pane_info(&moved),
    }));
    // Something unrelated changes nothing.
    fake.send(herdr::StreamItem::Event(E::PaneClosed {
        pane_id: herdr::PaneId("w7:p7".into()),
        workspace_id: herdr::WorkspaceId("w7".into()),
    }));

    let after = wait_until(&core, |s| {
        terminals(s).len() == 3
            && s.tile(tiles[2])
                .is_ok_and(|t| matches!(&t.content, TileContent::Terminal { herdr: Some(h), .. } if h.pane_id == "w2:p1"))
    });
    let t = terminals(&after);
    assert_eq!(
        t.iter().map(|t| t.0).collect::<Vec<_>>(),
        [tiles[0], tiles[2], tiles[4]]
    );
    assert!(t[0].2, "exited: ended");
    assert!(!t[2].2);
    assert_eq!(t[2].1.as_ref(), Some(&refs[4]));
    let seen: Vec<Event> = events.try_iter().collect();
    let count = |f: &dyn Fn(&Event) -> bool| seen.iter().filter(|e| f(e)).count();
    assert_eq!(count(&|e| matches!(e, Event::TileUpdated { .. })), 2);
    assert_eq!(
        count(&|e| matches!(e, Event::TileClosed { kill: false, .. })),
        2
    );
    // Closed elsewhere isn't "closed here": nothing to keep out of adoption.
    assert!(after.detached_herdr.is_empty());
    // We never killed anything.
    assert!(fake.state().killed.is_empty());
}

#[test]
fn a_closed_herdr_workspace_takes_its_tiles_and_binding() {
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    split(&core);
    wait_until(&core, |s| terminals(s).len() == 2 && all_bound(s));
    // A second workspace with a browser and a terminal, so the first can go.
    let created = core
        .execute(Command::WorkspaceCreate(
            azulejo_protocol::WorkspaceCreate {
                window: None,
                name: Some("other".into()),
                cwd: None,
                machine: None,
            },
        ))
        .unwrap();
    let Reply::Created(created) = created.reply else {
        panic!("{created:?}")
    };
    core.execute(Command::TabCreate(azulejo_protocol::TabCreate {
        workspace: created.workspace,
        ..Default::default()
    }))
    .unwrap();
    let s = wait_until(&core, |s| {
        s.windows[0].workspaces.len() == 2 && all_bound(s) && terminals(s).len() == 3
    });
    let (first, second) = (&s.windows[0].workspaces[0], &s.windows[0].workspaces[1]);
    let herdr_ws = first.herdr_workspace.clone().unwrap();
    assert_ne!(second.herdr_workspace.as_deref(), Some(herdr_ws.as_str()));

    fake.send(herdr::StreamItem::Event(
        herdr::HerdrEvent::WorkspaceClosed {
            workspace_id: herdr::WorkspaceId(herdr_ws.clone()),
        },
    ));
    let after = wait_until(&core, |s| s.windows[0].workspaces.len() == 1);
    assert_eq!(after.windows[0].workspaces[0].id, second.id);
    assert_eq!(terminals(&after).len(), 1);
}

#[test]
fn resync_reresolves_and_marks_lost_panes() {
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    split(&core);
    let session = wait_until(&core, |s| terminals(s).len() == 2 && all_bound(s));
    let gone = terminals(&session)[1].1.clone().unwrap();
    // herdr lost a pane while we weren't listening.
    fake.state()
        .panes
        .retain(|p| p.terminal_id.0 != gone.terminal_id);
    fake.send(herdr::StreamItem::Resync);
    // A pane lost without a `pane_exited` is ended, never closed, even with
    // the default `on_exit = "close"`: we can't tell what happened to it.
    let after = wait_until(&core, |s| terminals(s)[1].2);
    assert!(!terminals(&after)[0].2);
}

fn exited(fake: &Fake, pane_id: &str) {
    fake.state().panes.retain(|p| p.pane_id.0 != pane_id);
    fake.send(herdr::StreamItem::Event(herdr::HerdrEvent::PaneExited {
        pane_id: herdr::PaneId(pane_id.into()),
        workspace_id: herdr::WorkspaceId("w1".into()),
    }));
}

#[test]
fn a_shell_that_exits_closes_its_tile_by_default() {
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    split(&core);
    split(&core);
    let session = wait_until(&core, |s| terminals(s).len() == 3 && all_bound(s));
    let t = terminals(&session);
    let events = core.subscribe();

    // ctrl+d in the middle tile: it goes, like any terminal window.
    exited(&fake, &t[1].1.as_ref().unwrap().pane_id);
    let after = wait_until(&core, |s| terminals(s).len() == 2);
    assert_eq!(
        terminals(&after).iter().map(|t| t.0).collect::<Vec<_>>(),
        [t[0].0, t[2].0]
    );
    assert!(terminals(&after).iter().all(|t| !t.2), "nothing ended");
    assert!(after.check_invariants().is_ok());
    let seen: Vec<Event> = events.try_iter().collect();
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::TileClosed { tile, kill: false, .. } if tile.id == t[1].0)),
        "{seen:?}"
    );
    // The pane is gone already: nothing to kill, nothing to keep out of adoption.
    assert!(fake.state().killed.is_empty());
    assert!(after.detached_herdr.is_empty());

    // The last tile of a tab takes the tab with it.
    let tab = after.focus().tab.unwrap();
    exited(&fake, &t[0].1.as_ref().unwrap().pane_id);
    exited(&fake, &t[2].1.as_ref().unwrap().pane_id);
    let empty = wait_until(&core, |s| terminals(s).is_empty());
    assert!(empty.tab(tab).is_err(), "tab closed with its last tile");
    assert!(empty.check_invariants().is_ok());
}

#[test]
fn on_exit_keep_leaves_the_tile_ended_and_can_change_live() {
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    split(&core);
    let session = wait_until(&core, |s| terminals(s).len() == 2 && all_bound(s));
    let t = terminals(&session);

    core.set_on_exit(OnExit::Keep);
    exited(&fake, &t[0].1.as_ref().unwrap().pane_id);
    let after = wait_until(&core, |s| terminals(s)[0].2);
    assert_eq!(terminals(&after).len(), 2, "kept, ended");

    // A config reload switches back; the next exit closes.
    core.set_on_exit(OnExit::Close);
    exited(&fake, &t[1].1.as_ref().unwrap().pane_id);
    let after = wait_until(&core, |s| terminals(s).len() == 1);
    assert_eq!(terminals(&after)[0].0, t[0].0);
}

#[test]
fn layout_survives_a_hard_kill() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let core = Core::start(opts(Some(path.clone())));
    split(&core);
    split(&core);
    core.execute(Command::LayoutFocus(azulejo_protocol::LayoutFocus {
        direction: Direction::Left,
    }))
    .unwrap();
    let before = core.session().unwrap();
    // The debounced save lands; then the process "dies" without flushing.
    std::thread::sleep(persist::SAVE_DEBOUNCE + Duration::from_millis(300));
    std::mem::forget(core);

    let core = Core::start(opts(Some(path)));
    assert_eq!(core.session().unwrap(), before);
}

#[test]
fn shutdown_flushes_and_corrupt_state_starts_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let core = Core::start(opts(Some(path.clone())));
    split(&core);
    let before = core.session().unwrap();
    core.shutdown();
    let Ok(persist::Loaded::Session(saved)) = persist::load(&path) else {
        panic!("expected a saved session");
    };
    assert_eq!(saved, before);

    std::fs::write(&path, "{garbage").unwrap();
    let logged = Arc::new(Mutex::new(Vec::new()));
    let sink = logged.clone();
    let core = Core::start(CoreOptions {
        state_path: Some(path.clone()),
        api_socket: String::new(),
        log: Arc::new(move |line| sink.lock().unwrap().push(line)),
    });
    assert_eq!(core.session().unwrap().tiles().count(), 1);
    core.shutdown();
    assert!(path.with_extension("json.bak").exists());
    assert!(logged.lock().unwrap()[0].contains("unusable"));
}

// ----- terminal I/O (F20) -----

fn focused_pane(core: &Core) -> (TileId, String) {
    let session = wait_until(core, all_bound);
    let tile = session.focused_tile().unwrap();
    match &session.tile(tile).unwrap().content {
        TileContent::Terminal { herdr: Some(h), .. } => (tile, h.pane_id.clone()),
        other => panic!("not a bound terminal: {other:?}"),
    }
}

fn reply(core: &Core, command: Command) -> Result<Reply, azulejo_protocol::Error> {
    core.execute(command).map(|o| o.reply)
}

#[test]
fn terminal_io_goes_to_the_tiles_pane() {
    use azulejo_protocol::*;
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    let (tile, pane) = focused_pane(&core);

    let sent = reply(
        &core,
        Command::TerminalSendText(TerminalSendText {
            tile: None,
            text: "echo hi\nError: boom\nerror again\n".into(),
        }),
    );
    assert_eq!(sent, Ok(Reply::Ok));
    assert_eq!(
        reply(
            &core,
            Command::TerminalSendKeys(TerminalSendKeys {
                tile: Some(tile),
                keys: vec!["ctrl+c".into(), "enter".into()],
            }),
        ),
        Ok(Reply::Ok)
    );
    assert_eq!(
        fake.state().keys,
        vec![(
            pane.clone(),
            vec!["ctrl+c".to_string(), "enter".to_string()]
        )]
    );

    let text = reply(
        &core,
        Command::TerminalRead(TerminalRead {
            tile: Some(tile),
            source: ReadSource::Recent,
            lines: Some(50),
        }),
    );
    assert_eq!(
        text,
        Ok(Reply::Text {
            text: "echo hi\nError: boom\nerror again\n".into()
        })
    );
    assert_eq!(
        fake.state().reads.last(),
        Some(&(pane.clone(), herdr::ReadSource::Recent, Some(50)))
    );

    let scrolled = reply(
        &core,
        Command::TerminalScroll(TerminalScroll {
            tile: None,
            offset_from_bottom: 250,
        }),
    );
    assert_eq!(
        scrolled,
        Ok(Reply::Scroll(ScrollState {
            offset_from_bottom: 100,
            max_offset_from_bottom: 100,
            viewport_rows: 40,
        }))
    );
    assert_eq!(fake.state().scrolled, vec![(pane.clone(), 250)]);

    let found = reply(
        &core,
        Command::TerminalSearch(TerminalSearch {
            tile: None,
            query: "ERROR".into(),
            case_sensitive: false,
            lines: None,
        }),
    );
    assert_eq!(
        found,
        Ok(Reply::Matches {
            matches: vec![
                SearchMatch {
                    line: 1,
                    text: "Error: boom".into()
                },
                SearchMatch {
                    line: 2,
                    text: "error again".into()
                },
            ],
            truncated: false,
        })
    );
    assert_eq!(
        fake.state().reads.last(),
        Some(&(pane, herdr::ReadSource::RecentUnwrapped, None))
    );
    let exact = reply(
        &core,
        Command::TerminalSearch(TerminalSearch {
            tile: None,
            query: "ERROR".into(),
            case_sensitive: true,
            lines: None,
        }),
    );
    assert_eq!(
        exact,
        Ok(Reply::Matches {
            matches: vec![],
            truncated: false
        })
    );
}

#[test]
fn terminal_io_errors() {
    use azulejo_protocol::*;
    let read = |tile| {
        Command::TerminalRead(TerminalRead {
            tile,
            source: ReadSource::Visible,
            lines: None,
        })
    };

    // No herdr at all.
    let core = Core::start(opts(None));
    assert_eq!(
        reply(&core, read(None)).unwrap_err().code,
        ErrorCode::Unavailable
    );
    drop(core);

    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    // An exited pane leaves the tile ended, for the "ended" error below.
    core.set_on_exit(OnExit::Keep);
    let (tile, pane) = focused_pane(&core);

    // Not a terminal.
    let browser = match core
        .execute(Command::LayoutSplit(LayoutSplit {
            tile: None,
            direction: Direction::Down,
            content: Some(TileContent::Browser {
                url: None,
                page_title: None,
            }),
        }))
        .unwrap()
        .reply
    {
        Reply::Created(c) => c.tile.unwrap(),
        other => panic!("{other:?}"),
    };
    let err = reply(&core, read(Some(browser))).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidState);
    assert!(err.message.contains("browser tile"), "{err}");

    // Unknown tile.
    assert_eq!(
        reply(&core, read(Some(TileId::new()))).unwrap_err().code,
        ErrorCode::NotFound
    );

    // Bad params never reach herdr.
    let err = reply(
        &core,
        Command::TerminalSendKeys(TerminalSendKeys {
            tile: Some(tile),
            keys: vec![],
        }),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParams);
    assert!(fake.state().keys.is_empty());

    // herdr down.
    fake.state().down = true;
    let err = reply(&core, read(Some(tile))).unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable);
    assert!(err.message.contains("connection refused"), "{err}");
    fake.state().down = false;

    // Ended pane.
    fake.send(herdr::StreamItem::Event(herdr::HerdrEvent::PaneExited {
        pane_id: herdr::PaneId(pane),
        workspace_id: herdr::WorkspaceId("w1".into()),
    }));
    wait_until(&core, |s| {
        matches!(
            s.tile(tile).map(|t| &t.content),
            Ok(TileContent::Terminal { ended: true, .. })
        )
    });
    let err = reply(&core, read(Some(tile))).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidState);
    assert!(err.message.contains("ended"), "{err}");
}

#[test]
fn herdr_socket_comes_from_the_port() {
    let core = Core::start_with_herdr(opts(None), Fake::default());
    assert_eq!(core.herdr_socket(), None);
    let core = Core::start(opts(None));
    assert_eq!(core.herdr_socket(), None);
}

// ----- adoption (ADR 0008) -----

fn adopted(s: &Session) -> usize {
    terminals(s).iter().filter(|(_, h, _)| h.is_some()).count()
}

/// Waits until every bound tile has been re-checked once (restore ran).
fn wait_for_calls(fake: &Fake, call: &str, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.state().calls.iter().filter(|c| **c == call).count() < n {
        assert!(Instant::now() < deadline, "calls: {:?}", fake.state().calls);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn adopting_and_restoring_never_change_herdr() {
    let fake = Fake::with_existing(crate::adopt::tests::fixture());
    let core = Core::start_with_herdr(opts(None), fake.clone());
    let session = wait_until(&core, |s| adopted(s) == 6);
    wait_for_calls(&fake, "reresolve", 6);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(fake.mutations(), Vec::<&str>::new());
    assert_eq!(fake.state().calls[0], "survey", "adoption comes first");
    assert_eq!(session.windows[0].workspaces.len(), 2, "no placeholder");
    core.shutdown();

    // Relaunch from saved state: still nothing but reads.
    let dir = std::env::temp_dir().join(format!("azj-adopt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let state = dir.join("state.json");
    let fake = Fake::with_existing(crate::adopt::tests::fixture());
    let core = Core::start_with_herdr(opts(Some(state.clone())), fake.clone());
    let first = wait_until(&core, |s| adopted(s) == 6);
    wait_for_calls(&fake, "reresolve", 6);
    core.shutdown();
    let fake = Fake::with_existing(crate::adopt::tests::fixture());
    let core = Core::start_with_herdr(opts(Some(state)), fake.clone());
    wait_for_calls(&fake, "reresolve", 6);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        core.session().unwrap(),
        first,
        "same session, no duplicates"
    );
    assert_eq!(fake.mutations(), Vec::<&str>::new());
    core.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_empty_herdr_still_gets_the_first_terminal() {
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    wait_until(&core, all_bound);
    let s = fake.state();
    assert_eq!(s.calls[0], "survey");
    assert_eq!(s.created_workspaces, 1);
}

#[test]
fn new_herdr_tabs_are_adopted_and_detached_ones_stay_out() {
    let fake = Fake::with_existing(crate::adopt::tests::fixture());
    let core = Core::start_with_herdr(opts(None), fake.clone());
    wait_until(&core, |s| adopted(s) == 6);

    // The user opens a tab in their herdr TUI.
    let pane: herdr::PaneInfo = serde_json::from_value(json!({
        "pane_id": "w2:p7", "terminal_id": "term_tui", "workspace_id": "w2", "tab_id": "w2:t7"
    }))
    .unwrap();
    let tab: herdr::TabInfo =
        serde_json::from_value(json!({ "tab_id": "w2:t7", "workspace_id": "w2", "number": 7 }))
            .unwrap();
    {
        let mut s = fake.state();
        s.panes.push(pane.terminal_ref());
        s.existing.panes.push(pane);
        s.existing.tabs.push(tab.clone());
    }
    fake.send(herdr::StreamItem::Event(herdr::HerdrEvent::TabCreated(tab)));
    let session = wait_until(&core, |s| adopted(s) == 7);
    let tile = session
        .tiles()
        .find(|(_, t)| {
            matches!(&t.content, TileContent::Terminal { herdr: Some(h), .. } if h.pane_id == "w2:p7")
        })
        .map(|(_, t)| t.id)
        .unwrap();

    // Closing it here detaches; a resync doesn't bring it back.
    core.execute(Command::TileClose(TileClose {
        tile: Some(tile),
        kill: false,
    }))
    .unwrap();
    fake.send(herdr::StreamItem::Resync);
    let surveys = fake
        .state()
        .calls
        .iter()
        .filter(|c| **c == "survey")
        .count();
    wait_for_calls(&fake, "survey", surveys + 1);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(adopted(&core.session().unwrap()), 6);
    assert_eq!(fake.mutations(), Vec::<&str>::new());
}

#[test]
fn respawn_gives_an_ended_tile_a_fresh_pane_in_place() {
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    // Respawn is for tiles kept after their shell exits.
    core.set_on_exit(OnExit::Keep);
    let second = split(&core);
    let session = wait_until(&core, |s| terminals(s).len() == 2 && all_bound(s));
    let tree_before = session
        .tab(session.focus().tab.unwrap())
        .unwrap()
        .root
        .clone();
    let old = terminals(&session)[1].1.clone().unwrap();

    // A running tile can't be respawned.
    let err = reply(
        &core,
        Command::TerminalRespawn(azulejo_protocol::TileTarget { tile: Some(second) }),
    )
    .unwrap_err();
    assert_eq!(err.code, azulejo_protocol::ErrorCode::InvalidState);

    // Its pane ends outside Azulejo.
    fake.state().panes.retain(|p| p.pane_id.0 != old.pane_id);
    fake.send(herdr::StreamItem::Event(herdr::HerdrEvent::PaneExited {
        pane_id: herdr::PaneId(old.pane_id.clone()),
        workspace_id: herdr::WorkspaceId("w1".into()),
    }));
    wait_until(&core, |s| terminals(s)[1].2);
    let created_before = fake.state().created_terminals.len();

    reply(
        &core,
        Command::TerminalRespawn(azulejo_protocol::TileTarget { tile: Some(second) }),
    )
    .unwrap();
    let after = wait_until(&core, |s| {
        let t = terminals(s);
        t[1].0 == second && t[1].1.as_ref().is_some_and(|h| *h != old) && !t[1].2
    });
    // Same tile, same place; one new pane in its own tab, with the tile's env.
    assert_eq!(
        after.tab(after.focus().tab.unwrap()).unwrap().root,
        tree_before
    );
    let s = fake.state();
    assert_eq!(s.created_terminals.len(), created_before + 1);
    let spec = s.created_terminals.last().unwrap();
    assert!(
        spec.env
            .iter()
            .any(|(k, v)| k == "AZULEJO_TILE_ID" && *v == second.to_string())
    );
    assert!(s.killed.is_empty());
}

#[test]
fn respawn_needs_herdr_and_a_terminal() {
    let core = Core::start(opts(None));
    let err = reply(
        &core,
        Command::TerminalRespawn(azulejo_protocol::TileTarget { tile: None }),
    )
    .unwrap_err();
    assert_eq!(err.code, azulejo_protocol::ErrorCode::Unavailable);

    let mut engine = Engine::new();
    let browser = engine
        .execute(Command::LayoutSplit(LayoutSplit {
            tile: None,
            direction: Direction::Right,
            content: Some(TileContent::Browser {
                url: None,
                page_title: None,
            }),
        }))
        .unwrap();
    let azulejo_protocol::Reply::Created(c) = browser.reply else {
        panic!()
    };
    let err = engine
        .execute(Command::TerminalRespawn(azulejo_protocol::TileTarget {
            tile: c.tile,
        }))
        .unwrap_err();
    assert_eq!(err.code, azulejo_protocol::ErrorCode::InvalidState);
}

#[test]
fn herdr_link_changes_reach_listeners() {
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    wait_until(&core, all_bound);
    let events = core.subscribe();
    // Make sure the subscription is registered before herdr talks.
    core.session().unwrap();
    fake.send(herdr::StreamItem::Disconnected("connection refused".into()));
    fake.send(herdr::StreamItem::Resync);
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while seen.len() < 2 && Instant::now() < deadline {
        if let Ok(Event::HerdrStatus { connected, .. }) =
            events.recv_timeout(Duration::from_millis(50))
        {
            seen.push(connected);
        }
    }
    assert_eq!(seen, vec![false, true]);
}

fn agent_of(s: &Session, tile: TileId) -> Option<azulejo_model::AgentInfo> {
    s.tile(tile).ok().and_then(|t| t.agent.clone())
}

#[test]
fn agent_status_follows_bound_panes_and_lights_attention() {
    use azulejo_model::AgentStatus;
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    let first = wait_until(&core, all_bound).focused_tile().unwrap();
    let second = split(&core);
    let s = wait_until(&core, |s| terminals(s).len() == 2 && all_bound(s));
    let panes: Vec<String> = terminals(&s)
        .into_iter()
        .map(|(_, h, _)| h.unwrap().pane_id)
        .collect();
    // The subscription follows the bound panes.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut want = panes.clone();
        want.sort();
        if fake.state().agent_subs.last() == Some(&want) {
            break;
        }
        assert!(Instant::now() < deadline, "{:?}", fake.state().agent_subs);
        std::thread::sleep(Duration::from_millis(5));
    }
    let events = core.subscribe();

    // `second` is focused (split focuses the new tile); `first` isn't.
    fake.send_agent(&panes[0], herdr::AgentStatus::Blocked);
    let s = wait_until(&core, |s| agent_of(s, first).is_some_and(|a| a.attention));
    let a = agent_of(&s, first).unwrap();
    assert_eq!(
        (a.status, a.name.as_deref()),
        (AgentStatus::Blocked, Some("claude"))
    );
    let ws = &s.windows[0].workspaces[0];
    let info = match core
        .execute(Command::WorkspaceList(Default::default()))
        .unwrap()
        .reply
    {
        Reply::Workspaces { workspaces } => workspaces[0].workspace.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!((info.id, info.blocked, info.attention), (ws.id, 1, 1));

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = Vec::new();
    while !seen
        .iter()
        .any(|e| matches!(e, Event::TileAttentionChanged { .. }))
    {
        let left = deadline.saturating_duration_since(Instant::now());
        seen.push(events.recv_timeout(left).expect("attention event"));
    }
    assert!(seen.iter().any(|e| matches!(e,
        Event::AgentStateChanged(c) if c.tile == first
            && c.state == azulejo_protocol::AgentState::Blocked
            && c.agent.as_deref() == Some("claude"))));
    assert!(seen.iter().any(|e| matches!(e,
        Event::WorkspaceUpdated { workspace, .. } if workspace.blocked == 1)));

    // Looking at it clears the attention; it's still blocked.
    core.execute(Command::TileFocus(azulejo_protocol::TileTarget {
        tile: Some(first),
    }))
    .unwrap();
    let a = agent_of(&core.session().unwrap(), first).unwrap();
    assert_eq!((a.status, a.attention), (AgentStatus::Blocked, false));

    // The focused tile lights up too; `tile.seen` clears it in place.
    fake.send_agent(&panes[0], herdr::AgentStatus::Working);
    fake.send_agent(&panes[0], herdr::AgentStatus::Done);
    wait_until(&core, |s| agent_of(s, first).is_some_and(|a| a.attention));
    core.execute(Command::TileSeen(azulejo_protocol::TileTarget {
        tile: None,
    }))
    .unwrap();
    assert!(!agent_of(&core.session().unwrap(), first).unwrap().attention);
    assert_eq!(core.session().unwrap().focused_tile(), Some(first));
    let _ = second;
}

#[test]
fn panes_split_off_elsewhere_become_tiles_and_closing_there_removes_them() {
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    let s = wait_until(&core, all_bound);
    let focus = s.focus();
    let bound = terminals(&s)[0].1.clone().unwrap();
    // Split in herdr's own UI: a new pane in the tile's herdr tab.
    let pane = herdr::TerminalRef {
        workspace_id: herdr::WorkspaceId("w1".into()),
        tab_id: herdr::TabId(bound.tab_id.clone()),
        pane_id: herdr::PaneId("w1:p99".into()),
        terminal_id: herdr::TerminalId("term_99".into()),
    };
    fake.state().panes.push(pane.clone());
    fake.send(herdr::StreamItem::Event(herdr::HerdrEvent::PaneCreated(
        pane_info(&pane),
    )));
    let s = wait_until(&core, |s| terminals(s).len() == 2);
    let tab = s.tab(focus.tab.unwrap()).unwrap();
    assert_eq!(tab.tiles.len(), 2, "same tab as its sibling");
    assert_eq!(s.focus(), focus, "focus stays put");
    assert!(fake.mutations().iter().all(|c| *c != "create_terminal"));

    // Closed there again: the tile goes.
    fake.state().panes.retain(|p| p.pane_id.0 != "w1:p99");
    fake.send(herdr::StreamItem::Event(herdr::HerdrEvent::PaneClosed {
        pane_id: herdr::PaneId("w1:p99".into()),
        workspace_id: herdr::WorkspaceId("w1".into()),
    }));
    wait_until(&core, |s| terminals(s).len() == 1);
}

#[test]
fn a_tiles_own_on_exit_beats_the_config() {
    let bound = |pane: &str, on_exit: Option<OnExit>| {
        azulejo_model::Tile::new(TileContent::Terminal {
            cwd: None,
            command: None,
            env: Default::default(),
            herdr: Some(HerdrRef {
                pane_id: pane.into(),
                terminal_id: format!("term_{pane}"),
                tab_id: "w1:t1".into(),
                machine: None,
            }),
            ended: false,
            on_exit,
        })
    };
    let mut session = Session::new();
    let (_, workspace) = session.create_window("main", None);
    let plain = bound("w1:p1", None);
    let kept = bound("w1:p2", Some(OnExit::Keep));
    let closed = bound("w1:p3", Some(OnExit::Close));
    let (plain_id, kept_id, closed_id) = (plain.id, kept.id, closed.id);
    session.create_tab(workspace, plain, None).unwrap();
    session.split(plain_id, Direction::Right, kept).unwrap();
    session.split(kept_id, Direction::Down, closed).unwrap();

    let exit = |pane: &str, default| {
        crate::link::reconcile(
            &None,
            &herdr::StreamItem::Event(herdr::HerdrEvent::PaneExited {
                pane_id: herdr::PaneId(pane.into()),
                workspace_id: herdr::WorkspaceId("w1".into()),
            }),
            &session,
            default,
        )
        .internals
    };
    use crate::link::Internal::{ClosedElsewhere, MarkEnded};
    assert_eq!(
        exit("w1:p1", OnExit::Close),
        [ClosedElsewhere { tile: plain_id }]
    );
    assert_eq!(exit("w1:p1", OnExit::Keep), [MarkEnded { tile: plain_id }]);
    assert_eq!(exit("w1:p2", OnExit::Close), [MarkEnded { tile: kept_id }]);
    assert_eq!(
        exit("w1:p3", OnExit::Keep),
        [ClosedElsewhere { tile: closed_id }]
    );
    assert!(exit("w9:p9", OnExit::Close).is_empty());
}

// ----- several herdr machines (ADR 0011) -----

const REMOTE: &str = "0123456789abcdef0123456789abcdef";

fn on_machine(s: &Session, machine: Option<&str>) -> Vec<(TileId, HerdrRef)> {
    terminals(s)
        .into_iter()
        .filter_map(|(id, h, _)| h.map(|h| (id, h)))
        .filter(|(_, h)| h.machine.as_deref() == machine)
        .collect()
}

fn machines(core: &Core) -> Vec<azulejo_protocol::MachineInfo> {
    match core.execute(Command::MachinesList).unwrap().reply {
        Reply::Machines { machines } => machines,
        other => panic!("{other:?}"),
    }
}

/// Local and a remote machine whose herdr uses the very same IDs (`w1:p1`
/// …): every match must look at the machine first.
fn two_machines() -> (Core, Fake, Fake) {
    let local = Fake::with_existing(crate::adopt::tests::fixture());
    let remote = Fake::with_existing(crate::adopt::tests::fixture());
    let core = Core::start_with_herdr(opts(None), local.clone());
    wait_until(&core, |s| on_machine(s, None).len() == 6);
    core.add_machine(REMOTE.into(), "dev-server".into(), remote.clone());
    wait_until(&core, |s| on_machine(s, Some(REMOTE)).len() == 6);
    (core, local, remote)
}

#[test]
fn a_remote_machines_herdr_is_adopted_beside_locals() {
    let (core, local, remote) = two_machines();
    let s = core.session().unwrap();
    let ws: Vec<_> = s.windows[0].workspaces.iter().collect();
    assert_eq!(ws.len(), 4, "two herdr workspaces per machine");
    assert_eq!(
        ws.iter()
            .filter(|w| w.machine.as_deref() == Some(REMOTE))
            .count(),
        2
    );
    // Same herdr IDs on both machines, kept apart.
    let local_p1 = on_machine(&s, None)
        .into_iter()
        .find(|(_, h)| h.pane_id == "w1:p1")
        .unwrap();
    let remote_p1 = on_machine(&s, Some(REMOTE))
        .into_iter()
        .find(|(_, h)| h.pane_id == "w1:p1")
        .unwrap();
    assert_ne!(local_p1.0, remote_p1.0);
    // Adopting the remote one only read the remote herdr.
    wait_for_calls(&remote, "reresolve", 6);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(remote.mutations(), Vec::<&str>::new());
    assert_eq!(local.mutations(), Vec::<&str>::new());

    // herdr closing w1:p1 on the remote machine removes only its tile.
    remote.send(herdr::StreamItem::Event(herdr::HerdrEvent::PaneClosed {
        pane_id: herdr::PaneId("w1:p1".into()),
        workspace_id: herdr::WorkspaceId("w1".into()),
    }));
    let s = wait_until(&core, |s| on_machine(s, Some(REMOTE)).len() == 5);
    assert!(s.tile(local_p1.0).is_ok(), "Local's w1:p1 stays");

    // Agent status for w1:p2 on Local doesn't touch the remote tile.
    let remote_p2 = on_machine(&s, Some(REMOTE))
        .into_iter()
        .find(|(_, h)| h.pane_id == "w1:p2")
        .unwrap()
        .0;
    let local_p2 = on_machine(&s, None)
        .into_iter()
        .find(|(_, h)| h.pane_id == "w1:p2")
        .unwrap()
        .0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while local.state().agent_tx.is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    local.send_agent("w1:p2", herdr::AgentStatus::Blocked);
    let s = wait_until(&core, |s| agent_of(s, local_p2).is_some());
    assert!(agent_of(&s, remote_p2).is_none());
    core.shutdown();
}

#[test]
fn workspaces_on_a_machine_get_their_terminals_there() {
    let (core, local, remote) = two_machines();
    // As the daemon does from the default config: new tabs are dynamic Tile.
    core.set_layout_prefs(azulejo_model::LayoutPrefs {
        default: azulejo_model::LayoutKind::Tile,
        ..azulejo_model::LayoutPrefs::default()
    });
    let created_local = local.state().created_terminals.len();
    let reply = core
        .execute(Command::WorkspaceCreate(
            azulejo_protocol::WorkspaceCreate {
                window: None,
                name: Some("remote work".into()),
                cwd: None,
                machine: Some("dev-server".into()),
            },
        ))
        .unwrap();
    let Reply::Created(created) = reply.reply else {
        panic!()
    };
    let ws = created.workspace.unwrap();
    assert_eq!(
        core.session()
            .unwrap()
            .workspace(ws)
            .unwrap()
            .machine
            .as_deref(),
        Some(REMOTE),
        "by label"
    );
    // A terminal in it gets a pane from the remote herdr.
    let tab = core
        .execute(Command::TabCreate(azulejo_protocol::TabCreate {
            workspace: Some(ws),
            ..Default::default()
        }))
        .unwrap();
    assert!(matches!(tab.reply, Reply::Created(_)));
    let s = wait_until(&core, |s| on_machine(s, Some(REMOTE)).len() == 7);
    assert_eq!(
        remote.state().created_workspaces,
        101,
        "a remote herdr workspace"
    );
    assert_eq!(local.state().created_terminals.len(), created_local);
    let ws_tiles: Vec<TileId> = s
        .workspace(ws)
        .unwrap()
        .tabs
        .iter()
        .flat_map(|t| t.tiles.iter().map(|t| t.id))
        .collect();
    assert!(
        on_machine(&s, Some(REMOTE))
            .iter()
            .any(|(id, _)| ws_tiles.contains(id))
    );
    // A new tab there is a dynamic Tile tab (ADR 0010), and tiles it gains
    // get their panes from the same remote herdr (ADR 0011).
    let new_tab = s.workspace(ws).unwrap().tabs.last().unwrap();
    assert!(new_tab.is_dynamic(), "new tabs default to dynamic Tile");
    let new_tab = new_tab.id;
    core.execute(Command::LayoutSplitAuto(
        azulejo_protocol::LayoutSplitAuto {
            tab: Some(new_tab),
            content: None,
        },
    ))
    .unwrap();
    let s = wait_until(&core, |s| on_machine(s, Some(REMOTE)).len() == 8);
    assert!(s.workspace(ws).unwrap().tabs.last().unwrap().is_dynamic());
    assert_eq!(local.state().created_terminals.len(), created_local);
    // Unknown machines are refused.
    let err = core
        .execute(Command::WorkspaceCreate(
            azulejo_protocol::WorkspaceCreate {
                machine: Some("nope".into()),
                ..Default::default()
            },
        ))
        .unwrap_err();
    assert_eq!(err.code, azulejo_protocol::ErrorCode::InvalidParams);
    core.shutdown();
}

#[test]
fn each_machine_has_its_own_connection_state() {
    let (core, local, remote) = two_machines();
    // The fake doesn't say Connected by itself; the real stream does.
    local.send(herdr::StreamItem::Connected);
    let deadline = Instant::now() + Duration::from_secs(5);
    while machines(&core)[0].state != azulejo_protocol::MachineState::Connected {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let events = core.subscribe();
    core.session().unwrap();
    let list = machines(&core);
    assert_eq!(list[0].id, azulejo_protocol::LOCAL_MACHINE);
    assert_eq!(list[1].label, "dev-server");

    remote.send(herdr::StreamItem::Disconnected(
        "machine dev-server: ssh: connect to host dev-server port 22: Connection refused".into(),
    ));
    let status = |want: azulejo_protocol::MachineState| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline, "no {want:?}");
            match events.recv_timeout(Duration::from_millis(50)) {
                Ok(Event::MachineStatus { machine }) if machine.state == want => return machine,
                Ok(Event::HerdrStatus { .. }) => panic!("Local's status must not change"),
                _ => {}
            }
        }
    };
    let m = status(azulejo_protocol::MachineState::Reconnecting);
    assert_eq!(m.id, REMOTE);
    assert!(m.message.unwrap().contains("Connection refused"));
    let list = machines(&core);
    assert_eq!(list[0].state, azulejo_protocol::MachineState::Connected);
    assert_eq!(list[1].state, azulejo_protocol::MachineState::Reconnecting);

    // A host key problem won't fix itself by retrying.
    remote.send(herdr::StreamItem::Disconnected(
        "machine dev-server: Host key verification failed.".into(),
    ));
    status(azulejo_protocol::MachineState::Error);
    remote.send(herdr::StreamItem::Resync);
    status(azulejo_protocol::MachineState::Connected);

    // Removed from herdr: listed no more, tiles stay.
    let tiles = on_machine(&core.session().unwrap(), Some(REMOTE)).len();
    core.remove_machine(REMOTE.into());
    let deadline = Instant::now() + Duration::from_secs(5);
    while machines(&core).len() != 1 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        on_machine(&core.session().unwrap(), Some(REMOTE)).len(),
        tiles
    );
    // Terminal I/O to a tile on a machine that's gone says so.
    let (tile, _) = on_machine(&core.session().unwrap(), Some(REMOTE))[0].clone();
    let err = core
        .execute(Command::TerminalRead(azulejo_protocol::TerminalRead {
            tile: Some(tile),
            ..Default::default()
        }))
        .unwrap_err();
    assert_eq!(err.code, azulejo_protocol::ErrorCode::Unavailable);
    core.shutdown();
}

fn process(shell_pid: u32, fg: (u32, &str, &str), cwd: &str) -> herdr::ProcessInfo {
    herdr::ProcessInfo {
        shell_pid: Some(shell_pid),
        foreground_process_group_id: Some(fg.0),
        foreground_processes: vec![herdr::ProcessEntry {
            pid: fg.0,
            name: fg.1.into(),
            cmdline: fg.2.into(),
            cwd: Some(cwd.into()),
        }],
    }
}

fn header(session: &Session, tile: TileId) -> (String, Option<String>) {
    let h = session.tile(tile).unwrap().header.clone().unwrap();
    (h.title, h.subtitle)
}

#[test]
fn terminal_headers_follow_herdr_titles_and_processes() {
    let fake = Fake::default();
    fake.state().processes = Some(HashMap::new());
    let core = Core::start_with_herdr(opts(None), fake.clone());
    let session = wait_until(&core, all_bound);
    let (tile, pane) = match &terminals(&session)[0] {
        (id, Some(h), _) => (*id, h.pane_id.clone()),
        other => panic!("unbound {other:?}"),
    };
    let events = core.subscribe();

    // The shell at its prompt: its name and where it is.
    fake.state()
        .processes
        .as_mut()
        .unwrap()
        .insert(pane.clone(), process(10, (10, "zsh", "-zsh"), "/srv/shop"));
    wait_until(&core, |s| {
        header(s, tile) == ("zsh".into(), Some("/srv/shop".into()))
    });

    // A command in the foreground replaces the folder.
    fake.state().processes.as_mut().unwrap().insert(
        pane.clone(),
        process(
            10,
            (20, "just", "/opt/homebrew/bin/just dogfood"),
            "/srv/shop",
        ),
    );
    let started = Instant::now();
    let s = wait_until(&core, |s| {
        header(s, tile).1.as_deref() == Some("just dogfood")
    });
    assert_eq!(header(&s, tile).0, "zsh", "keeps the shell it saw");
    assert!(started.elapsed() < meta::PROCESS_POLL * 3);

    // herdr's title for the pane (OSC 2) lands on the tile too.
    let mut info = s
        .tile(tile)
        .ok()
        .and_then(|t| match &t.content {
            TileContent::Terminal { herdr: Some(h), .. } => Some(h.clone()),
            _ => None,
        })
        .map(|h| {
            pane_info(&herdr::TerminalRef {
                workspace_id: herdr::WorkspaceId("w1".into()),
                tab_id: herdr::TabId(h.tab_id),
                pane_id: herdr::PaneId(h.pane_id),
                terminal_id: herdr::TerminalId(h.terminal_id),
            })
        })
        .unwrap();
    info.terminal_title = Some("building…".into());
    fake.send(herdr::StreamItem::Event(herdr::HerdrEvent::PaneUpdated(
        info,
    )));
    let s = wait_until(&core, |s| {
        s.tile(tile)
            .ok()
            .and_then(|t| t.pane.as_ref())
            .and_then(|m| m.title.clone())
            .as_deref()
            == Some("building…")
    });
    assert_eq!(header(&s, tile).1.as_deref(), Some("just dogfood"));

    // Shells hear of it as tile updates with the new header.
    let saw_header = events.try_iter().any(|e| {
        matches!(e, Event::TileUpdated { tile: t, .. }
            if t.id == tile && t.header.as_ref().and_then(|h| h.subtitle.as_deref()) == Some("just dogfood"))
    });
    assert!(saw_header);

    // Back at the prompt.
    fake.state()
        .processes
        .as_mut()
        .unwrap()
        .insert(pane, process(10, (10, "zsh", "-zsh"), "/srv/shop/web"));
    wait_until(&core, |s| {
        header(s, tile) == ("zsh".into(), Some("/srv/shop/web".into()))
    });
}

#[test]
fn a_port_without_process_info_stops_polling() {
    // The default fake has no process info: headers fall back, nothing hangs.
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    let session = wait_until(&core, all_bound);
    let tile = terminals(&session)[0].0;
    assert_eq!(header(&session, tile).0, "terminal");
}

/// herdr streams agent statuses only once it accepts the subscription: a
/// status that changed before then (here: blocked, never streamed) still
/// reaches the tile when the subscription goes live, without adopting or
/// changing anything in herdr.
#[test]
fn a_status_from_before_the_agent_subscription_went_live_still_arrives() {
    use azulejo_model::AgentStatus;
    let fake = Fake::default();
    fake.state().agent_unconfirmed = true;
    let core = Core::start_with_herdr(opts(None), fake.clone());
    let s = wait_until(&core, all_bound);
    let tile = s.focused_tile().unwrap();
    let pane = terminals(&s)[0].1.clone().unwrap().pane_id;
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.state().agent_subs.last() != Some(&vec![pane.clone()]) {
        assert!(Instant::now() < deadline, "{:?}", fake.state().agent_subs);
        std::thread::sleep(Duration::from_millis(5));
    }
    let before = fake.mutations();
    // The agent blocks while herdr is still accepting the subscription.
    fake.state()
        .agent_statuses
        .insert(pane.clone(), herdr::AgentStatus::Blocked);
    std::thread::sleep(Duration::from_millis(50));
    assert!(agent_of(&core.session().unwrap(), tile).is_none());
    // herdr accepts it.
    let tx = fake.state().agent_tx.clone().unwrap();
    tx.blocking_send(herdr::StreamItem::Connected).unwrap();
    let s = wait_until(&core, |s| agent_of(s, tile).is_some_and(|a| a.attention));
    assert_eq!(agent_of(&s, tile).unwrap().status, AgentStatus::Blocked);
    assert_eq!(fake.mutations(), before, "reading statuses changes nothing");
    assert_eq!(terminals(&s).len(), 1, "and adopts nothing");
}
