//! The running core with a fake herdr.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use azulejo_herdr as herdr;
use azulejo_model::{HerdrRef, Session, TileContent, TileId};
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
            return Ok(herdr::EnsuredWorkspace {
                workspace: info(&id),
                created_terminal: None,
            });
        }
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
        if !s.workspaces.contains(&spec.workspace_id.0) {
            return Err(format!("no workspace {}", spec.workspace_id));
        }
        let t = new_pane(&mut s, &spec.workspace_id.0.clone());
        s.created_terminals.push(spec);
        Ok(t)
    }

    async fn kill_terminal(&self, pane_id: String) -> Result<(), String> {
        let mut s = self.state();
        s.panes.retain(|p| p.pane_id.0 != pane_id);
        s.killed.push(pane_id);
        Ok(())
    }

    async fn reresolve(&self, stored: herdr::TerminalRef) -> Result<herdr::Resolved, String> {
        let s = self.state();
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

    fn events(&self) -> tmpsc::Receiver<herdr::StreamItem> {
        let (tx, rx) = tmpsc::channel(16);
        self.state().events = Some(tx);
        rx
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
fn herdr_events_end_or_rebind_tiles() {
    let fake = Fake::default();
    let core = Core::start_with_herdr(opts(None), fake.clone());
    for _ in 0..3 {
        split(&core);
    }
    let session = wait_until(&core, |s| terminals(s).len() == 4 && all_bound(s));
    let refs: Vec<HerdrRef> = terminals(&session)
        .into_iter()
        .map(|(_, h, _)| h.unwrap())
        .collect();
    let events = core.subscribe();

    use herdr::HerdrEvent as E;
    fake.send(herdr::StreamItem::Event(E::PaneClosed {
        pane_id: herdr::PaneId(refs[0].pane_id.clone()),
        workspace_id: herdr::WorkspaceId("w1".into()),
    }));
    fake.send(herdr::StreamItem::Event(E::TabClosed {
        tab_id: herdr::TabId(refs[1].tab_id.clone()),
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
        let t = terminals(s);
        t[0].2 && t[1].2 && t[2].1.as_ref().unwrap().pane_id == "w2:p1"
    });
    let t = terminals(&after);
    assert!(!t[3].2);
    assert_eq!(t[3].1.as_ref(), Some(&refs[3]));
    // Each change reached listeners as `tile.updated`.
    let updates = events
        .try_iter()
        .filter(|e| matches!(e, Event::TileUpdated { .. }))
        .count();
    assert_eq!(updates, 3);

    // The ended pane was never killed by us.
    assert!(fake.state().killed.is_empty());
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
    let after = wait_until(&core, |s| terminals(s)[1].2);
    assert!(!terminals(&after)[0].2);
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
