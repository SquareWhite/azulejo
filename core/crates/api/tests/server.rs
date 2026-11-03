//! The server on a real socket, backed by an in-process engine.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use azulejo_api::{Client, ClientError, Server, ServerHandle, spawn_engine};
use azulejo_engine::Engine;
use azulejo_protocol::ErrorCode;
use serde_json::json;

/// A fresh directory under /tmp; macOS's $TMPDIR is too long for some socket
/// paths, and sockets cap at about 100 bytes.
fn temp_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(format!(
        "/tmp/azj-api-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn start(path: &PathBuf) -> ServerHandle {
    let (tx, _engine) = spawn_engine(Engine::new(), |_| {});
    Server::bind(path).unwrap().spawn(Arc::new(tx)).unwrap()
}

#[test]
fn serves_commands_over_the_socket() {
    let dir = temp_dir();
    let path = dir.join("azulejo.sock");
    let _server = start(&path);

    let mut client = Client::connect(&path).unwrap();
    let pong = client.call("core.ping", None).unwrap();
    assert_eq!(pong["type"], "pong");

    let created = client
        .call(
            "layout.split",
            Some(json!({"direction": "right", "content": {"kind": "browser", "url": "http://localhost:3000"}})),
        )
        .unwrap();
    let tile = created["tile"].as_str().unwrap().to_owned();

    let info = client
        .call("tile.get", Some(json!({"tile": tile})))
        .unwrap();
    assert_eq!(info["kind"], "browser");
    assert_eq!(info["url"], "http://localhost:3000");
    assert_eq!(info["focused"], true);

    let version = client.call("core.version", None).unwrap();
    assert_eq!(version["api_version"], 1);

    fs::remove_dir_all(dir).ok();
}

#[test]
fn errors_carry_the_protocol_code() {
    let dir = temp_dir();
    let path = dir.join("azulejo.sock");
    let _server = start(&path);
    let mut client = Client::connect(&path).unwrap();
    let err = client
        .call(
            "tile.get",
            Some(json!({"tile": "tile_01JBX3K6Z8V9W0Q1R2S3T4V5W6"})),
        )
        .unwrap_err();
    match err {
        ClientError::Rpc(e) => {
            assert_eq!(e.kind, Some(ErrorCode::NotFound));
            assert_eq!(e.code, -32001);
        }
        other => panic!("expected an RPC error, got {other:?}"),
    }
    // The connection stays usable after an error.
    assert_eq!(client.call("core.ping", None).unwrap()["type"], "pong");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn an_over_long_socket_path_is_refused_with_its_limit() {
    let dir = temp_dir();
    let max = azulejo_api::max_socket_path_len();
    let mut name = "s".repeat(max + 10);
    name.push_str(".sock");
    let path = dir.join(name);
    let Err(err) = Server::bind(&path) else {
        panic!("bind must fail")
    };
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let msg = err.to_string();
    assert!(msg.contains(&format!("limit of {max}")), "{msg}");
    assert!(
        !dir.exists(),
        "nothing created for a path the OS can't take"
    );
}

#[test]
fn socket_is_private() {
    let dir = temp_dir().join("nested");
    let path = dir.join("azulejo.sock");
    let _server = start(&path);
    let dir_mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    let sock_mode = fs::symlink_metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700, "socket directory mode");
    assert_eq!(sock_mode, 0o600, "socket mode");
    fs::remove_dir_all(dir.parent().unwrap()).ok();
}

#[test]
fn many_clients_at_once() {
    let dir = temp_dir();
    let path = dir.join("azulejo.sock");
    let _server = start(&path);
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || {
                let mut client = Client::connect(&path).unwrap();
                for _ in 0..20 {
                    client
                        .call("layout.split", Some(json!({"direction": "down"})))
                        .unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let mut client = Client::connect(&path).unwrap();
    let tiles = client.call("tile.list", None).unwrap();
    assert_eq!(tiles["tiles"].as_array().unwrap().len(), 1 + 8 * 20);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn stale_socket_is_replaced_and_live_one_is_not() {
    let dir = temp_dir();
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("azulejo.sock");

    // A socket file nobody listens on, as a crashed server leaves behind.
    drop(UnixListener::bind(&path).unwrap());
    assert!(path.exists());
    let server = start(&path);
    assert!(Client::connect(&path).is_ok());

    // A second server on the same path is refused.
    let err = Server::bind(&path).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);

    // Stopping removes the socket file.
    server.shutdown();
    assert!(!path.exists());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn never_deletes_a_regular_file() {
    let dir = temp_dir();
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("azulejo.sock");
    fs::write(&path, "not a socket").unwrap();
    let err = Server::bind(&path).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&path).unwrap(), "not a socket");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn not_running_is_its_own_error() {
    let dir = temp_dir();
    let err = Client::connect(dir.join("azulejo.sock")).unwrap_err();
    assert!(matches!(err, ClientError::NotRunning { .. }), "{err:?}");
}
