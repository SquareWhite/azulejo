//! The `azj` binary against a real socket and an in-process engine.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use azulejo_api::{Client, Server, ServerHandle, WithInfo, spawn_engine};
use azulejo_engine::Engine;
use serde_json::{Value, json};

const HERDR_SOCKET: &str = "/tmp/azj-cli-test-herdr.sock";

struct App {
    _server: ServerHandle,
    dir: PathBuf,
    socket: PathBuf,
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn start() -> App {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(format!(
        "/tmp/azj-cli-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let socket = dir.join("azulejo.sock");
    let (tx, _engine) = spawn_engine(Engine::new(), |_| {});
    // Pretend the core's herdr session lives at HERDR_SOCKET.
    let info = serde_json::Map::from_iter([("herdr_socket".to_owned(), json!(HERDR_SOCKET))]);
    let server = Server::bind(&socket)
        .unwrap()
        .spawn(Arc::new(WithInfo::new(tx, info)))
        .unwrap();
    App {
        _server: server,
        dir,
        socket,
    }
}

/// Runs `azj` with a clean environment plus `env`.
fn azj(socket: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_azj"));
    cmd.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("AZULEJO_SOCKET", socket)
        .args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn json_of(o: &Output) -> Value {
    assert!(o.status.success(), "azj failed: {}", stderr(o));
    serde_json::from_str(&stdout(o)).unwrap()
}

#[test]
fn browser_split_example() {
    let app = start();
    let out = azj(
        &app.socket,
        &[],
        &[
            "tile",
            "split",
            "--right",
            "--kind",
            "browser",
            "--url",
            "http://localhost:3000",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let line = stdout(&out);
    let tile = line
        .trim()
        .strip_prefix("tile ")
        .expect("prints the new tile")
        .to_owned();

    let info = json_of(&azj(&app.socket, &[], &["--json", "tile", "get", &tile]));
    assert_eq!(info["kind"], "browser");
    assert_eq!(info["url"], "http://localhost:3000");

    let list = stdout(&azj(&app.socket, &[], &["tile", "list"]));
    assert!(list.contains("browser   http://localhost:3000"), "{list}");
    assert_eq!(list.lines().count(), 2);
}

#[test]
fn json_output_is_the_raw_result() {
    let app = start();
    let pong = json_of(&azj(&app.socket, &[], &["ping", "--json"]));
    assert_eq!(pong, json!({"type": "pong", "api_version": 1}));
    let version = json_of(&azj(&app.socket, &[], &["--json", "version"]));
    assert_eq!(version["api_version"], 1);
    let layout = json_of(&azj(&app.socket, &[], &["layout", "get", "--json"]));
    assert_eq!(layout["type"], "layout");
}

#[test]
fn not_running_is_a_clear_error() {
    let out = azj(
        Path::new("/tmp/azj-nothing-here/azulejo.sock"),
        &[],
        &["ping"],
    );
    assert_eq!(out.status.code(), Some(3));
    let err = stderr(&out);
    assert!(err.contains("Azulejo isn't running"), "{err}");
    assert!(err.contains("AZULEJO_SOCKET"), "{err}");
}

#[test]
fn usage_and_api_errors_have_their_own_codes() {
    let app = start();
    let out = azj(&app.socket, &[], &["layout", "split"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("--direction"));

    let out = azj(
        &app.socket,
        &[],
        &["tile", "get", "tile_01JBX3K6Z8V9W0Q1R2S3T4V5W6"],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("not_found"), "{}", stderr(&out));

    let out = azj(&app.socket, &[], &["nope", "thing"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn acts_on_the_calling_tile() {
    let app = start();
    let mut client = Client::connect(&app.socket).unwrap();
    let first = client.call("tile.list", None).unwrap()["tiles"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // A second tile takes focus; the first is the "calling" terminal.
    let second = client
        .call("layout.split", Some(json!({"direction": "right"})))
        .unwrap()["tile"]
        .as_str()
        .unwrap()
        .to_owned();

    let out = azj(
        &app.socket,
        &[("AZULEJO_TILE_ID", &first)],
        &["--json", "tile", "split", "--down"],
    );
    let new = json_of(&out)["tile"].as_str().unwrap().to_owned();
    let layout = client.call("layout.get", None).unwrap();
    // The caller was split, not the focused tile: right side is still `second`.
    let tree = &layout["tree"];
    assert_eq!(tree["b"]["id"], second);
    assert_eq!(tree["a"]["a"]["id"], first);
    assert_eq!(tree["a"]["b"]["id"], new);

    // --no-caller acts on the focused tile instead.
    let out = azj(
        &app.socket,
        &[("AZULEJO_TILE_ID", &first)],
        &["--json", "--no-caller", "tile", "get"],
    );
    assert_eq!(json_of(&out)["id"], new);

    // A stale caller gets a hint.
    let out = azj(
        &app.socket,
        &[("AZULEJO_TILE_ID", "tile_01JBX3K6Z8V9W0Q1R2S3T4V5W6")],
        &["tile", "get"],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("--no-caller"), "{}", stderr(&out));
}

#[test]
fn falls_back_to_the_herdr_pane() {
    let app = start();
    let mut client = Client::connect(&app.socket).unwrap();
    let first = client.call("tile.list", None).unwrap()["tiles"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    client
        .call(
            "terminal.bind",
            Some(json!({"tile": first, "herdr": {"pane_id": "w7:p3", "terminal_id": "term_9", "tab_id": "w7:t3"}})),
        )
        .unwrap();
    client
        .call("layout.split", Some(json!({"direction": "right"})))
        .unwrap();

    let out = azj(
        &app.socket,
        &[("HERDR_PANE_ID", "w7:p3")],
        &["--json", "tile", "get"],
    );
    assert_eq!(json_of(&out)["id"], first);

    // An unknown pane falls back to the focused tile.
    let out = azj(
        &app.socket,
        &[("HERDR_PANE_ID", "w1:p1")],
        &["--json", "tile", "get"],
    );
    assert_ne!(json_of(&out)["id"], first);

    // The same pane ID in the core's herdr session is ours...
    let out = azj(
        &app.socket,
        &[
            ("HERDR_PANE_ID", "w7:p3"),
            ("HERDR_SOCKET_PATH", HERDR_SOCKET),
        ],
        &["--json", "tile", "get"],
    );
    assert_eq!(json_of(&out)["id"], first);
    // ...but in another herdr session it isn't.
    let out = azj(
        &app.socket,
        &[
            ("HERDR_PANE_ID", "w7:p3"),
            ("HERDR_SOCKET_PATH", "/tmp/some-other-herdr.sock"),
        ],
        &["--json", "tile", "get"],
    );
    assert_ne!(json_of(&out)["id"], first);
}

#[test]
fn caller_tab_is_looked_up() {
    let app = start();
    let mut client = Client::connect(&app.socket).unwrap();
    let first_tab = client.call("tile.list", None).unwrap()["tiles"][0]["tab"]
        .as_str()
        .unwrap()
        .to_owned();
    let first_tile = client.call("tile.list", None).unwrap()["tiles"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // A new tab takes focus.
    client.call("tab.create", None).unwrap();
    let out = azj(
        &app.socket,
        &[("AZULEJO_TILE_ID", &first_tile)],
        &["tab", "rename", "build"],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let session = client.call("session.get", None).unwrap();
    let tabs = &session["session"]["windows"][0]["workspaces"][0]["tabs"];
    let renamed = tabs
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == first_tab)
        .unwrap();
    assert_eq!(renamed["name"], "build");
}

#[test]
fn raw_call_and_help() {
    let app = start();
    let out = json_of(&azj(
        &app.socket,
        &[],
        &["--json", "call", "layout.split", r#"{"direction":"down"}"#],
    ));
    assert_eq!(out["type"], "created");

    let help = azj(&app.socket, &[], &["help", "tile", "split"]);
    assert!(help.status.success());
    let text = stdout(&help);
    assert!(
        text.contains("layout.split") && text.contains("--kind browser"),
        "{text}"
    );

    let methods = stdout(&azj(&app.socket, &[], &["methods"]));
    assert!(methods.lines().any(|l| l == "terminal.send_keys"));
}
