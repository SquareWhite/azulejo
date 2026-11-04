//! Integration tests against a real herdr, in a throwaway named session.
//! Ignored by default; run with `just test-herdr`. Never touches the user's
//! default session: every herdr process gets a clean environment and an
//! explicit `--session`.

use std::time::Duration;

use azulejo_herdr::{
    AttachExit, Client, ClientOptions, Herdr, HerdrEvent, ReadSource, Resolved, Session,
    StreamItem, Subscription, TerminalSpec, TileEnv, WorkspaceSpec,
};

/// Stops and deletes the session even if the test panics.
struct TempSession {
    herdr: Herdr,
    name: String,
    socket: std::path::PathBuf,
}

impl TempSession {
    async fn new() -> Self {
        let herdr = Herdr::locate().expect("herdr on PATH");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let name = format!("azj-test-{}-{nanos:x}", std::process::id());
        let socket = herdr.socket_path(&Session::named(&name)).await.unwrap();
        Self {
            herdr,
            name,
            socket,
        }
    }

    fn session(&self) -> Session {
        Session::named(&self.name)
    }
}

impl Drop for TempSession {
    fn drop(&mut self) {
        let socket = self.socket.clone();
        let name = self.name.clone();
        let bin = self.herdr.path().to_owned();
        // Separate thread: Drop can run inside the runtime.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let _ = Client::new(&socket, quick()).stop_server().await;
                tokio::time::sleep(Duration::from_millis(500)).await;
                let _ = Herdr::with_path(bin).delete_session(&name).await;
            });
        })
        .join()
        .unwrap();
    }
}

fn quick() -> ClientOptions {
    ClientOptions {
        request_timeout: Duration::from_secs(5),
        connect: azulejo_herdr::Backoff {
            initial: Duration::from_millis(50),
            max: Duration::from_millis(500),
            give_up_after: Duration::from_secs(10),
        },
    }
}

async fn next_event(
    events: &mut azulejo_herdr::EventStream,
    want: impl Fn(&StreamItem) -> bool,
) -> StreamItem {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let item = events.recv().await.expect("stream ended");
            if want(&item) {
                return item;
            }
        }
    })
    .await
    .expect("timed out waiting for event")
}

