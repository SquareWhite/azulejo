//! ADR 0012 against a real herdr, in throwaway named sessions: Azulejo's
//! layout mirrored into herdr and back. Ignored by default; run with
//! `just test-herdr`. Every test asserts it runs on an `azj-test-` session:
//! the mirror (and the migration it does) must never reach the user's own.

use std::sync::Arc;
use std::time::{Duration, Instant};

use azulejo_engine::{Core, CoreOptions};
use azulejo_herdr::{
    Backoff, Client, ClientOptions, Herdr, LayoutNode, PaneId, ReadSource, Session as HerdrSession,
    SplitDirection, TabId as HerdrTab, TabLayout,
};
use azulejo_model::{Axis, HerdrRef, LayoutKind, LayoutPrefs, Node, Session, TileContent, TileId};
use azulejo_protocol::{
    Command, Direction, LayoutSetKind, LayoutSplit, LayoutZoom, TileClose, TileTarget,
};

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

/// A herdr server in its own `azj-test-` session; stopped and deleted on drop.
struct TempHerdr {
    rt: tokio::runtime::Runtime,
    herdr: Herdr,
    name: String,
    client: Client,
}

impl TempHerdr {
    fn start() -> Self {
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
        let name = format!("azj-test-mirror-{}-{nanos:x}", std::process::id());
        let (client, _) = rt
            .block_on(herdr.ensure_server(&HerdrSession::named(&name), quick()))
            .expect("herdr server");
        let me = Self {
            rt,
            herdr,
            name,
            client,
        };
        me.assert_throwaway();
        me
    }

    /// The mirror moves panes around: never on the user's own session.
    fn assert_throwaway(&self) {
        assert!(self.name.starts_with("azj-test-"), "{}", self.name);
        let socket = self.client.socket_path().expect("a local socket");
        assert!(
            socket.to_string_lossy().contains("/sessions/azj-test-"),
            "not a throwaway session: {}",
            socket.display()
        );
    }

    fn export(&self, tab: &str) -> TabLayout {
        self.rt
            .block_on(self.client.layout_export(&HerdrTab(tab.into())))
            .unwrap()
    }

    fn tabs(&self) -> usize {
        let snap = self.rt.block_on(self.client.snapshot()).unwrap();
        if snap.tabs.len() != 1 {
            eprintln!("herdr tabs: {:#?}\npanes: {:#?}", snap.tabs, snap.panes);
        }
        snap.tabs.len()
    }

    fn terminals(&self) -> Vec<String> {
        let mut t: Vec<String> = self
            .rt
            .block_on(self.client.panes())
            .unwrap()
            .into_iter()
            .map(|p| p.terminal_id.0)
            .collect();
        t.sort();
        t
    }

    fn cli(&self, args: &[&str]) -> String {
        self.rt
            .block_on(self.herdr.cli(&HerdrSession::named(&self.name), args))
            .unwrap_or_else(|e| panic!("herdr {args:?}: {e}"))
    }

    fn restart(&self) {
        let _ = self.rt.block_on(self.client.stop_server());
        std::thread::sleep(Duration::from_millis(800));
        self.rt
            .block_on(
                self.herdr
                    .ensure_server(&HerdrSession::named(&self.name), quick()),
            )
            .expect("herdr back");
    }
}

impl Drop for TempHerdr {
    fn drop(&mut self) {
        let _ = self.rt.block_on(self.client.stop_server());
        // herdr refuses to delete a session until its server has gone.
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(300));
            if self
                .rt
                .block_on(self.herdr.delete_session(&self.name))
                .is_ok()
            {
                break;
            }
        }
    }
}

fn opts(state: &std::path::Path) -> CoreOptions {
    CoreOptions {
        state_path: Some(state.to_owned()),
        api_socket: "/tmp/azj-test.sock".into(),
        log: Arc::new(|line| eprintln!("core: {line}")),
    }
}

/// A core on `herdr` with mirroring on and new tabs of `kind`.
fn core(herdr: &TempHerdr, state: &std::path::Path, kind: LayoutKind) -> Core {
    let core = Core::start_with_herdr(opts(state), herdr.client.clone());
    core.set_layout_prefs(LayoutPrefs {
        default: kind,
        ..LayoutPrefs::default()
    });
    core.set_mirror_layout(true);
    core
}

fn terminals(s: &Session) -> Vec<(TileId, Option<HerdrRef>)> {
    s.tiles()
        .filter_map(|(_, t)| match &t.content {
            TileContent::Terminal { herdr, ended, .. } if !ended => Some((t.id, herdr.clone())),
            _ => None,
        })
        .collect()
}

fn all_bound(s: &Session, n: usize) -> bool {
    let t = terminals(s);
    t.len() == n && t.iter().all(|(_, h)| h.is_some())
}

