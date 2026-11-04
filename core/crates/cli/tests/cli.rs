//! The `azj` binary against a real socket and an in-process engine.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use azulejo_api::{Client, EventFeed, Server, ServerHandle, WithEvents, WithInfo, spawn_engine};
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
    let feed = EventFeed::new();
    let (tx, _engine) = spawn_engine(Engine::new(), feed.sink());
    // Pretend the core's herdr session lives at HERDR_SOCKET.
    let info = serde_json::Map::from_iter([("herdr_socket".to_owned(), json!(HERDR_SOCKET))]);
    let server = Server::bind(&socket)
        .unwrap()
        .spawn(Arc::new(WithEvents::new(WithInfo::new(tx, info), feed)))
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
    assert!(o.status.success(), "azj failed: {}{}", stdout(o), stderr(o));
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

/// Runs `azj` like [`azj`], from working folder `cwd`.
fn azj_in(cwd: &Path, socket: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_azj"));
    cmd.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("AZULEJO_SOCKET", socket)
        .current_dir(cwd)
        .args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

#[test]
fn preview_open_from_a_terminal_opens_the_plan_beside_it() {
    let app = start();
    let work = app.dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    // What azj sees as its working folder (/tmp is a symlink on macOS).
    let work = work.canonicalize().unwrap();
    let mut client = Client::connect(&app.socket).unwrap();
    let terminal = client.call("tile.list", None).unwrap()["tiles"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let out = azj_in(
        &work,
        &app.socket,
        &[("AZULEJO_TILE_ID", &terminal)],
        &["--json", "preview", "open", "--path", "plan.md"],
    );
    let reply = json_of(&out);
    assert_eq!(reply["type"], "created");
    let preview = reply["tile"].as_str().unwrap().to_owned();
    let info = json_of(&azj(&app.socket, &[], &["--json", "tile", "get", &preview]));
    assert_eq!(info["kind"], "preview");
    assert_eq!(info["path"], work.join("plan.md").display().to_string());
    let layout = client.call("layout.get", None).unwrap();
    assert_eq!(layout["tree"]["a"]["id"], terminal.as_str());
    assert_eq!(layout["tree"]["b"]["id"], preview.as_str());
    assert_eq!(layout["focused"], terminal.as_str(), "focus stays put");

    // Run against the preview tile itself, it shows the file there.
    let out = azj_in(
        &work,
        &app.socket,
        &[],
        &[
            "--json", "preview", "open", "--tile", &preview, "--path", "b.md",
        ],
    );
    assert_eq!(json_of(&out)["type"], "ok");
    let info = json_of(&azj(&app.socket, &[], &["--json", "tile", "get", &preview]));
    assert_eq!(info["path"], work.join("b.md").display().to_string());
    assert_eq!(
        client.call("tile.list", None).unwrap()["tiles"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn files_open_paths_are_relative_to_the_tiles_root_not_azjs_folder() {
    let app = start();
    let root = app.dir.join("repo");
    let elsewhere = app.dir.join("elsewhere");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(root.join("README.md"), "# hi\n").unwrap();
    let files = json_of(&azj(
        &app.socket,
        &[],
        &[
            "--json",
            "tile",
            "split",
            "--right",
            "--kind",
            "files",
            "--root",
            root.to_str().unwrap(),
        ],
    ))["tile"]
        .as_str()
        .unwrap()
        .to_owned();

    let out = azj_in(
        &elsewhere,
        &app.socket,
        &[],
        &[
            "--json",
            "files",
            "open",
            "--tile",
            &files,
            "--path",
            "README.md",
            "--target",
            "terminal",
        ],
    );
    let terminal = json_of(&out)["tile"].as_str().unwrap().to_owned();
    let info = json_of(&azj(
        &app.socket,
        &[],
        &["--json", "tile", "get", &terminal],
    ));
    assert_eq!(info["cwd"], root.display().to_string(), "{info}");
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

/// `azj events` prints events until `--count` of them arrived. Run inside a
/// tile, it still streams everything: the caller's IDs aren't filters.
#[test]
fn events_follow_prints_each_event() {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let app = start();
    let mut c = Client::connect(&app.socket).unwrap();
    let session = c.call("session.get", None).unwrap();
    let ws = &session["session"]["windows"][0]["workspaces"][0];
    let tile = ws["tabs"][0]["focused"].as_str().unwrap().to_owned();

    let spawn = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_azj"))
            .env_clear()
            .env("AZULEJO_SOCKET", &app.socket)
            .env("AZULEJO_TILE_ID", &tile)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let mut json = spawn(&[
        "events",
        "--follow",
        "--filter",
        "tile.created",
        "--count",
        "2",
        "--json",
    ]);
    let mut human = spawn(&["events", "subscribe", "--filter", "tile.", "-n", "2"]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app._server.subscriptions() < 2 {
        assert!(Instant::now() < deadline, "azj never subscribed");
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut created = Vec::new();
    for _ in 0..2 {
        let r = c
            .call("layout.split", Some(json!({"direction": "right"})))
            .unwrap();
        created.push(r["tile"].clone());
    }
    let read = |child: &mut std::process::Child| {
        let status = child.wait().unwrap();
        let mut out = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut out)
            .unwrap();
        let mut err = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut err)
            .unwrap();
        assert!(status.success(), "{err}");
        out
    };
    let out = read(&mut json);
    let lines: Vec<Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "{out}");
    for (line, id) in lines.iter().zip(&created) {
        assert_eq!(line["method"], "events.event");
        assert_eq!(line["params"]["event"], "tile.created");
        assert_eq!(&line["params"]["data"]["tile"]["id"], id);
    }
    let out = read(&mut human);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(lines[0].starts_with("tile.created tile="), "{out}");
    assert!(lines[0].contains("kind=terminal"), "{out}");
}

#[test]
fn events_flags_are_checked() {
    let app = start();
    let o = azj(&app.socket, &[], &["events", "--count", "lots"]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    let o = azj(&app.socket, &[], &["events", "--workspace", "nope"]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
}

/// Parses a `--json` error: one object on stdout, nothing on stderr, and the
/// usual exit code.
fn json_error(o: &Output, exit: i32) -> Value {
    assert_eq!(o.status.code(), Some(exit), "{}{}", stdout(o), stderr(o));
    assert_eq!(stderr(o), "", "--json errors go to stdout only");
    let text = stdout(o);
    assert_eq!(text.lines().count(), 1, "{text}");
    let error: Value = serde_json::from_str(&text).unwrap();
    let keys: Vec<&String> = error.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["code", "data", "message"], "{error}");
    assert!(error["message"].as_str().is_some_and(|m| !m.is_empty()));
    assert!(error["data"].is_object(), "{error}");
    error
}

#[test]
fn json_errors_without_a_daemon_are_objects() {
    let socket = Path::new("/tmp/azj-nothing-here/azulejo.sock");
    for args in [
        &["--json", "ping"][..],
        &["--json", "call", "core.ping"],
        &["--json", "events", "--count", "1"],
    ] {
        let error = json_error(&azj(socket, &[], args), 3);
        assert_eq!(error["code"], "unavailable", "{args:?}");
        assert_eq!(error["data"]["socket"], socket.display().to_string());
        assert!(
            error["message"].as_str().unwrap().contains("isn't running"),
            "{error}"
        );
    }
}

#[test]
fn json_errors_from_the_core_keep_its_code_and_data() {
    let app = start();
    let missing = "tile_01JBX3K6Z8V9W0Q1R2S3T4V5W6";
    let error = json_error(
        &azj(&app.socket, &[], &["--json", "tile", "get", missing]),
        1,
    );
    assert_eq!(error["code"], "not_found");
    assert_eq!(error["data"], json!({"tile": missing}));
    assert!(
        !error["message"].as_str().unwrap().contains(missing),
        "the message names no IDs: {error}"
    );

    let error = json_error(&azj(&app.socket, &[], &["--json", "call", "nope.thing"]), 1);
    assert_eq!(error["code"], "method_not_found");

    // A stale caller from the environment.
    let error = json_error(
        &azj(
            &app.socket,
            &[("AZULEJO_TILE_ID", missing)],
            &["--json", "tile", "get"],
        ),
        1,
    );
    assert_eq!(error["code"], "not_found");
    assert_eq!(error["data"]["tile"], missing);
}

#[test]
fn json_errors_for_bad_arguments() {
    let app = start();
    let cases: &[(&[&str], &str)] = &[
        (&["--json"], "invalid_request"),
        (&["--json", "tile", "--right"], "invalid_request"),
        (&["--json", "nope", "thing"], "method_not_found"),
        (&["--json", "help", "nope"], "method_not_found"),
        (&["--json", "layout", "split"], "invalid_params"),
        (&["--json", "call"], "invalid_request"),
        (&["--json", "call", "core.ping", "{"], "invalid_params"),
        (&["--json", "events", "--count", "lots"], "invalid_params"),
        (&["--json", "ping", "--socket"], "invalid_params"),
    ];
    for (args, code) in cases {
        let error = json_error(&azj(&app.socket, &[], args), 2);
        assert_eq!(error["code"], *code, "{args:?}: {error}");
    }
    let error = json_error(&azj(&app.socket, &[], &["--json", "nope", "thing"]), 2);
    assert_eq!(error["data"], json!({"method": "nope.thing"}));
    let error = json_error(&azj(&app.socket, &[], &["--json", "events", "-n", "x"]), 2);
    assert_eq!(error["data"], json!({"param": "count"}));

    // Without --json it's still text on stderr.
    let o = azj(&app.socket, &[], &["nope", "thing"]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(stdout(&o), "");
    assert!(
        stderr(&o).starts_with("azj: unknown command"),
        "{}",
        stderr(&o)
    );
}

/// `azj config keys` lists every action with its context, default and current
/// chords and source; `azj config defaults` prints a template that parses.
#[test]
fn config_keys_and_defaults() {
    use azulejo_config::Platform;
    use azulejo_daemon::{App as Daemon, GhosttyFiles, HerdrChoice, Options};
    let dir = PathBuf::from(format!("/tmp/azj-cli-keys-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        "[keys.macos.palette]\npalette_next = \"ctrl+down\"\n[keys]\nquit = \"none\"\n",
    )
    .unwrap();
    let socket = dir.join("azulejo.sock");
    let daemon = Daemon::start(Options {
        config_path: Some(config.clone()),
        ghostty: GhosttyFiles::Off,
        state_path: None,
        socket_path: Some(socket.clone()),
        api_required: true,
        herdr: HerdrChoice::Off,
        platform: Platform::Macos,
        log: Arc::new(|line| eprintln!("test: {line}")),
    })
    .unwrap();

    let listing = json_of(&azj(&socket, &[], &["--json", "config", "keys"]));
    assert_eq!(listing["type"], "keys");
    assert_eq!(listing["platform"], "macos");
    assert_eq!(listing["path"], config.display().to_string());
    let keys = listing["keys"].as_array().unwrap();
    let row = |action: &str, context: &str| {
        keys.iter()
            .find(|k| k["action"] == action && k["context"] == context)
            .unwrap_or_else(|| panic!("no {action} in {context}"))
            .clone()
    };
    let next = row("palette_next", "palette");
    assert_eq!(next["default"], json!(["down", "ctrl+n", "ctrl+j"]));
    assert_eq!(next["current"], json!(["ctrl+down"]));
    assert_eq!(next["source"], "user");
    let quit = row("quit", "global");
    assert_eq!(quit["default"], json!(["cmd+q"]));
    assert_eq!(quit["current"], json!([]));
    assert_eq!(quit["source"], "user");
    let back = row("browser_back", "browser");
    assert_eq!(back["current"], json!(["cmd+bracketleft"]));
    assert_eq!(back["source"], "default");
    assert_eq!(row("switch_tab", "global")["current"], json!(["ctrl+1..8"]));
    for context in [
        "global", "terminal", "browser", "preview", "files", "palette", "find_bar", "resize",
    ] {
        assert!(keys.iter().any(|k| k["context"] == context), "{context}");
    }

    // The table, and `azj keys list` as another name for it.
    let table = azj(&socket, &[], &["config", "keys"]);
    assert!(table.status.success(), "{}", stderr(&table));
    let text = stdout(&table);
    assert!(text.starts_with("macos keymap; config: "), "{text}");
    let line = text
        .lines()
        .find(|l| l.starts_with("palette_next "))
        .unwrap();
    let cells: Vec<&str> = line
        .split("  ")
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect();
    assert_eq!(
        cells,
        [
            "palette_next",
            "palette",
            "down, ctrl+n, ctrl+j",
            "ctrl+down",
            "user"
        ],
        "{line}"
    );
    assert!(
        text.lines()
            .any(|l| l.starts_with("quit ") && l.contains(" - ")),
        "{text}"
    );
    assert_eq!(stdout(&azj(&socket, &[], &["keys", "list"])), text);
    // send_text: one row per chord, its text in config syntax.
    let clear = row("send_text", "terminal");
    assert_eq!(clear["current"], json!(["cmd+l"]));
    assert_eq!(clear["text"], "\u{c}");
    assert_eq!(clear["default_text"], "\u{c}");
    let line = text
        .lines()
        .find(|l| l.starts_with("send_text "))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        line.starts_with("send_text \"\\u000c\"")
            && line.contains("terminal")
            && line.contains("cmd+l"),
        "{line}"
    );

    // The template: every action, commented out, parseable as is.
    let defaults = azj(&socket, &[], &["config", "defaults"]);
    assert!(defaults.status.success(), "{}", stderr(&defaults));
    let template = stdout(&defaults);
    for section in [
        "[keys.macos]",
        "[keys.macos.palette]",
        "[keys.macos.find_bar]",
        "[keys.macos.resize]",
    ] {
        assert!(template.contains(section), "{section}");
    }
    assert!(template.contains("# palette_next = [\"down\", \"ctrl+n\", \"ctrl+j\"]"));
    let parsed = azulejo_config::Config::from_toml(&template, None, Platform::Macos);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let raw = json_of(&azj(&socket, &[], &["--json", "config", "defaults"]));
    assert_eq!(raw["type"], "text");

    daemon.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