async fn read_until(client: &Client, pane: &azulejo_herdr::PaneId, needle: &str) -> String {
    for _ in 0..50 {
        let text = client
            .read(pane, ReadSource::Recent, Some(50))
            .await
            .unwrap()
            .text;
        if text.contains(needle) {
            return text;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("`{needle}` never appeared in the pane");
}

#[tokio::test]
#[ignore = "needs herdr; run with `just test-herdr`"]
async fn full_lifecycle_against_real_herdr() {
    let tmp = TempSession::new().await;
    let session = tmp.session();

    // Start the server (clean env) and check the version.
    let (client, info) = tmp.herdr.ensure_server(&session, quick()).await.unwrap();
    assert!(info.protocol >= azulejo_herdr::MIN_PROTOCOL);
    // A second ensure finds the running server.
    let (_, again) = tmp.herdr.ensure_server(&session, quick()).await.unwrap();
    assert_eq!(again, info);

    let mut events = client.subscribe(Subscription::lifecycle());
    assert_eq!(
        next_event(&mut events, |_| true).await,
        StreamItem::Connected
    );

    // Workspace 1:1. Creating returns herdr's built-in first terminal.
    let ensured = client
        .ensure_workspace(
            None,
            &WorkspaceSpec {
                label: Some("azj-ws".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let ws = ensured.workspace.workspace_id.clone();
    assert!(ensured.created_terminal.is_some());
    let reused = client
        .ensure_workspace(Some(&ws), &WorkspaceSpec::default())
        .await
        .unwrap();
    assert_eq!(reused.workspace.workspace_id, ws);
    assert!(reused.created_terminal.is_none());

    // A terminal tile = a labelled tab with the tile's env vars.
    let env = TileEnv {
        socket: "/tmp/azj.sock".into(),
        tile_id: "tile-42".into(),
        workspace_id: "app-ws-1".into(),
    };
    let term = client
        .create_terminal(&TerminalSpec {
            workspace_id: ws.clone(),
            label: Some("agent".into()),
            cwd: None,
            env: env.to_pairs(),
        })
        .await
        .unwrap();
    assert!(term.terminal_id.as_str().starts_with("term_"));
    next_event(
        &mut events,
        |i| matches!(i, StreamItem::Event(HerdrEvent::PaneCreated(p)) if p.pane_id == term.pane_id),
    )
    .await;

    // The env reaches the shell, and the server didn't leak ours. `env`
    // output works in any shell (the user's may be fish).
    client
        .send_text(
            &term.pane_id,
            "env | sort | grep -E '^(AZULEJO|CLAUDE)'; echo done-$AZULEJO_WORKSPACE_ID",
        )
        .await
        .unwrap();
    client.send_keys(&term.pane_id, &["Enter"]).await.unwrap();
    let text = read_until(&client, &term.pane_id, "done-app-ws-1").await;
    assert!(text.contains("\nAZULEJO_TILE_ID=tile-42"), "{text}");
    assert!(
        !text.contains("\nCLAUDE"),
        "inherited CLAUDE* vars leaked: {text}"
    );

    // Lookups.
    let found = client
        .resolve_terminal(&term.terminal_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.pane_id, term.pane_id);
    assert!(client.pane(&"w99:p99".into()).await.unwrap().is_none());
    let snap = client.snapshot().await.unwrap();
    assert!(snap.panes.iter().any(|p| p.terminal_id == term.terminal_id));
    client.scroll(&term.pane_id, 0).await.unwrap();

    // Attach exit diagnosis for a live terminal vs a gone one.
    assert_eq!(
        client
            .diagnose_attach_exit(Some(1), None, &term.terminal_id)
            .await,
        AttachExit::TakenOver
    );
    assert_eq!(
        client
            .diagnose_attach_exit(Some(1), None, &"term_nope".into())
            .await,
        AttachExit::TerminalGone
    );

    // Kill = close the pane; its tab goes with it.
    client.kill_terminal(&term.pane_id).await.unwrap();
    next_event(&mut events, |i| {
        matches!(i, StreamItem::Event(HerdrEvent::PaneClosed { pane_id, .. }) if *pane_id == term.pane_id)
            || matches!(i, StreamItem::Event(HerdrEvent::TabClosed { tab_id, .. }) if *tab_id == term.tab_id)
    })
    .await;
    assert!(client.pane(&term.pane_id).await.unwrap().is_none());

    // A second terminal survives into the restart below.
    let keep = client
        .create_terminal(&TerminalSpec {
            workspace_id: ws.clone(),
            label: Some("keep".into()),
            cwd: None,
            env: env.to_pairs(),
        })
        .await
        .unwrap();
    assert!(matches!(
        client.reresolve(&keep).await.unwrap(),
        Resolved::Same(_)
    ));
    assert_eq!(client.reresolve(&term).await.unwrap(), Resolved::Gone);
    // The same answers, in order, from one read of herdr's panes.
    let many = client
        .reresolve_many(&[keep.clone(), term.clone()])
        .await
        .unwrap();
    assert!(matches!(many[0], Resolved::Same(_)), "{many:?}");
    assert_eq!(many[1], Resolved::Gone);

    // Restart the server mid-test: requests and the event stream recover.
    client.stop_server().await.unwrap();
    next_event(&mut events, |i| matches!(i, StreamItem::Disconnected(_))).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let restart = {
        let herdr = tmp.herdr.clone();
        let session = session.clone();
        tokio::spawn(async move { herdr.ensure_server(&session, quick()).await })
    };
    // The old client rides out the restart through its connect backoff.
    let after = client.ping().await.unwrap();
    assert_eq!(after.protocol, info.protocol);
    restart.await.unwrap().unwrap();
    assert_eq!(
        next_event(&mut events, |i| matches!(i, StreamItem::Resync)).await,
        StreamItem::Resync
    );

    // herdr restores workspaces, tabs and pane IDs on restart, but with
    // fresh processes and new terminal IDs.
    let ensured = client
        .ensure_workspace(Some(&ws), &WorkspaceSpec::default())
        .await
        .unwrap();
    assert_eq!(ensured.workspace.workspace_id, ws);
    assert!(ensured.created_terminal.is_none());
    let Resolved::Respawned(p) = client.reresolve(&keep).await.unwrap() else {
        panic!("expected the kept terminal to come back respawned");
    };
    assert_eq!(p.pane_id, keep.pane_id);
    assert_ne!(p.terminal_id, keep.terminal_id);
    assert_eq!(
        client
            .reresolve_many(std::slice::from_ref(&keep))
            .await
            .unwrap(),
        [Resolved::Respawned(p)]
    );

    // Events flow on the recovered stream.
    client
        .create_terminal(&TerminalSpec {
            workspace_id: ws.clone(),
            label: None,
            cwd: None,
            env: vec![],
        })
        .await
        .unwrap();
    next_event(&mut events, |i| {
        matches!(i, StreamItem::Event(HerdrEvent::PaneCreated(_)))
    })
    .await;
}

#[tokio::test]
#[ignore = "needs herdr; run with `just test-herdr`"]
async fn attach_command_runs_and_reports_takeover() {
    let tmp = TempSession::new().await;
    let session = tmp.session();
    let (client, _) = tmp.herdr.ensure_server(&session, quick()).await.unwrap();
    let (_, term) = client
        .create_workspace(&WorkspaceSpec::default())
        .await
        .unwrap();

    // herdr attach needs a terminal with a size, so run it under `script`,
    // which gives it a PTY. stderr goes to a file via with_stderr_file.
    let errs = tempfile::Builder::new()
        .prefix("azj-attach-")
        .tempdir()
        .unwrap();
    let dir = errs.path();
    let spawn = |takeover: bool, name: &str| {
        let err = dir.join(format!("{name}.err"));
        let _ = std::fs::remove_file(&err);
        let cmd = tmp
            .herdr
            .attach(&session, &term.terminal_id, takeover)
            .with_stderr_file(&err);
        // `script`'s PTY starts at 0x0, and attach refuses a zero-sized PTY.
        let mut argv = vec![
            "-q".to_owned(),
            "/dev/null".to_owned(),
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            r#"stty rows 30 cols 100; exec "$0" "$@""#.to_owned(),
        ];
        argv.extend(cmd.argv());
        let child = tokio::process::Command::new("script")
            .args(&argv)
            .env_clear()
            .envs(cmd.env.clone())
            .env("TERM", "xterm-256color")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        (child, err)
    };

    let (mut first, first_err) = spawn(false, "first");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        first.try_wait().unwrap().is_none(),
        "first attach should be running"
    );

    let (mut second, _) = spawn(true, "second");
    let status = tokio::time::timeout(Duration::from_secs(5), first.wait())
        .await
        .expect("first attach should exit after takeover")
        .unwrap();
    let stderr = std::fs::read_to_string(&first_err).unwrap_or_default();
    let exit = client
        .diagnose_attach_exit(status.code(), Some(&stderr), &term.terminal_id)
        .await;
    assert_eq!(
        exit,
        AttachExit::TakenOver,
        "status {status:?}, stderr {stderr:?}"
    );
    let _ = second.kill().await;
}

/// The ssh stand-in (scripts/ssh-standin.sh), copied so a test can cut its
/// "link" by creating `<copy>.fail`.
fn ssh_standin(tag: &str) -> std::path::PathBuf {
    let src =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/ssh-standin.sh");
    let dir = std::env::temp_dir().join(format!("azj-standin-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dst = dir.join("ssh");
    std::fs::copy(&src, &dst).unwrap();
    dst
}

#[tokio::test]
#[ignore = "needs herdr; run with `just test-herdr`"]
async fn a_remote_machine_through_herdrs_bridge() {
    // A "remote" machine: a throwaway local session reached only through
    // the SSH bridge path (ssh stand-in → remote-api-bridge), ADR 0011.
    let tmp = TempSession::new().await;
    tmp.herdr
        .ensure_server(&tmp.session(), quick())
        .await
        .unwrap();
    let ssh = ssh_standin("bridge");
    let machine = azulejo_herdr::RemoteMachine::new(
        azulejo_herdr::MachineProfile {
            id: "0123456789abcdef0123456789abcdef".into(),
            label: "standin".into(),
            target: "standin-host".into(),
            session: tmp.name.clone(),
            enabled: true,
        },
        azulejo_herdr::Ssh {
            program: ssh.clone(),
            control_dir: std::env::temp_dir(),
        },
    );
    let client = machine.client();
    assert!(client.socket_path().is_none());
    let info = client.ping().await.unwrap();
    assert!(info.protocol >= azulejo_herdr::MIN_PROTOCOL);

    // A subscription over the bridge sees a workspace made over the bridge.
    let mut events = client.subscribe(Subscription::lifecycle());
    assert_eq!(
        next_event(&mut events, |_| true).await,
        StreamItem::Connected
    );
    let t0 = std::time::Instant::now();
    let ensured = client
        .ensure_workspace(
            None,
            &WorkspaceSpec {
                label: Some("over-ssh".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let ws = ensured.workspace.workspace_id.clone();
    next_event(
        &mut events,
        |i| matches!(i, StreamItem::Event(HerdrEvent::WorkspaceCreated(w)) if w.workspace_id == ws),
    )
    .await;
    eprintln!("remote request + event: {:?}", t0.elapsed());
    let survey = client.survey().await.unwrap();
    assert!(
        survey
            .workspaces
            .iter()
            .any(|w| w.label.as_deref() == Some("over-ssh"))
    );

    // Cut the link: requests fail as unreachable with ssh's reason; a new
    // stream reports it and comes back when the link does.
    drop(events);
    let fail = std::path::PathBuf::from(format!("{}.fail", ssh.display()));
    std::fs::write(&fail, "").unwrap();
    let err = client.ping().await.unwrap_err();
    assert!(err.is_unreachable(), "{err}");
    assert!(err.to_string().contains("Connection refused"), "{err}");
    let mut events = client.subscribe(Subscription::lifecycle());
    let first = next_event(&mut events, |_| true).await;
    assert!(
        matches!(&first, StreamItem::Disconnected(why) if why.contains("Connection refused")),
        "{first:?}"
    );
    std::fs::remove_file(&fail).unwrap();
    let back = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let StreamItem::Connected | StreamItem::Resync = events.recv().await.unwrap() {
                break;
            }
        }
    })
    .await;
    assert!(back.is_ok(), "stream never came back");

    // A tile on that machine attaches through `ssh -tt`.
    let term = ensured.created_terminal.unwrap().terminal_id;
    let attach = machine.attach(&term, false);
    assert_eq!(attach.program, ssh);
    assert!(attach.args.iter().any(|a| a == "-tt"));
    let _ = std::fs::remove_dir_all(ssh.parent().unwrap());
}

/// The last `tick N` a pane printed.
async fn last_tick(client: &Client, pane: &azulejo_herdr::PaneId) -> u64 {
    let text = client
        .read(pane, ReadSource::Recent, Some(20))
        .await
        .unwrap()
        .text;
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("tick ")?.trim().parse().ok())
        .next_back()
        .unwrap_or(0)
}

/// ADR 0012: one herdr tab per tile becomes one tab with Azulejo's splits,
/// pane by pane. Nothing is killed: the terminals keep their IDs, a ticker
/// keeps counting and an attached client stays attached. Then a rebuild to
/// another shape, and zoom.
#[tokio::test]
#[ignore = "needs herdr; run with `just test-herdr`"]
async fn mirror_merges_tabs_and_rebuilds_without_killing_anything() {
    use azulejo_herdr::{LayoutNode, MirrorTarget, PaneId, SplitDirection, TabId};
    let tmp = TempSession::new().await;
    assert!(tmp.name.starts_with("azj-test-"), "throwaway session only");
    assert!(
        tmp.socket.to_string_lossy().contains("azj-test-"),
        "never the default session: {}",
        tmp.socket.display()
    );
    let session = tmp.session();
    let (client, _) = tmp.herdr.ensure_server(&session, quick()).await.unwrap();
    let (ws, first) = client
        .create_workspace(&WorkspaceSpec::default())
        .await
        .unwrap();
    let mut refs = vec![first];
    for label in ["two", "three"] {
        refs.push(
            client
                .create_terminal(&TerminalSpec {
                    workspace_id: ws.workspace_id.clone(),
                    label: Some(label.into()),
                    cwd: None,
                    env: Vec::new(),
                })
                .await
                .unwrap(),
        );
    }
    let tabs: Vec<TabId> = refs.iter().map(|r| r.tab_id.clone()).collect();
    assert_eq!(
        tabs.iter().collect::<std::collections::HashSet<_>>().len(),
        3,
        "one tab per terminal to start with"
    );
    // A ticker in the second terminal.
    tokio::time::sleep(Duration::from_millis(800)).await;
    client
        .send_text(
            &refs[1].pane_id,
            "sh -c 'i=0; while true; do i=$((i+1)); echo tick $i; sleep 0.2; done'\n",
        )
        .await
        .unwrap();
    read_until(&client, &refs[1].pane_id, "tick 3").await;

    // An attached client on the third, under `script` for a PTY.
    let cmd = tmp.herdr.attach(&session, &refs[2].terminal_id, false);
    let mut argv = vec![
        "-q".to_owned(),
        "/dev/null".to_owned(),
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        r#"stty rows 30 cols 100; exec "$0" "$@""#.to_owned(),
    ];
    argv.extend(cmd.argv());
    let mut attached = tokio::process::Command::new("script")
        .args(&argv)
        .env_clear()
        .envs(cmd.env.clone())
        .env("TERM", "xterm-256color")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert!(attached.try_wait().unwrap().is_none(), "attach is running");

    let mut events = client.subscribe(vec![Subscription::LayoutUpdated]);
    next_event(&mut events, |i| matches!(i, StreamItem::Connected)).await;

    let pane = |i: usize| LayoutNode::pane(refs[i].pane_id.as_str());
    let target = MirrorTarget {
        tab: Some(tabs[0].clone()),
        root: LayoutNode::split(
            SplitDirection::Right,
            0.5,
            pane(0),
            LayoutNode::split(SplitDirection::Down, 0.4, pane(1), pane(2)),
        ),
        zoomed: None,
        label: None,
        known: Vec::new(),
        park_unknown: false,
    };
    let ticks = last_tick(&client, &refs[1].pane_id).await;
    let out = client.mirror(&target).await.unwrap();
    assert!(azulejo_herdr::satisfies(&out.layout, &target), "{out:?}");
    // herdr says so on the stream, typed.
    let item = next_event(&mut events, |i| {
        matches!(i, StreamItem::Event(HerdrEvent::LayoutUpdated { .. }))
    })
    .await;
    assert!(
        matches!(&item, StreamItem::Event(HerdrEvent::LayoutUpdated { tab_id, .. }) if *tab_id == tabs[0]),
        "{item:?}"
    );

    let snap = client.snapshot().await.unwrap();
    assert_eq!(snap.tabs.len(), 1, "the emptied tabs are gone");
    let mut before: Vec<&str> = refs.iter().map(|r| r.terminal_id.as_str()).collect();
    let mut after: Vec<&str> = snap.panes.iter().map(|p| p.terminal_id.as_str()).collect();
    before.sort_unstable();
    after.sort_unstable();
    assert_eq!(before, after, "the same terminals");
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(
        last_tick(&client, &refs[1].pane_id).await > ticks,
        "the ticker kept counting"
    );
    assert!(
        attached.try_wait().unwrap().is_none(),
        "still attached after the move"
    );

    // Another shape (a Tile turned master-top): rebuilt through a staging
    // tab, still nothing killed; zoom last.
    let turned = MirrorTarget {
        root: LayoutNode::split(
            SplitDirection::Down,
            0.5,
            pane(0),
            LayoutNode::split(SplitDirection::Right, 0.5, pane(1), pane(2)),
        ),
        zoomed: Some(PaneId(refs[2].pane_id.0.clone())),
        ..target.clone()
    };
    let out = client.mirror(&turned).await.unwrap();
    assert!(azulejo_herdr::satisfies(&out.layout, &turned), "{out:?}");
    assert!(out.layout.zoomed);
    let snap = client.snapshot().await.unwrap();
    assert_eq!(snap.tabs.len(), 1, "the staging tab is gone");
    assert_eq!(snap.panes.len(), 3);
    let ticks = last_tick(&client, &refs[1].pane_id).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(last_tick(&client, &refs[1].pane_id).await > ticks);
    assert!(
        attached.try_wait().unwrap().is_none(),
        "still attached after the rebuild"
    );
    // Pushing it again writes nothing.
    assert_eq!(client.mirror(&turned).await.unwrap().writes, 0);
    let _ = attached.kill().await;
}