fn wait<T>(what: &str, timeout: Duration, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_session(core: &Core, what: &str, ok: impl Fn(&Session) -> bool) -> Session {
    wait(what, Duration::from_secs(10), || {
        let s = core.session().unwrap();
        ok(&s).then_some(s)
    })
}

fn pane_of(s: &Session, tile: TileId) -> HerdrRef {
    terminals(s)
        .into_iter()
        .find(|(t, _)| *t == tile)
        .and_then(|(_, h)| h)
        .expect("bound")
}

fn split(core: &Core, direction: Direction) -> TileId {
    let out = core
        .execute(Command::LayoutSplit(LayoutSplit {
            tile: None,
            direction,
            content: None,
        }))
        .unwrap();
    match out.reply {
        azulejo_protocol::Reply::Created(c) => c.tile.unwrap(),
        other => panic!("{other:?}"),
    }
}

fn tab_root(s: &Session) -> &Node {
    &s.windows[0].workspaces[0].tabs[0].root
}

fn tab_layout(s: &Session) -> &azulejo_model::TabLayout {
    &s.windows[0].workspaces[0].tabs[0].layout
}

/// The herdr tree with tile IDs in place of pane IDs.
fn as_tiles(s: &Session, node: &LayoutNode) -> String {
    match node {
        LayoutNode::Pane { pane_id } => {
            let pane = pane_id.as_ref().unwrap().as_str();
            terminals(s)
                .into_iter()
                .find(|(_, h)| h.as_ref().is_some_and(|h| h.pane_id == pane))
                .map(|(t, _)| t.to_string())
                .unwrap_or_else(|| format!("?{pane}"))
        }
        LayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } => format!(
            "({:?} {ratio:.2} {} {})",
            direction,
            as_tiles(s, first),
            as_tiles(s, second)
        ),
    }
}

/// Azulejo's tree in the same notation (terminals only).
fn azulejo_shape(node: &Node) -> String {
    match node {
        Node::Leaf { id } => id.to_string(),
        Node::Split { axis, ratio, a, b } => format!(
            "({:?} {ratio:.2} {} {})",
            match axis {
                Axis::Horizontal => SplitDirection::Right,
                Axis::Vertical => SplitDirection::Down,
            },
            azulejo_shape(a),
            azulejo_shape(b)
        ),
    }
}

