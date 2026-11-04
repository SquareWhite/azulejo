//! End to end, headless: `azulejod` plus `azj` against a real herdr in a
//! throwaway named session. Ignored by default; run with `just e2e`, which
//! builds `azj` first. Never touches the user's default herdr session.
//!
//! Builds a tall layout with terminal and browser tiles through `azj`, kills
//! the daemon with SIGKILL, restarts it, and checks the session comes back
//! identical with every terminal tile on the same herdr pane.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use azulejo_herdr::{Client, ClientOptions, Herdr, Session as HerdrSession};
use serde_json::Value;

/// `azj`, built next to this test's binary by `just e2e`.
fn azj_bin() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let bin = exe.parent().and_then(Path::parent).unwrap().join("azj");
    assert!(
        bin.exists(),
        "{} is missing: run `just e2e` (or `cargo build -p azulejo-cli`) first",
        bin.display()
    );
    bin
}

/// The environment for our processes: the caller's, minus anything that would
/// point at another herdr session or Azulejo instance.
fn clean(cmd: &mut Command) -> &mut Command {
    for (key, _) in std::env::vars_os() {
        let k = key.to_string_lossy();
        if k.starts_with("HERDR_") || k.starts_with("AZULEJO_") {
            cmd.env_remove(&key);
        }
    }
    cmd
}

struct World {
    dir: PathBuf,
    socket: PathBuf,
    state: PathBuf,
    session: String,
    daemon: Option<Child>,
    rt: tokio::runtime::Runtime,
}

impl World {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let tag = format!("{}-{nanos:x}", std::process::id());
        // Short path: Unix socket paths are limited to ~100 bytes.
        let dir = PathBuf::from(format!("/tmp/azj-e2e-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        World {
            socket: dir.join("azulejo.sock"),
            state: dir.join("state.json"),
            session: format!("azj-e2e-{tag}"),
            dir,
            daemon: None,
            rt: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        }
    }

