//! The engine against a real herdr, in a throwaway named session. Ignored by
//! default; run with `just test-herdr`. Never touches the user's default
//! session: herdr runs with a clean environment and an explicit `--session`.

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use azulejo_engine::{Core, CoreOptions};
use azulejo_herdr::{Backoff, Client, ClientOptions, Herdr, Session as HerdrSession};
use azulejo_model::{HerdrRef, Session, TileContent, TileId};
use azulejo_protocol::{Command, Direction, Event, LayoutSplit, TileClose};

fn quick() -> ClientOptions {
    ClientOptions {
        request_timeout: Duration::from_secs(5),
        connect: Backoff {
            initial: Duration::from_millis(50),
            max: Duration::from_millis(500),
            give_up_after: Duration::from_secs(10),
        },
    }
}

/// A herdr server in its own session; stopped and deleted on drop.
struct TempHerdr {
    rt: tokio::runtime::Runtime,
    herdr: Herdr,
    name: String,
    client: Client,
}

impl TempHerdr {
    fn start() -> Self {
        // Never inherit the caller's herdr session.
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("HERDR_") {
                // SAFETY: tests run single-threaded (`--test-threads=1`).
                unsafe { std::env::remove_var(key) };
            }
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let herdr = Herdr::locate().expect("herdr on PATH");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let name = format!("azj-test-engine-{}-{nanos:x}", std::process::id());
        let (client, _) = rt
            .block_on(herdr.ensure_server(&HerdrSession::named(&name), quick()))
            .expect("herdr server");
        Self {
            rt,
            herdr,
            name,
            client,
        }
    }

    fn panes(&self) -> Vec<azulejo_herdr::PaneInfo> {
        self.rt.block_on(self.client.panes()).unwrap()
    }

    /// Runs a herdr CLI command against this session.
    fn cli(&self, args: &[&str]) -> String {
        self.rt
            .block_on(self.herdr.cli(&HerdrSession::named(&self.name), args))
            .unwrap_or_else(|e| panic!("herdr {args:?}: {e}"))
    }

    /// herdr's structure: workspaces, tabs, panes and each tab's split tree.
    fn structure(&self) -> String {
        let snap = self.rt.block_on(self.client.snapshot()).unwrap();
        let mut out = Vec::new();
        for w in &snap.workspaces {
            out.push(format!("ws {} {:?}", w.workspace_id, w.label));
        }
        for t in &snap.tabs {
            let layout = self
                .rt
                .block_on(self.client.layout_export(&t.tab_id))
                .unwrap();
            out.push(format!("tab {} {:?} {:?}", t.tab_id, t.label, layout.root));
        }
        for p in &snap.panes {
            out.push(format!("pane {} {} {}", p.pane_id, p.terminal_id, p.tab_id));
        }
        out.join("\n")
    }

    fn kill(&self, pane: &str) {
        self.rt
            .block_on(
                self.client
                    .kill_terminal(&azulejo_herdr::PaneId(pane.into())),
            )
            .unwrap();
    }
}

impl Drop for TempHerdr {
    fn drop(&mut self) {
        let _ = self.rt.block_on(self.client.stop_server());
        std::thread::sleep(Duration::from_millis(500));
        let _ = self.rt.block_on(self.herdr.delete_session(&self.name));
    }
}

fn opts(state: &std::path::Path) -> CoreOptions {
    CoreOptions {
        state_path: Some(state.to_owned()),
        api_socket: "/tmp/azj-test.sock".into(),
        log: Arc::new(|line| eprintln!("core: {line}")),
    }
}

fn bound_terminals(session: &Session) -> Vec<(TileId, Option<HerdrRef>, bool)> {
    session
        .tiles()
        .filter_map(|(_, t)| match &t.content {
            TileContent::Terminal { herdr, ended, .. } => Some((t.id, herdr.clone(), *ended)),
            _ => None,
        })
        .collect()
}