/// Waits until herdr's tab holding the first tile shows Azulejo's tree,
/// ratios included, and stays that way past a push's quiet period.
fn wait_mirrored(herdr: &TempHerdr, core: &Core, what: &str) -> (Session, TabLayout) {
    let read = || {
        let s = core.session().unwrap();
        let first = terminals(&s).first()?.1.clone()?;
        let layout = herdr
            .rt
            .block_on(herdr.client.layout_export(&HerdrTab(first.tab_id.clone())))
            .ok()?;
        (as_tiles(&s, &layout.root) == azulejo_shape(tab_root(&s))).then_some((s, layout))
    };
    wait(what, Duration::from_secs(10), || {
        read()?;
        std::thread::sleep(Duration::from_millis(400));
        read()
    })
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn a_split_in_azulejo_is_a_pane_split_in_the_same_herdr_tab() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = core(&herdr, &dir.path().join("state.json"), LayoutKind::Manual);
    let s = wait_session(&core, "first pane", |s| all_bound(s, 1));
    let first = terminals(&s)[0].0;

    let b = split(&core, Direction::Right);
    let s = wait_session(&core, "second pane", |s| all_bound(s, 2));
    let (pa, pb) = (pane_of(&s, first), pane_of(&s, b));
    assert_eq!(pa.tab_id, pb.tab_id, "one herdr tab for the Azulejo tab");
    assert_eq!(herdr.tabs(), 1);
    let layout = herdr.export(&pa.tab_id);
    assert_eq!(
        layout.root,
        LayoutNode::split(
            SplitDirection::Right,
            0.5,
            LayoutNode::pane(pa.pane_id.as_str()),
            LayoutNode::pane(pb.pane_id.as_str())
        ),
        "split right, the new pane second"
    );

    // Down from the second, then left of it: herdr can only add a pane
    // second, so the push swaps it into place.
    let c = split(&core, Direction::Down);
    wait_session(&core, "third pane", |s| all_bound(s, 3));
    core.execute(Command::TileFocus(TileTarget { tile: Some(c) }))
        .unwrap();
    split(&core, Direction::Left);
    wait_session(&core, "fourth pane", |s| all_bound(s, 4));
    let (s, _) = wait_mirrored(&herdr, &core, "four panes in Azulejo's arrangement");
    assert_eq!(herdr.tabs(), 1);
    // The tab's tiles all point at the herdr tab.
    let tabs: std::collections::HashSet<String> = terminals(&s)
        .into_iter()
        .map(|(_, h)| h.unwrap().tab_id)
        .collect();
    assert_eq!(tabs.len(), 1);
    core.shutdown();
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn a_tile_layout_of_four_has_the_same_shape_in_herdr() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = core(&herdr, &dir.path().join("state.json"), LayoutKind::Tile);
    wait_session(&core, "first pane", |s| all_bound(s, 1));
    for n in 2..=4 {
        split(&core, Direction::Right);
        wait_session(&core, "panes", |s| all_bound(s, n));
    }
    let (s, layout) = wait_mirrored(&herdr, &core, "Tile with four");
    assert_eq!(tab_layout(&s).kind, LayoutKind::Tile);
    // Master left, three stacked on the right, evenly.
    let LayoutNode::Split {
        direction: SplitDirection::Right,
        ratio,
        second,
        ..
    } = &layout.root
    else {
        panic!("{:?}", layout.root);
    };
    assert!((ratio - 0.5).abs() < 0.01);
    let LayoutNode::Split {
        direction: SplitDirection::Down,
        ratio: stack,
        ..
    } = second.as_ref()
    else {
        panic!("{second:?}");
    };
    assert!((stack - 1.0 / 3.0).abs() < 0.01, "even stack: {stack}");

    // Rotate (alt+o): master on top. A different shape, rebuilt in herdr.
    let terms_before = herdr.terminals();
    core.execute(Command::LayoutRotate(
        azulejo_protocol::LayoutRotate::default(),
    ))
    .unwrap();
    let (_, layout) = wait_mirrored(&herdr, &core, "Tile turned");
    assert!(matches!(
        layout.root,
        LayoutNode::Split {
            direction: SplitDirection::Down,
            ..
        }
    ));
    assert_eq!(herdr.terminals(), terms_before, "nothing killed");
    assert_eq!(herdr.tabs(), 1, "the staging tab is gone");
    core.shutdown();
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn resizes_and_splits_in_herdr_come_back() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = core(&herdr, &dir.path().join("state.json"), LayoutKind::Manual);
    wait_session(&core, "first pane", |s| all_bound(s, 1));
    let b = split(&core, Direction::Right);
    let (s, _) = wait_mirrored(&herdr, &core, "two panes");
    let pb = pane_of(&s, b);

    // A resize in the TUI (`pane resize`, as its keys and mouse drag do).
    herdr.cli(&[
        "pane",
        "resize",
        "--pane",
        &pb.pane_id,
        "--direction",
        "left",
        "--amount",
        "0.2",
    ]);
    let want = herdr.export(&pb.tab_id);
    let LayoutNode::Split {
        ratio: herdr_ratio, ..
    } = want.root
    else {
        panic!()
    };
    assert!((herdr_ratio - 0.3).abs() < 0.01, "{herdr_ratio}");
    wait_session(
        &core,
        "Azulejo's ratio follows",
        |s| matches!(tab_root(s), Node::Split { ratio, .. } if (ratio - herdr_ratio).abs() < 1e-3),
    );
    // And stays: no push fights it.
    std::thread::sleep(Duration::from_millis(600));
    let LayoutNode::Split { ratio, .. } = herdr.export(&pb.tab_id).root else {
        panic!()
    };
    assert!((ratio - herdr_ratio).abs() < 1e-4, "no echo");

    // A split in the TUI: the tile lands beside its sibling, below it.
    herdr.cli(&[
        "pane",
        "split",
        &pb.pane_id,
        "--direction",
        "down",
        "--no-focus",
    ]);
    let s = wait_session(&core, "the split-off pane is a tile", |s| all_bound(s, 3));
    let Node::Split { b: right, .. } = tab_root(&s) else {
        panic!()
    };
    assert!(
        matches!(right.as_ref(), Node::Split { axis: Axis::Vertical, a, .. } if a.contains(b)),
        "{:?}",
        tab_root(&s)
    );
    wait_mirrored(&herdr, &core, "still the same tree");
    core.shutdown();
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn closing_either_way() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = core(&herdr, &dir.path().join("state.json"), LayoutKind::Manual);
    wait_session(&core, "first pane", |s| all_bound(s, 1));
    let b = split(&core, Direction::Right);
    let c = split(&core, Direction::Down);
    let (s, _) = wait_mirrored(&herdr, &core, "three panes");
    let (pb, pc) = (pane_of(&s, b), pane_of(&s, c));

    // Closed in Azulejo (detach, F14): the pane leaves the mirrored tab for
    // one of its own, still running.
    core.execute(Command::TileClose(TileClose {
        tile: Some(c),
        kill: false,
    }))
    .unwrap();
    wait_mirrored(&herdr, &core, "two panes after the close");
    let panes = herdr.rt.block_on(herdr.client.panes()).unwrap();
    let parked = panes
        .iter()
        .find(|p| p.terminal_id.as_str() == pc.terminal_id)
        .expect("detached, not killed");
    assert_ne!(parked.tab_id.as_str(), pb.tab_id, "out of the mirrored tab");

    // Closed in herdr: the tile goes.
    herdr.cli(&["pane", "close", &pb.pane_id]);
    wait_session(&core, "tile closed with its pane", |s| all_bound(s, 1));
    core.shutdown();
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn monocle_and_zoom_both_ways() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = core(&herdr, &dir.path().join("state.json"), LayoutKind::Tile);
    let s = wait_session(&core, "first pane", |s| all_bound(s, 1));
    let a = terminals(&s)[0].0;
    let b = split(&core, Direction::Right);
    let (s, _) = wait_mirrored(&herdr, &core, "two panes");
    let (pa, pb) = (pane_of(&s, a), pane_of(&s, b));
    let zoomed_on = |pane: &str| {
        let l = herdr.export(&pa.tab_id);
        (l.zoomed && l.focused_pane_id == Some(PaneId(pane.into()))).then_some(())
    };

    // Monocle → herdr zooms the focused tile.
    core.execute(Command::LayoutSet(LayoutSetKind {
        tab: None,
        kind: LayoutKind::Monocle,
    }))
    .unwrap();
    wait("herdr zoomed on b", Duration::from_secs(5), || {
        zoomed_on(&pb.pane_id)
    });
    core.execute(Command::TileFocus(TileTarget { tile: Some(a) }))
        .unwrap();
    wait("herdr zoomed on a", Duration::from_secs(5), || {
        zoomed_on(&pa.pane_id)
    });
    // Unzoomed in herdr → Monocle ends.
    herdr
        .rt
        .block_on(herdr.client.zoom_pane(&PaneId(pa.pane_id.clone()), false))
        .unwrap();
    wait_session(&core, "back to Tile", |s| {
        tab_layout(s).kind == LayoutKind::Tile
    });

    // Zoom in Azulejo → herdr; zoom in herdr → Azulejo.
    core.execute(Command::LayoutZoom(LayoutZoom {
        tile: Some(b),
        zoomed: Some(true),
    }))
    .unwrap();
    wait("herdr zoomed on b", Duration::from_secs(5), || {
        zoomed_on(&pb.pane_id)
    });
    herdr
        .rt
        .block_on(herdr.client.zoom_pane(&PaneId(pb.pane_id.clone()), false))
        .unwrap();
    wait_session(&core, "unzoomed from herdr", |s| {
        s.windows[0].workspaces[0].tabs[0].zoomed.is_none()
    });
    herdr
        .rt
        .block_on(herdr.client.zoom_pane(&PaneId(pa.pane_id.clone()), true))
        .unwrap();
    wait_session(&core, "zoomed from herdr", |s| {
        s.windows[0].workspaces[0].tabs[0].zoomed == Some(a)
    });
    core.shutdown();
}

