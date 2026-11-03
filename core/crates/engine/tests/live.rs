//! The engine against a real herdr, in a throwaway named session. Ignored by
//! default; run with `just test-herdr`. Never touches the user's default
//! session: herdr runs with a clean environment and an explicit `--session`.

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use azulejo_engine::{Core, CoreOptions};
use azulejo_herdr::{Backoff, Client, ClientOptions, Herdr, Session as HerdrSession};
use azulejo_model::{HerdrRef, OnExit, Session, TileContent, TileId};
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

    /// Ends a pane's shell, as a crash or `exit` would: herdr drops the pane
    /// and reports `pane_exited`. Kills the shell rather than typing `exit`,
    /// which a still-starting shell may swallow.
    fn exit_shell(&self, pane: &str) {
        let out = self.cli(&["pane", "process-info", "--pane", pane]);
        let pid = serde_json::from_str::<serde_json::Value>(&out)
            .ok()
            .and_then(|v| v["result"]["process_info"]["shell_pid"].as_i64())
            .unwrap_or_else(|| panic!("process-info: {out}"));
        let status = std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap();
        assert!(status.success(), "kill {pid}");
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

    // 3. A pane closed elsewhere takes its tile with it (F15).
    let events = core.subscribe();
    herdr.kill(&refs[0].pane_id);
    wait_for_event(&events, |e| {
        matches!(e, Event::TileClosed { tile, kill: false, .. }
            if matches!(&tile.content, TileContent::Terminal { herdr: Some(h), .. } if *h == refs[0]))
    });

    // 4. Closing a tile with `kill` kills its pane; the other is gone already.
    let live = bound_terminals(&core.session().unwrap())
        .into_iter()
        .find(|(_, _, ended)| !ended)
        .unwrap();
    assert_eq!(live.1.as_ref(), Some(&refs[1]));
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
    // The exit at the end checks I/O on an ended tile.
    core.set_on_exit(OnExit::Keep);
    assert_eq!(core.herdr_socket(), h.client.socket_path());
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

    // The shell exits: herdr drops the pane, the tile ends and I/O says so.
    assert_eq!(
        reply(Command::TerminalSendText(TerminalSendText {
            tile: Some(tile),
            text: "exit".into(),
        })),
        Ok(Reply::Ok)
    );
    reply(Command::TerminalSendKeys(TerminalSendKeys {
        tile: Some(tile),
        keys: vec!["enter".into()],
    }))
    .unwrap();
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

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn respawn_rebinds_an_ended_tile_in_place() {
    use azulejo_protocol::TileTarget;
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = Core::start_with_herdr(opts(&dir.path().join("state.json")), herdr.client.clone());
    // Respawn is for tiles kept after their shell exits.
    core.set_on_exit(OnExit::Keep);
    core.execute(Command::LayoutSplit(LayoutSplit {
        tile: None,
        direction: Direction::Right,
        content: None,
    }))
    .unwrap();
    let session = wait_until(&core, Duration::from_secs(10), |s| {
        bound_terminals(s).len() == 2 && all_bound(s)
    });
    let tree = session.windows[0].workspaces[0].tabs[0].root.clone();
    let tiles = bound_terminals(&session);
    let ended = |s: &Session, tile: TileId| {
        bound_terminals(s)
            .iter()
            .any(|(id, _, e)| *id == tile && *e)
    };
    let fresh = |s: &Session, tile: TileId, old: &HerdrRef| {
        bound_terminals(s)
            .iter()
            .any(|(id, h, e)| *id == tile && !e && h.as_ref().is_some_and(|h| h != old))
    };

    // 1. One pane ends; its workspace lives on. Respawn binds a new pane.
    let (second, old) = (tiles[1].0, tiles[1].1.clone().unwrap());
    herdr.exit_shell(&old.pane_id);
    wait_until(&core, Duration::from_secs(5), |s| ended(s, second));
    core.execute(Command::TerminalRespawn(TileTarget { tile: Some(second) }))
        .unwrap();
    let after = wait_until(&core, Duration::from_secs(10), |s| fresh(s, second, &old));
    let new = bound_terminals(&after)[1].1.clone().unwrap();
    assert!(
        herdr
            .panes()
            .iter()
            .any(|p| p.terminal_id.as_str() == new.terminal_id),
        "respawned pane runs in herdr"
    );
    assert_ne!(new.tab_id, tiles[0].1.as_ref().unwrap().tab_id, "own tab");
    assert_eq!(
        after.windows[0].workspaces[0].tabs[0].root, tree,
        "same place"
    );

    // 2. Every pane ends, so herdr drops the workspace. Respawn makes a new one.
    let first_old = tiles[0].1.clone().unwrap();
    herdr.exit_shell(&first_old.pane_id);
    herdr.exit_shell(&new.pane_id);
    wait_until(&core, Duration::from_secs(5), |s| {
        ended(s, tiles[0].0) && ended(s, second)
    });
    core.execute(Command::TerminalRespawn(TileTarget {
        tile: Some(tiles[0].0),
    }))
    .unwrap();
    let last = wait_until(&core, Duration::from_secs(10), |s| {
        fresh(s, tiles[0].0, &first_old)
    });
    let r = bound_terminals(&last)[0].1.clone().unwrap();
    assert!(
        herdr
            .panes()
            .iter()
            .any(|p| p.terminal_id.as_str() == r.terminal_id)
    );
    core.shutdown();
}

/// Waits for the first event `want` accepts; returns when it arrived.
fn time_event(events: &Receiver<Event>, want: impl Fn(&Event) -> bool) -> Instant {
    wait_for_event(events, want);
    Instant::now()
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn agent_status_and_changes_made_in_herdr_reach_the_core_within_a_second() {
    use azulejo_protocol::AgentState;
    let h = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = Core::start_with_herdr(opts(&dir.path().join("state.json")), h.client.clone());
    let session = wait_until(&core, Duration::from_secs(10), all_bound);
    let tile = session.focused_tile().unwrap();
    let pane = match &session.tile(tile).unwrap().content {
        TileContent::Terminal { herdr: Some(r), .. } => r.pane_id.clone(),
        other => panic!("{other:?}"),
    };
    let events = core.subscribe();
    let report = |state: &str| {
        h.cli(&[
            "pane",
            "report-agent",
            &pane,
            "--source",
            "azj-test",
            "--agent",
            "claude",
            "--state",
            state,
        ]);
    };

    // The agent subscription opens once the tile is bound; wait for it.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        report("working");
        let got = (0..20).any(|_| {
            matches!(
                events.recv_timeout(Duration::from_millis(50)),
                Ok(Event::AgentStateChanged(c)) if c.state == AgentState::Working
            )
        });
        if got {
            break;
        }
        assert!(Instant::now() < deadline, "no agent status from herdr");
    }

    // Blocked: lights the tile's attention (it's focused, so `tile.seen`
    // is the shell's job; the core lights it regardless).
    let t0 = Instant::now();
    report("blocked");
    let blocked = time_event(
        &events,
        |e| matches!(e, Event::AgentStateChanged(c) if c.tile == tile && c.state == AgentState::Blocked),
    );
    let attention = time_event(
        &events,
        |e| matches!(e, Event::TileAttentionChanged { tile: t, attention: true, .. } if *t == tile),
    );
    let blocked_ms = (blocked - t0).as_millis();
    eprintln!(
        "LATENCY blocked: {blocked_ms} ms to agent.state_changed, {} ms to attention (incl. herdr CLI)",
        (attention - t0).as_millis()
    );
    assert!(blocked_ms < 1000, "{blocked_ms} ms");
    let a = core
        .session()
        .unwrap()
        .tile(tile)
        .unwrap()
        .agent
        .clone()
        .unwrap();
    assert!(a.attention);

    // A pane split off in herdr becomes a tile beside its sibling (F15).
    let t0 = Instant::now();
    let out = h.cli(&["pane", "split", &pane, "--direction", "right", "--no-focus"]);
    let new_pane = serde_json::from_str::<serde_json::Value>(&out)
        .ok()
        .and_then(|v| v["result"]["pane"]["pane_id"].as_str().map(str::to_owned))
        .unwrap_or_else(|| panic!("split output: {out}"));
    let created = time_event(&events, |e| {
        matches!(e, Event::TileCreated { tile, .. }
            if matches!(&tile.content, TileContent::Terminal { herdr: Some(r), .. } if r.pane_id == new_pane))
    });
    let split_ms = (created - t0).as_millis();
    eprintln!("LATENCY external split: {split_ms} ms to tile.created (incl. herdr CLI)");
    assert!(split_ms < 1000, "{split_ms} ms");
    let s = core.session().unwrap();
    let tab = s.tab(s.focus().tab.unwrap()).unwrap();
    assert_eq!(tab.tiles.len(), 2);
    assert_eq!(s.focused_tile(), Some(tile), "focus stays put");

    // Closed there: the tile goes.
    let t0 = Instant::now();
    h.cli(&["pane", "close", &new_pane]);
    let closed = time_event(&events, |e| {
        matches!(e, Event::TileClosed { tile, .. }
            if matches!(&tile.content, TileContent::Terminal { herdr: Some(r), .. } if r.pane_id == new_pane))
    });
    let close_ms = (closed - t0).as_millis();
    eprintln!("LATENCY external close: {close_ms} ms to tile.closed (incl. herdr CLI)");
    assert!(close_ms < 1000, "{close_ms} ms");
    core.shutdown();
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn a_shell_that_exits_closes_its_tile_and_leaves_nothing_in_herdr() {
    use azulejo_protocol::{ReadSource, Reply, TerminalRead, TerminalSendKeys};
    let h = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = Core::start_with_herdr(opts(&dir.path().join("state.json")), h.client.clone());
    for _ in 0..2 {
        core.execute(Command::LayoutSplit(LayoutSplit {
            tile: None,
            direction: Direction::Down,
            content: None,
        }))
        .unwrap();
    }
    let session = wait_until(&core, Duration::from_secs(10), |s| {
        bound_terminals(s).len() == 3 && all_bound(s)
    });
    let tiles = bound_terminals(&session);
    // Wait for a shell to draw its prompt, so ctrl+d reaches a reading shell.
    let ready = |tile: TileId| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text = match core.execute(Command::TerminalRead(TerminalRead {
                tile: Some(tile),
                source: ReadSource::Visible,
                lines: None,
            })) {
                Ok(o) => match o.reply {
                    Reply::Text { text, .. } => text,
                    other => panic!("{other:?}"),
                },
                Err(_) => String::new(),
            };
            if !text.trim().is_empty() {
                std::thread::sleep(Duration::from_millis(300));
                return;
            }
            assert!(Instant::now() < deadline, "no prompt in {tile}");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    let ctrl_d = |tile: TileId| {
        core.execute(Command::TerminalSendKeys(TerminalSendKeys {
            tile: Some(tile),
            keys: vec!["ctrl+d".into()],
        }))
        .unwrap();
    };

    // The default: ctrl+d closes the tile, and herdr has dropped the pane and
    // the tab Azulejo made for it.
    let (gone, gone_ref, _) = tiles[2].clone();
    let gone_ref = gone_ref.unwrap();
    ready(gone);
    let start = Instant::now();
    ctrl_d(gone);
    let after = wait_until(&core, Duration::from_secs(5), |s| s.tile(gone).is_err());
    let took = start.elapsed();
    eprintln!("ctrl+d → tile closed: {took:?}");
    assert!(took < Duration::from_secs(1), "took {took:?}");
    assert_eq!(bound_terminals(&after).len(), 2);
    assert!(after.check_invariants().is_ok());
    let structure = h.structure();
    assert!(
        !structure.contains(&gone_ref.pane_id) && !structure.contains(&gone_ref.tab_id),
        "left behind in herdr:\n{structure}"
    );

    // `keep`: the tile stays, ended, for Respawn.
    core.set_on_exit(OnExit::Keep);
    let (kept, _, _) = tiles[1].clone();
    ready(kept);
    ctrl_d(kept);
    let after = wait_until(&core, Duration::from_secs(5), |s| {
        matches!(
            s.tile(kept).map(|t| &t.content),
            Ok(TileContent::Terminal { ended: true, .. })
        )
    });
    assert_eq!(bound_terminals(&after).len(), 2);
    core.shutdown();
}

fn on_machine(s: &Session, machine: Option<&str>) -> usize {
    bound_terminals(s)
        .into_iter()
        .filter(|(_, h, _)| h.as_ref().is_some_and(|h| h.machine.as_deref() == machine))
        .count()
}

fn machine_state(e: &Event, id: &str, state: azulejo_protocol::MachineState) -> bool {
    matches!(e, Event::MachineStatus { machine } if machine.id == id && machine.state == state)
}

/// herdr's saved machines (ADR 0011), simulated: a second throwaway session
/// reached only through the SSH path (an `ssh` stand-in that runs the remote
/// command here) and herdr's `remote-api-bridge`.
#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn a_remote_machine_alongside_local() {
    use azulejo_protocol::MachineState::Connected;
    let local = TempHerdr::start();
    let remote = TempHerdr::start();
    remote.cli(&["workspace", "create", "--label", "on-remote"]);
    let dir = std::env::temp_dir().join(format!("azj-live-mm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ssh = dir.join("ssh");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/ssh-standin.sh"),
        &ssh,
    )
    .unwrap();
    let fail = std::path::PathBuf::from(format!("{}.fail", ssh.display()));
    let machine = azulejo_herdr::RemoteMachine::new(
        azulejo_herdr::MachineProfile {
            id: "0123456789abcdef0123456789abcdef".into(),
            label: "standin".into(),
            target: "standin-host".into(),
            session: remote.name.clone(),
            enabled: true,
        },
        azulejo_herdr::Ssh {
            program: ssh.clone(),
            control_dir: dir.clone(),
        },
    );
    let id = machine.profile.id.clone();

    let core = Core::start_with_herdr(opts(&dir.join("state.json")), local.client.clone());
    let events = core.subscribe();
    wait_until(&core, Duration::from_secs(10), all_bound);
    let local_tiles = on_machine(&core.session().unwrap(), None);
    let before_local = local.structure();
    core.add_machine(id.clone(), "standin".into(), machine.client());
    // Adopted over SSH: the remote session's workspace and its pane.
    let s = wait_until(&core, Duration::from_secs(15), |s| {
        s.windows[0]
            .workspaces
            .iter()
            .any(|w| w.name == "on-remote" && w.machine.as_deref() == Some(id.as_str()))
            && on_machine(s, Some(&id)) == 1
    });
    assert_eq!(on_machine(&s, None), local_tiles);
    wait_for_event(&events, |e| machine_state(e, &id, Connected));

    // A workspace made on the machine runs its shell there.
    let t0 = Instant::now();
    let reply = core
        .execute(Command::WorkspaceCreate(
            azulejo_protocol::WorkspaceCreate {
                name: Some("made-here".into()),
                machine: Some("standin".into()),
                ..Default::default()
            },
        ))
        .unwrap();
    let azulejo_protocol::Reply::Created(created) = reply.reply else {
        panic!("{reply:?}")
    };
    let ws_id = created.workspace.unwrap();
    core.execute(Command::TabCreate(azulejo_protocol::TabCreate {
        workspace: Some(ws_id),
        ..Default::default()
    }))
    .unwrap();
    let s = wait_until(&core, Duration::from_secs(15), |s| {
        let ws = s.workspace(ws_id).unwrap();
        !ws.tabs.is_empty()
            && ws.tabs.iter().flat_map(|t| &t.tiles).all(|t| {
                matches!(&t.content, TileContent::Terminal { herdr: Some(h), .. } if h.machine.is_some())
            })
    });
    eprintln!("remote workspace + terminal: {:?}", t0.elapsed());
    let herdr_ws = s.workspace(ws_id).unwrap().herdr_workspace.clone().unwrap();
    assert!(
        remote
            .panes()
            .iter()
            .any(|p| p.workspace_id.as_str() == herdr_ws),
        "the pane is in the remote herdr"
    );
    assert_eq!(local.structure(), before_local, "Local's herdr untouched");

    // Cut the "SSH link": end the running bridges as a dropped link would
    // end ssh, and refuse new ones. Only that machine reconnects.
    std::fs::write(&fail, "").unwrap();
    let _ = std::process::Command::new("pkill")
        .args(["-f", &format!("{} remote-api-bridge", remote.name)])
        .status();
    let down = wait_for_event(
        &events,
        |e| matches!(e, Event::MachineStatus { machine } if machine.id == id && machine.state != Connected),
    );
    eprintln!("link down: {down:?}");
    std::fs::remove_file(&fail).unwrap();
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(e) if machine_state(&e, &id, Connected) => break,
            Ok(Event::HerdrStatus {
                connected: false, ..
            }) => panic!("Local dropped too"),
            Ok(_) => {}
            Err(_) => panic!("machine never came back"),
        }
    }
    eprintln!("link back after {:?}", t0.elapsed());
    core.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