/// Waits until every terminal tile has a pane (or `timeout`).
fn wait_until(core: &Core, timeout: Duration, ok: impl Fn(&Session) -> bool) -> Session {
    let deadline = Instant::now() + timeout;
    loop {
        let session = core.session().unwrap();
        if ok(&session) {
            return session;
        }
        assert!(
            Instant::now() < deadline,
            "timed out; session: {:#?}",
            bound_terminals(&session)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn all_bound(session: &Session) -> bool {
    bound_terminals(session)
        .iter()
        .all(|(_, h, ended)| h.is_some() && !ended)
}

fn wait_for_event(events: &Receiver<Event>, want: impl Fn(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let event = events.recv_timeout(left).expect("event in time");
        if want(&event) {
            return event;
        }
    }
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn terminals_get_panes_survive_restart_and_follow_herdr() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state.json");

    // 1. Fresh start: the first terminal gets a pane (herdr's first pane in a
    //    new workspace), a split gets a second one in the same workspace.
    let core = Core::start_with_herdr(opts(&state), herdr.client.clone());
    wait_until(&core, Duration::from_secs(10), all_bound);
    core.execute(Command::LayoutSplit(LayoutSplit {
        tile: None,
        direction: Direction::Right,
        content: None,
    }))
    .unwrap();
    let before = wait_until(&core, Duration::from_secs(10), |s| {
        bound_terminals(s).len() == 2 && all_bound(s)
    });
    let herdr_ws = before.windows[0].workspaces[0]
        .herdr_workspace
        .clone()
        .expect("workspace bound");
    let refs: Vec<HerdrRef> = bound_terminals(&before)
        .into_iter()
        .map(|(_, h, _)| h.unwrap())
        .collect();
    let panes = herdr.panes();
    for r in &refs {
        let pane = panes
            .iter()
            .find(|p| p.terminal_id.as_str() == r.terminal_id)
            .unwrap_or_else(|| panic!("no herdr pane for {r:?}"));
        assert_eq!(pane.workspace_id.as_str(), herdr_ws);
    }

    // 2. "kill -9": no shutdown, no final flush. The debounced save already
    //    happened; a new core restores the same layout and the same panes.
    std::thread::sleep(Duration::from_millis(600));
    std::mem::forget(core);
    let core = Core::start_with_herdr(opts(&state), herdr.client.clone());
    let restored = core.session().unwrap();
    assert_eq!(restored, before, "layout restored exactly");
    // Reresolving finds the same panes, so nothing changes.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(core.session().unwrap(), before);
    assert_eq!(herdr.panes().len(), 2, "no extra panes on restore");

    // 3. A pane closed elsewhere marks its tile ended.
    let events = core.subscribe();
    herdr.kill(&refs[0].pane_id);
    wait_for_event(&events, |e| {
        matches!(e, Event::TileUpdated { tile, .. }
            if matches!(&tile.content, TileContent::Terminal { ended: true, .. }))
    });

    // 4. Closing a tile with `kill` kills its pane; the other is gone already.
    let live = bound_terminals(&core.session().unwrap())
        .into_iter()
        .find(|(_, _, ended)| !ended)
        .unwrap();
    core.execute(Command::TileClose(TileClose {
        tile: Some(live.0),
        kill: true,
    }))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while herdr
        .panes()
        .iter()
        .any(|p| p.terminal_id.as_str() == live.1.as_ref().unwrap().terminal_id)
    {
        assert!(Instant::now() < deadline, "pane not killed");
        std::thread::sleep(Duration::from_millis(50));
    }
    core.shutdown();
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn closing_a_tile_without_kill_detaches() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = Core::start_with_herdr(opts(&dir.path().join("state.json")), herdr.client.clone());
    core.execute(Command::LayoutSplit(LayoutSplit {
        tile: None,
        direction: Direction::Down,
        content: None,
    }))
    .unwrap();
    let session = wait_until(&core, Duration::from_secs(10), |s| {
        bound_terminals(s).len() == 2 && all_bound(s)
    });
    let (tile, herdr_ref, _) = bound_terminals(&session).pop().unwrap();
    core.execute(Command::TileClose(TileClose {
        tile: Some(tile),
        kill: false,
    }))
    .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        herdr
            .panes()
            .iter()
            .any(|p| p.terminal_id.as_str() == herdr_ref.as_ref().unwrap().terminal_id),
        "detached pane keeps running"
    );
    core.shutdown();
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn terminal_io_round_trips_through_a_real_pane() {
    use azulejo_protocol::{
        ErrorCode, ReadSource, Reply, TerminalRead, TerminalScroll, TerminalSearch,
        TerminalSendKeys, TerminalSendText,
    };
    let h = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = Core::start_with_herdr(opts(&dir.path().join("state.json")), h.client.clone());
    assert_eq!(core.herdr_socket(), Some(h.client.socket_path()));
    let session = wait_until(&core, Duration::from_secs(10), all_bound);
    let tile = session.focused_tile().unwrap();

    let reply = |c| core.execute(c).map(|o| o.reply);
    assert_eq!(
        reply(Command::TerminalSendText(TerminalSendText {
            tile: Some(tile),
            text: "printf 'azj-%s\\n' marker-42".into(),
        })),
        Ok(Reply::Ok)
    );
    assert_eq!(
        reply(Command::TerminalSendKeys(TerminalSendKeys {
            tile: Some(tile),
            keys: vec!["enter".into()],
        })),
        Ok(Reply::Ok)
    );

    // Only the shell's output has `azj-marker-42` in one piece, so seeing it
    // proves the text and the Enter key both reached the pane.
    let deadline = Instant::now() + Duration::from_secs(10);
    let text = loop {
        let Ok(Reply::Text { text }) = reply(Command::TerminalRead(TerminalRead {
            tile: Some(tile),
            source: ReadSource::Visible,
            lines: None,
        })) else {
            panic!("read failed")
        };
        if text.lines().any(|l| l.trim() == "azj-marker-42") {
            break text;
        }
        assert!(Instant::now() < deadline, "no output; screen:\n{text}");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(text.contains("printf 'azj-%s"), "{text}");

    let Ok(Reply::Matches { matches, .. }) = reply(Command::TerminalSearch(TerminalSearch {
        tile: Some(tile),
        query: "AZJ-MARKER-42".into(),
        case_sensitive: false,
        lines: Some(200),
    })) else {
        panic!("search failed")
    };
    assert!(
        matches.iter().any(|m| m.text.trim() == "azj-marker-42"),
        "{matches:?}"
    );

    let Ok(Reply::Scroll(state)) = reply(Command::TerminalScroll(TerminalScroll {
        tile: Some(tile),
        offset_from_bottom: 0,
    })) else {
        panic!("scroll failed")
    };
    assert_eq!(state.offset_from_bottom, 0);
    assert!(state.viewport_rows > 0, "{state:?}");

    // A pane killed behind the core's back: the tile ends and I/O says so.
    let pane = match &core.session().unwrap().tile(tile).unwrap().content {
        TileContent::Terminal { herdr: Some(r), .. } => r.pane_id.clone(),
        other => panic!("{other:?}"),
    };
    h.kill(&pane);
    wait_until(&core, Duration::from_secs(10), |s| {
        matches!(
            s.tile(tile).map(|t| &t.content),
            Ok(TileContent::Terminal { ended: true, .. })
        )
    });
    let err = reply(Command::TerminalRead(TerminalRead {
        tile: Some(tile),
        source: ReadSource::Visible,
        lines: None,
    }))
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidState);
    core.shutdown();
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn adopts_what_herdr_already_runs_without_changing_it() {
    use azulejo_model::{Axis, Node};
    let h = TempHerdr::start();
    // The user's own herdr: a workspace whose tab is split into three panes.
    h.cli(&["workspace", "create", "--label", "probe"]);
    let first = h.panes()[0].pane_id.clone();
    h.cli(&["pane", "split", first.as_str(), "--direction", "right"]);
    let second = h
        .panes()
        .into_iter()
        .find(|p| p.pane_id != first)
        .unwrap()
        .pane_id;
    h.cli(&[
        "pane",
        "split",
        second.as_str(),
        "--direction",
        "down",
        "--ratio",
        "0.3",
    ]);
    let before = h.structure();
    assert_eq!(h.panes().len(), 3);

    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state.json");
    let core = Core::start_with_herdr(opts(&state), h.client.clone());
    let session = wait_until(&core, Duration::from_secs(10), |s| {
        bound_terminals(s).len() == 3 && all_bound(s)
    });
    // Give restore (re-resolving the adopted tiles) time to run too.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(h.structure(), before, "herdr unchanged");

    let ws = &session.windows[0].workspaces;
    assert_eq!(ws.len(), 1, "the placeholder workspace is dropped");
    assert_eq!(ws[0].name, "probe");
    assert_eq!(ws[0].tabs.len(), 1);
    let tab = &ws[0].tabs[0];
    let pane_of = |id| match &tab.tile(id).unwrap().content {
        TileContent::Terminal { herdr: Some(r), .. } => r.pane_id.clone(),
        other => panic!("{other:?}"),
    };
    let Node::Split { axis, a, b, .. } = &tab.root else {
        panic!("{:?}", tab.root)
    };
    assert_eq!(*axis, Axis::Horizontal);
    let Node::Leaf { id } = **a else { panic!() };
    assert_eq!(pane_of(id), first.as_str());
    let Node::Split { axis, ratio, .. } = &**b else {
        panic!()
    };
    assert_eq!(*axis, Axis::Vertical);
    assert!((ratio - 0.3).abs() < 0.01, "{ratio}");

    // Relaunch: the same session, still no changes in herdr.
    core.shutdown();
    let core = Core::start_with_herdr(opts(&state), h.client.clone());
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(core.session().unwrap(), session);
    assert_eq!(h.structure(), before, "herdr unchanged after relaunch");
    core.shutdown();
}