/// The last `tick N` a pane printed.
fn last_tick(herdr: &TempHerdr, pane: &str) -> u64 {
    let text = herdr
        .rt
        .block_on(
            herdr
                .client
                .read(&PaneId(pane.into()), ReadSource::Recent, Some(20)),
        )
        .unwrap()
        .text;
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("tick ")?.trim().parse().ok())
        .next_back()
        .unwrap_or(0)
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn a_session_with_a_tab_per_tile_migrates_without_losing_a_process() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state.json");

    // The old mapping: mirroring off, a herdr tab per terminal tile.
    let old = Core::start_with_herdr(opts(&state), herdr.client.clone());
    wait_session(&old, "first pane", |s| all_bound(s, 1));
    let b = split(&old, Direction::Right);
    split(&old, Direction::Down);
    let s = wait_session(&old, "three panes", |s| all_bound(s, 3));
    assert_eq!(herdr.tabs(), 3, "one herdr tab per tile before");
    let before = terminals(&s);
    let pb = pane_of(&s, b);
    std::thread::sleep(Duration::from_millis(800));
    herdr.cli(&[
        "pane",
        "run",
        &pb.pane_id,
        "sh -c 'i=0; while true; do i=$((i+1)); echo tick $i; sleep 0.2; done'",
    ]);
    wait("ticking", Duration::from_secs(10), || {
        (last_tick(&herdr, &pb.pane_id) > 2).then_some(())
    });
    let terms = herdr.terminals();
    std::thread::sleep(Duration::from_millis(400));
    old.shutdown();

    // The new build: mirroring on. The first push merges the tab.
    let core = Core::start_with_running_herdr(opts(&state), herdr.client.clone());
    core.set_mirror_layout(true);
    let ticks = last_tick(&herdr, &pb.pane_id);
    let (s, _) = wait_mirrored(&herdr, &core, "merged into one tab");
    assert_eq!(herdr.tabs(), 1, "one herdr tab for the Azulejo tab");
    assert_eq!(herdr.terminals(), terms, "the same terminal IDs");
    let after = terminals(&s);
    assert_eq!(after.len(), 3, "no tile closed");
    for ((t0, h0), (t1, h1)) in before.iter().zip(&after) {
        assert_eq!(t0, t1);
        assert_eq!(
            h0.as_ref().unwrap().terminal_id,
            h1.as_ref().unwrap().terminal_id
        );
    }
    std::thread::sleep(Duration::from_millis(600));
    assert!(
        last_tick(&herdr, &pb.pane_id) > ticks,
        "the ticker kept counting"
    );
    core.shutdown();
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn herdr_restarting_mid_way_keeps_the_mirror() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = core(&herdr, &dir.path().join("state.json"), LayoutKind::Manual);
    wait_session(&core, "first pane", |s| all_bound(s, 1));
    split(&core, Direction::Right);
    let (s, _) = wait_mirrored(&herdr, &core, "two panes");
    let old: Vec<String> = terminals(&s)
        .into_iter()
        .map(|(_, h)| h.unwrap().terminal_id)
        .collect();

    herdr.restart();
    // herdr restores the panes with fresh terminals; the core rebinds.
    wait_session(&core, "rebound after the restart", |s| {
        all_bound(s, 2)
            && terminals(s)
                .iter()
                .all(|(_, h)| !old.contains(&h.as_ref().unwrap().terminal_id))
    });
    wait_mirrored(&herdr, &core, "still mirrored after the restart");
    // And it keeps working: a split lands in the same tab.
    split(&core, Direction::Down);
    wait_session(&core, "third pane", |s| all_bound(s, 3));
    wait_mirrored(&herdr, &core, "three panes after the restart");
    assert_eq!(herdr.tabs(), 1);
    core.shutdown();
}