    fn start_daemon(&mut self) {
        let child = clean(&mut Command::new(env!("CARGO_BIN_EXE_azulejod")))
            .args(["--state".as_ref(), self.state.as_os_str()])
            .args(["--socket".as_ref(), self.socket.as_os_str()])
            .args(["--no-config", "--herdr-session", &self.session])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        self.daemon = Some(child);
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let out = self.azj_raw(&["ping"]);
            if out.status.success() {
                return;
            }
            if let Some(status) = self.daemon.as_mut().unwrap().try_wait().unwrap() {
                panic!("azulejod exited early: {status}");
            }
            assert!(Instant::now() < deadline, "azulejod never answered");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn signal_daemon(&mut self, signal: &str) -> std::process::ExitStatus {
        let mut child = self.daemon.take().expect("daemon running");
        let ok = Command::new("kill")
            .args([signal, &child.id().to_string()])
            .status()
            .unwrap()
            .success();
        assert!(ok, "kill {signal} failed");
        child.wait().unwrap()
    }

    fn azj_raw(&self, args: &[&str]) -> Output {
        clean(&mut Command::new(azj_bin()))
            .env("AZULEJO_SOCKET", &self.socket)
            .args(args)
            .output()
            .unwrap()
    }

    fn azj(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let out = self.azj_raw(&all);
        // With --json the error object is on stdout.
        assert!(
            out.status.success(),
            "azj {args:?} failed: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    fn session(&self) -> Value {
        self.azj(&["session", "get"])
    }

    fn herdr(&self) -> Client {
        let herdr = Herdr::locate().unwrap();
        let session = HerdrSession::named(&self.session);
        let socket = self.rt.block_on(herdr.socket_path(&session)).unwrap();
        Client::new(socket, ClientOptions::default())
    }

    fn herdr_panes(&self) -> Vec<String> {
        let mut panes: Vec<String> = self
            .rt
            .block_on(self.herdr().panes())
            .unwrap()
            .into_iter()
            .map(|p| p.pane_id.0)
            .collect();
        panes.sort();
        panes
    }
}

impl Drop for World {
    fn drop(&mut self) {
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = self.rt.block_on(self.herdr().stop_server());
        std::thread::sleep(Duration::from_millis(500));
        if let Ok(herdr) = Herdr::locate() {
            let _ = self.rt.block_on(herdr.delete_session(&self.session));
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Every terminal tile: (tile id, herdr pane id if bound, ended).
fn terminals(session: &Value) -> Vec<(String, Option<String>, bool)> {
    let mut out = Vec::new();
    for window in session["session"]["windows"].as_array().unwrap() {
        for ws in window["workspaces"].as_array().unwrap() {
            for tab in ws["tabs"].as_array().unwrap() {
                for tile in tab["tiles"].as_array().unwrap() {
                    if tile["kind"] == "terminal" {
                        out.push((
                            tile["id"].as_str().unwrap().to_owned(),
                            tile["herdr"]["pane_id"].as_str().map(str::to_owned),
                            tile["ended"].as_bool().unwrap_or(false),
                        ));
                    }
                }
            }
        }
    }
    out
}

/// `session.get` without what herdr reports live (pane titles and commands,
/// headers, git branches): what a restart must bring back exactly.
fn structural(mut session: Value) -> Value {
    fn strip(v: &mut Value) {
        match v {
            Value::Object(map) => {
                for key in ["pane", "header", "git"] {
                    map.remove(key);
                }
                map.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut session);
    session
}

fn wait_bound(world: &World, expected: usize) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let session = world.session();
        let t = terminals(&session);
        if t.len() == expected && t.iter().all(|(_, pane, ended)| pane.is_some() && !ended) {
            return session;
        }
        assert!(
            Instant::now() < deadline,
            "terminal tiles never all bound: {t:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "needs herdr; run with `just e2e`"]
fn layout_and_panes_survive_a_hard_kill() {
    let mut world = World::new();
    world.start_daemon();

    // A fresh session has one terminal tile. Build a tall layout in a new
    // workspace next to it.
    world.azj(&["workspace", "create", "--name", "e2e"]);
    world.azj(&["tab", "create"]);
    for _ in 0..3 {
        world.azj(&["layout", "split-auto"]);
    }
    world.azj(&["layout", "promote"]);
    world.azj(&["layout", "rotate"]);
    world.azj(&[
        "tile",
        "split",
        "--right",
        "--kind",
        "browser",
        "--url",
        "http://localhost:3000",
    ]);

    let before = wait_bound(&world, 5);
    let panes_before = world.herdr_panes();
    let mut ours: Vec<String> = terminals(&before)
        .into_iter()
        .filter_map(|(_, pane, _)| pane)
        .collect();
    ours.sort();
    assert_eq!(panes_before, ours, "one herdr pane per terminal tile");
    let tiles = before["session"]["windows"][0]["workspaces"][1]["tabs"][0]["tiles"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(
        tiles, 5,
        "4 tall terminals + 1 browser in the e2e workspace"
    );

    // Let the debounced save land, then pull the plug.
    std::thread::sleep(Duration::from_millis(750));
    let status = world.signal_daemon("-KILL");
    assert!(!status.success());
    assert!(world.socket.exists(), "a killed daemon leaves its socket");

    world.start_daemon();
    let after = wait_bound(&world, 5);
    assert_eq!(
        structural(after),
        structural(before),
        "the session comes back identical"
    );
    assert_eq!(world.herdr_panes(), panes_before, "same panes, none added");

    let version = world.azj(&["version"]);
    assert_eq!(version["herdr_session"], world.session.as_str());
    assert!(version["herdr_socket"].is_string());

    // A clean stop removes the socket.
    let status = world.signal_daemon("-TERM");
    assert!(status.success(), "azulejod exits cleanly on SIGTERM");
    assert!(!world.socket.exists(), "a clean stop removes the socket");
}