/// A tab's herdr tree in tile IDs, for a tab on `machine`.
fn remote_shape(s: &Session, machine: &str, node: &LayoutNode) -> String {
    match node {
        LayoutNode::Pane { pane_id } => {
            let pane = pane_id.as_ref().unwrap().as_str();
            terminals(s)
                .into_iter()
                .find(|(_, h)| {
                    h.as_ref()
                        .is_some_and(|h| h.pane_id == pane && h.machine.as_deref() == Some(machine))
                })
                .map(|(t, _)| t.to_string())
                .unwrap_or_else(|| format!("?{pane}"))
        }
        LayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } => format!(
            "({direction:?} {ratio:.2} {} {})",
            remote_shape(s, machine, first),
            remote_shape(s, machine, second)
        ),
    }
}

/// ADR 0011 machines mirror the same way: over herdr's SSH bridge (a
/// stand-in that runs it locally against a second throwaway session).
#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn a_remote_machines_tabs_are_mirrored_there() {
    let local = TempHerdr::start();
    let remote = TempHerdr::start();
    let dir = std::env::temp_dir().join(format!("azj-live-mirror-mm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ssh = dir.join("ssh");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/ssh-standin.sh"),
        &ssh,
    )
    .unwrap();
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
    let core = core(&local, &dir.join("state.json"), LayoutKind::Manual);
    wait_session(&core, "local pane", |s| all_bound(s, 1));
    core.add_machine(id.clone(), "standin".into(), machine.client());

    let reply = core
        .execute(Command::WorkspaceCreate(
            azulejo_protocol::WorkspaceCreate {
                name: Some("made-there".into()),
                machine: Some("standin".into()),
                ..Default::default()
            },
        ))
        .unwrap();
    let azulejo_protocol::Reply::Created(created) = reply.reply else {
        panic!("{reply:?}")
    };
    let ws = created.workspace.unwrap();
    core.execute(Command::TabCreate(azulejo_protocol::TabCreate {
        workspace: Some(ws),
        ..Default::default()
    }))
    .unwrap();
    wait_session(&core, "remote pane", |s| all_bound(s, 2));
    split(&core, Direction::Right);
    split(&core, Direction::Down);
    let remote_tab = |s: &Session| s.workspace(ws).unwrap().tabs[0].clone();
    let (s, layout) = wait("mirrored on the machine", Duration::from_secs(15), || {
        let s = core.session().unwrap();
        let tab = remote_tab(&s);
        let first = tab.tiles.iter().find_map(|t| match &t.content {
            TileContent::Terminal { herdr: Some(h), .. } => Some(h.clone()),
            _ => None,
        })?;
        let layout = remote
            .rt
            .block_on(remote.client.layout_export(&HerdrTab(first.tab_id)))
            .ok()?;
        (tab.tiles.len() == 3 && remote_shape(&s, &id, &layout.root) == azulejo_shape(&tab.root))
            .then_some((s, layout))
    });
    assert!(matches!(
        layout.root,
        LayoutNode::Split {
            direction: SplitDirection::Right,
            ..
        }
    ));
    assert_eq!(remote.tabs(), 1, "one herdr tab on the machine");
    assert_eq!(local.tabs(), 1, "Local untouched");
    let tabs: std::collections::HashSet<String> = remote_tab(&s)
        .tiles
        .iter()
        .filter_map(|t| match &t.content {
            TileContent::Terminal { herdr: Some(h), .. } => Some(h.tab_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(tabs.len(), 1);
    core.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The root split's ratio in a tree.
fn root_ratio(node: &Node) -> Option<f64> {
    match node {
        Node::Split { ratio, .. } => Some(*ratio),
        Node::Leaf { .. } => None,
    }
}

/// A resize in herdr right after a split made in Azulejo, before that
/// split's push: the resize wins, every time.
#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn a_resize_in_herdr_right_after_a_split_is_never_lost() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = core(&herdr, &dir.path().join("state.json"), LayoutKind::Manual);
    let s = wait_session(&core, "first pane", |s| all_bound(s, 1));
    let first = terminals(&s)[0].0;
    let mut lost = Vec::new();
    for i in 0..20 {
        core.execute(Command::TileFocus(TileTarget { tile: Some(first) }))
            .unwrap();
        let new = split(&core, Direction::Right);
        // As soon as herdr has the pane, resize there: well inside the
        // split's 150 ms quiet period.
        let s = wait("pane made", Duration::from_secs(10), || {
            let s = core.session().unwrap();
            all_bound(&s, 2).then_some(s)
        });
        let tab = pane_of(&s, new).tab_id;
        let want = if i % 2 == 0 { 0.3 } else { 0.7 };
        herdr
            .rt
            .block_on(
                herdr
                    .client
                    .set_split_ratio(&HerdrTab(tab.clone()), &[], want),
            )
            .unwrap();
        let followed = (0..100).any(|_| {
            std::thread::sleep(Duration::from_millis(20));
            core.session()
                .ok()
                .and_then(|s| root_ratio(tab_root(&s)))
                .is_some_and(|r| (r - want).abs() < 1e-3)
        });
        // And nothing pushes it back.
        std::thread::sleep(Duration::from_millis(400));
        let kept = matches!(herdr.export(&tab).root, LayoutNode::Split { ratio, .. } if (ratio - want).abs() < 1e-3)
            && root_ratio(tab_root(&core.session().unwrap()))
                .is_some_and(|r| (r - want).abs() < 1e-3);
        if !(followed && kept) {
            lost.push(i);
        }
        core.execute(Command::TileClose(TileClose {
            tile: Some(new),
            kill: true,
        }))
        .unwrap();
        wait_session(&core, "back to one", |s| all_bound(s, 1));
    }
    assert!(lost.is_empty(), "resizes lost in rounds {lost:?}");
    core.shutdown();
}

fn tab_kind(s: &Session) -> LayoutKind {
    tab_layout(s).kind
}

/// Monocle with a browser beside the terminals: focusing the browser keeps
/// Monocle, and herdr keeps showing a terminal. Only an unzoom made in herdr
/// ends Monocle.
#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn monocle_with_a_browser_stays_monocle() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = core(&herdr, &dir.path().join("state.json"), LayoutKind::Tile);
    let s = wait_session(&core, "first pane", |s| all_bound(s, 1));
    let a = terminals(&s)[0].0;
    let browser = core
        .execute(Command::LayoutSplit(LayoutSplit {
            tile: None,
            direction: Direction::Right,
            content: Some(TileContent::Browser {
                url: None,
                page_title: None,
            }),
        }))
        .unwrap();
    let azulejo_protocol::Reply::Created(created) = browser.reply else {
        panic!()
    };
    let b = created.tile.unwrap();
    core.execute(Command::LayoutSet(LayoutSetKind {
        tab: None,
        kind: LayoutKind::Monocle,
    }))
    .unwrap();
    // One terminal and a browser: back and forth, Monocle stays.
    for tile in [a, b, a, b] {
        core.execute(Command::TileFocus(TileTarget { tile: Some(tile) }))
            .unwrap();
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(tab_kind(&core.session().unwrap()), LayoutKind::Monocle);
    }

    // A second terminal: focusing the browser keeps herdr on the terminal
    // focused last, zoomed.
    core.execute(Command::TileFocus(TileTarget { tile: Some(a) }))
        .unwrap();
    let c = split(&core, Direction::Right);
    let s = wait_session(&core, "second pane", |s| all_bound(s, 2));
    let (pa, pc) = (pane_of(&s, a), pane_of(&s, c));
    let mut last_terminal = c;
    for tile in [c, b, a, b] {
        if tile != b {
            last_terminal = tile;
        }
        core.execute(Command::TileFocus(TileTarget { tile: Some(tile) }))
            .unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let s = core.session().unwrap();
        assert_eq!(tab_kind(&s), LayoutKind::Monocle, "after focusing {tile}");
        let l = herdr.export(&pa.tab_id);
        assert!(l.zoomed, "herdr stays zoomed after focusing {tile}");
        let shown = if last_terminal == c {
            &pc.pane_id
        } else {
            &pa.pane_id
        };
        assert_eq!(l.focused_pane_id, Some(PaneId(shown.clone())));
        assert_eq!(s.windows[0].workspaces[0].tabs[0].focused, tile);
    }

    // Unzoomed in herdr, for real: Monocle ends.
    herdr
        .rt
        .block_on(herdr.client.zoom_pane(&PaneId(pa.pane_id.clone()), false))
        .unwrap();
    wait_session(&core, "Monocle ends", |s| {
        tab_kind(s) != LayoutKind::Monocle
    });
    core.shutdown();
}

/// Quick splits into a Tile stack (as restore.sh builds one): herdr's own
/// in-between ratios (each new split at ½) are never taken for a change, so
/// the stack stays even.
#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn quick_splits_into_a_tile_stack_stay_even() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = core(&herdr, &dir.path().join("state.json"), LayoutKind::Tile);
    wait_session(&core, "first pane", |s| all_bound(s, 1));
    for _ in 0..5 {
        core.execute(Command::LayoutSplitAuto(
            azulejo_protocol::LayoutSplitAuto::default(),
        ))
        .unwrap();
    }
    wait_session(&core, "six panes", |s| all_bound(s, 6));
    let (s, _) = wait_mirrored(&herdr, &core, "the stack in herdr");
    std::thread::sleep(Duration::from_millis(800));
    let s2 = core.session().unwrap();
    let layout = tab_layout(&s2);
    assert_eq!(layout.kind, LayoutKind::Tile);
    assert!(layout.weights.is_empty(), "weights {:?}", layout.weights);
    assert_eq!(tab_root(&s), tab_root(&s2));
    core.shutdown();
}

/// Turns the focused tab a quarter turn (`alt+o`) and waits until herdr
/// shows the turned tree and keeps it: pushed, and not pulled back. Returns
/// the tab's tree and herdr's, in tile IDs.
fn turn_and_follow(herdr: &TempHerdr, core: &Core, what: &str) -> (Node, String) {
    let before = tab_root(&core.session().unwrap()).clone();
    core.execute(Command::LayoutRotate(
        azulejo_protocol::LayoutRotate::default(),
    ))
    .unwrap();
    let (s, layout) = wait_mirrored(herdr, core, what);
    let root = tab_root(&s).clone();
    assert_ne!(root, before, "{what}: the tree turned");
    // Still there a while later: no pull undid it, no absorb froze it.
    std::thread::sleep(Duration::from_millis(500));
    let later = core.session().unwrap();
    assert_eq!(tab_root(&later), &root, "{what}: pulled back");
    assert_eq!(
        tab_layout(&later).kind,
        tab_layout(&s).kind,
        "{what}: kind changed"
    );
    assert_eq!(herdr.tabs(), 1, "{what}: the staging tab is gone");
    (root, as_tiles(&s, &layout.root))
}

/// The focused tab and herdr's tree for it, once they agree.
fn settled(herdr: &TempHerdr, core: &Core, what: &str) -> (Node, String) {
    let (s, layout) = wait_mirrored(herdr, core, what);
    (tab_root(&s).clone(), as_tiles(&s, &layout.root))
}

/// A tab adopted from herdr is manual (ADR 0010 §5). `alt+o` turns its
/// tree, a 2×2 grid here, four ways; herdr's own tree follows every turn
/// and ends where it started, and so it does again after a resize made in
/// herdr was pulled in.
#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn rotating_an_adopted_grid_four_times_is_followed_in_herdr() {
    let herdr = TempHerdr::start();
    // The user's own herdr: a 2×2 grid split in the TUI.
    herdr.cli(&["workspace", "create", "--label", "grid"]);
    let panes = || herdr.rt.block_on(herdr.client.panes()).unwrap();
    let p1 = panes()[0].pane_id.0.clone();
    herdr.cli(&["pane", "split", &p1, "--direction", "right"]);
    let p2 = panes()
        .into_iter()
        .find(|p| p.pane_id.0 != p1)
        .unwrap()
        .pane_id
        .0;
    herdr.cli(&["pane", "split", &p1, "--direction", "down"]);
    herdr.cli(&["pane", "split", &p2, "--direction", "down"]);
    assert_eq!(panes().len(), 4);
    let terms = herdr.terminals();

    let dir = tempfile::tempdir().unwrap();
    let core = Core::start_with_herdr(opts(&dir.path().join("state.json")), herdr.client.clone());
    core.set_mirror_layout(true);
    wait_session(&core, "the grid adopted", |s| all_bound(s, 4));
    let start = settled(&herdr, &core, "adopted grid");
    assert_eq!(
        tab_layout(&core.session().unwrap()).kind,
        LayoutKind::Manual,
        "adopted tabs are manual"
    );
    let corner = |root: &Node, tile: TileId| {
        let r = root
            .rects()
            .into_iter()
            .find(|(t, _)| *t == tile)
            .unwrap()
            .1;
        (r.x > 0.25, r.y > 0.25)
    };
    let top_left = start.0.rects()[0].0;
    assert_eq!(corner(&start.0, top_left), (false, false));

    let mut corners = Vec::new();
    let mut last = start.clone();
    for turn in 1..=4 {
        last = turn_and_follow(&herdr, &core, &format!("grid turn {turn}"));
        corners.push(corner(&last.0, top_left));
    }
    // Top left → top right → bottom right → bottom left → home.
    assert_eq!(
        corners,
        [(true, false), (true, true), (false, true), (false, false)]
    );
    assert_eq!(
        last, start,
        "four turns: the same tree in Azulejo and herdr"
    );
    assert_eq!(herdr.terminals(), terms, "nothing killed");

    // A resize in herdr comes in, and four turns still go all the way round.
    let tab = pane_of(&core.session().unwrap(), top_left).tab_id;
    herdr
        .rt
        .block_on(herdr.client.set_split_ratio(&HerdrTab(tab), &[], 0.3))
        .unwrap();
    wait_session(&core, "the resize pulled in", |s| {
        root_ratio(tab_root(s)).is_some_and(|r| (r - 0.3).abs() < 1e-3)
    });
    let start = settled(&herdr, &core, "after the pull");
    let mut last = start.clone();
    for turn in 1..=4 {
        last = turn_and_follow(&herdr, &core, &format!("grid turn {turn} after the pull"));
    }
    assert_eq!(last.1, start.1, "herdr's tree back where it was");
    let mut want = start.0.clone();
    want.snap_ratios();
    assert_eq!(last.0, want, "Azulejo's tree back where it was");
    assert_eq!(herdr.terminals(), terms, "nothing killed");
    core.shutdown();
}

/// Four turns of a Tile tab: master left → top → right → bottom → left,
/// herdr following each one, the tab never freezing to manual.
fn four_tile_turns(herdr: &TempHerdr, core: &Core, round: &str) {
    use azulejo_model::Orientation;
    let start = settled(herdr, core, round);
    let mut last = start.clone();
    let mut seen = Vec::new();
    for turn in 1..=4 {
        last = turn_and_follow(herdr, core, &format!("Tile turn {turn} {round}"));
        let now = core.session().unwrap();
        assert_eq!(tab_layout(&now).kind, LayoutKind::Tile, "not frozen");
        seen.push(tab_layout(&now).params.tile.orientation);
    }
    assert_eq!(
        seen,
        [
            Orientation::Top,
            Orientation::Right,
            Orientation::Bottom,
            Orientation::Left
        ],
        "{round}"
    );
    assert_eq!(last, start, "{round}: four turns, the same tree in both");
}

/// A Tile tab turns four ways with herdr following, and again after a
/// master ratio set in herdr (the TUI's resize) was absorbed.
#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn rotating_a_tile_tab_four_times_is_followed_in_herdr() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let core = core(&herdr, &dir.path().join("state.json"), LayoutKind::Tile);
    wait_session(&core, "first pane", |s| all_bound(s, 1));
    for n in 2..=4 {
        split(&core, Direction::Right);
        wait_session(&core, "panes", |s| all_bound(s, n));
    }
    let terms = herdr.terminals();
    four_tile_turns(&herdr, &core, "before a pull");

    let tab = terminals(&core.session().unwrap())[0]
        .1
        .clone()
        .unwrap()
        .tab_id;
    herdr
        .rt
        .block_on(herdr.client.set_split_ratio(&HerdrTab(tab), &[], 0.65))
        .unwrap();
    let s = wait_session(&core, "the master ratio pulled in", |s| {
        (tab_layout(s).params.tile.ratio - 0.65).abs() < 1e-3
    });
    assert_eq!(
        tab_layout(&s).kind,
        LayoutKind::Tile,
        "absorbed, not frozen"
    );
    four_tile_turns(&herdr, &core, "after a pull");
    assert_eq!(herdr.terminals(), terms, "nothing killed");
    core.shutdown();
}
