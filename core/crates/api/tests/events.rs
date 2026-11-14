//! Event streams over a real socket: ordering, filters, lag, many
//! subscribers, unsubscribe and disconnect cleanup.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use azulejo_api::{Client, ClientError, EventFeed, Server, ServerHandle, WithEvents, spawn_engine};
use azulejo_engine::Engine;
use azulejo_protocol::{ErrorCode, Event};
use serde_json::{Value, json};

static N: AtomicUsize = AtomicUsize::new(0);

fn temp_socket() -> PathBuf {
    // Short paths: Unix sockets cap theirs at ~104 bytes on macOS.
    let dir = std::env::temp_dir().join(format!(
        "azj-ev-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("s.sock")
}

struct Rig {
    server: ServerHandle,
    feed: EventFeed,
    path: PathBuf,
}

/// Removes a test's socket directory.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn start(buffer: Option<usize>) -> Rig {
    let path = temp_socket();
    let feed = EventFeed::new();
    let (tx, _thread) = spawn_engine(Engine::new(), feed.sink());
    let mut server = Server::bind(&path).unwrap();
    if let Some(buffer) = buffer {
        server = server.event_buffer(buffer);
    }
    let server = server
        .spawn(Arc::new(WithEvents::new(tx, feed.clone())))
        .unwrap();
    Rig { server, feed, path }
}

fn client(rig: &Rig) -> Client {
    Client::connect(&rig.path).unwrap()
}

/// Every notification that arrives within `quiet` of the last one.
fn drain(c: &mut Client, quiet: Duration) -> Vec<Value> {
    let mut out = Vec::new();
    while let Some(n) = c.next_notification(Some(quiet)).unwrap() {
        out.push(n);
    }
    out
}

fn names(notes: &[Value]) -> Vec<String> {
    notes
        .iter()
        .filter(|n| n["method"] == "events.event")
        .map(|n| n["params"]["event"].as_str().unwrap().to_owned())
        .collect()
}

fn wait_for(mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

const QUIET: Duration = Duration::from_millis(300);

#[test]
fn events_stream_in_order_after_the_reply() {
    let rig = start(None);
    let mut sub = client(&rig);
    let id = sub.subscribe(None).unwrap();

    let mut actor = client(&rig);
    let mut created = Vec::new();
    for _ in 0..5 {
        let r = actor
            .call("layout.split", Some(json!({"direction": "right"})))
            .unwrap();
        created.push(r["tile"].as_str().unwrap().to_owned());
    }
    let notes = drain(&mut sub, QUIET);
    assert!(notes.iter().all(|n| n["params"]["subscription"] == id));
    let tiles: Vec<String> = notes
        .iter()
        .filter(|n| n["params"]["event"] == "tile.created")
        .map(|n| {
            n["params"]["data"]["tile"]["id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(tiles, created, "tile.created arrives in command order");
    // Each split's events stay together: tile.created before its layout.changed.
    let seq = names(&notes);
    let first_created = seq.iter().position(|n| n == "tile.created").unwrap();
    let first_layout = seq.iter().position(|n| n == "layout.changed").unwrap();
    assert!(first_created < first_layout, "{seq:?}");
}

#[test]
fn name_filters_narrow_the_stream() {
    let rig = start(None);
    let mut tiles = client(&rig);
    tiles.subscribe(Some(json!({"filter": ["tile."]}))).unwrap();
    let mut focus = client(&rig);
    focus
        .subscribe(Some(json!({"filter": ["focus.changed"]})))
        .unwrap();

    let mut actor = client(&rig);
    actor
        .call("layout.split", Some(json!({"direction": "down"})))
        .unwrap();

    let t = names(&drain(&mut tiles, QUIET));
    assert!(!t.is_empty());
    assert!(t.iter().all(|n| n.starts_with("tile.")), "{t:?}");
    let f = names(&drain(&mut focus, QUIET));
    assert_eq!(f, vec!["focus.changed"]);
}

#[test]
fn workspace_and_tile_scopes() {
    let rig = start(None);
    let mut actor = client(&rig);
    let first = actor.call("session.get", None).unwrap();
    let ws = first["session"]["windows"][0]["workspaces"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let tile = first["session"]["windows"][0]["workspaces"][0]["tabs"][0]["focused"]
        .as_str()
        .unwrap()
        .to_owned();
    let window = first["session"]["windows"][0]["id"].clone();

    let mut in_ws = client(&rig);
    in_ws.subscribe(Some(json!({"workspace": ws}))).unwrap();
    let mut on_tile = client(&rig);
    on_tile
        .subscribe(Some(
            json!({"tile": tile, "filter": ["tile.updated", "tile.closed"]}),
        ))
        .unwrap();

    // Something in another workspace…
    let other = actor
        .call("workspace.create", Some(json!({"window": window})))
        .unwrap();
    let other_ws = other["workspace"].as_str().unwrap().to_owned();
    actor
        .call("tab.create", Some(json!({"workspace": other_ws})))
        .unwrap();
    // …then something in ours: rename the seeded tile.
    actor
        .call("tile.rename", Some(json!({"tile": tile, "title": "logs"})))
        .unwrap();

    let ws_notes = drain(&mut in_ws, QUIET);
    for n in &ws_notes {
        let data = &n["params"]["data"];
        assert_ne!(data["workspace"], json!(other_ws), "{n}");
        assert_ne!(data["workspace"]["id"], json!(other_ws), "{n}");
    }
    assert!(names(&ws_notes).contains(&"tile.updated".to_owned()));

    let tile_notes = names(&drain(&mut on_tile, QUIET));
    assert_eq!(tile_notes, vec!["tile.updated"]);
}

#[test]
fn a_slow_reader_lags_instead_of_stalling_the_core() {
    let rig = start(Some(4));
    let mut slow = client(&rig);
    let id = slow.subscribe(None).unwrap();

    // Publish far more than the socket, the connection queue and the
    // subscription buffer can hold, without reading.
    let flood = 20_000;
    let big = "x".repeat(512);
    let started = Instant::now();
    for _ in 0..flood {
        rig.feed.publish(&[Event::HerdrStatus {
            connected: true,
            message: Some(big.clone()),
        }]);
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "publishing must never wait for subscribers"
    );
    // Commands still go through while the subscriber is stuck.
    let mut actor = client(&rig);
    actor.call("core.ping", None).unwrap();

    let mut notes = drain(&mut slow, QUIET);
    // Drops at the very end are reported with the next event.
    rig.feed.publish(&[Event::HerdrStatus {
        connected: false,
        message: None,
    }]);
    notes.extend(drain(&mut slow, QUIET));
    assert_eq!(notes.last().unwrap()["params"]["data"]["connected"], false);
    let lagged: u64 = notes
        .iter()
        .filter(|n| n["method"] == "events.lagged")
        .map(|n| {
            assert_eq!(n["params"]["subscription"], id);
            n["params"]["dropped"].as_u64().unwrap()
        })
        .sum();
    let delivered = names(&notes).len() as u64;
    assert!(lagged > 0, "expected a lag notice");
    assert_eq!(
        delivered + lagged,
        flood + 1,
        "every event is delivered or counted"
    );
}

#[test]
fn many_subscribers_each_get_everything() {
    let rig = start(None);
    let mut subs: Vec<Client> = (0..12)
        .map(|_| {
            let mut c = client(&rig);
            c.subscribe(None).unwrap();
            c
        })
        .collect();
    // Two subscriptions on one connection get the events twice, tagged apart.
    let mut doubled = client(&rig);
    let a = doubled.subscribe(None).unwrap();
    let b = doubled.subscribe(None).unwrap();
    assert_ne!(a, b);

    let mut actor = client(&rig);
    for _ in 0..3 {
        actor
            .call("layout.split", Some(json!({"direction": "right"})))
            .unwrap();
    }
    let expected = names(&drain(&mut subs[0], QUIET));
    assert!(!expected.is_empty());
    for s in &mut subs[1..] {
        assert_eq!(names(&drain(s, QUIET)), expected);
    }
    let both = drain(&mut doubled, QUIET);
    for id in [a, b] {
        let mine: Vec<Value> = both
            .iter()
            .filter(|n| n["params"]["subscription"] == id)
            .cloned()
            .collect();
        assert_eq!(names(&mine), expected);
    }
}

#[test]
fn unsubscribe_and_disconnect_clean_up() {
    let rig = start(None);
    let mut c = client(&rig);
    let a = c.subscribe(None).unwrap();
    let _b = c.subscribe(Some(json!({"filter": ["tile."]}))).unwrap();
    let mut other = client(&rig);
    other.subscribe(None).unwrap();
    wait_for(|| rig.server.subscriptions() == 3);

    c.call("events.unsubscribe", Some(json!({"subscription": a})))
        .unwrap();
    assert_eq!(rig.server.subscriptions(), 2);
    let err = c
        .call("events.unsubscribe", Some(json!({"subscription": a})))
        .unwrap_err();
    assert!(matches!(err, ClientError::Rpc(e) if e.kind == Some(ErrorCode::NotFound)));
    // Someone else's subscription can't be ended from here.
    let err = c
        .call("events.unsubscribe", Some(json!({"subscription": 999})))
        .unwrap_err();
    assert!(matches!(err, ClientError::Rpc(e) if e.kind == Some(ErrorCode::NotFound)));

    drop(c);
    wait_for(|| rig.server.subscriptions() == 1);
    drop(other);
    wait_for(|| rig.server.subscriptions() == 0);

    // The core keeps going.
    let mut actor = client(&rig);
    actor
        .call("layout.split", Some(json!({"direction": "right"})))
        .unwrap();
}

#[test]
fn responses_and_events_share_a_connection() {
    let rig = start(None);
    let mut c = client(&rig);
    c.subscribe(Some(json!({"filter": ["tile.created"]})))
        .unwrap();
    // Our own command's events arrive on the same connection as its reply.
    let r = c
        .call("layout.split", Some(json!({"direction": "right"})))
        .unwrap();
    let notes = drain(&mut c, QUIET);
    assert_eq!(names(&notes), vec!["tile.created"]);
    assert_eq!(notes[0]["params"]["data"]["tile"]["id"], r["tile"]);
}

#[test]
fn a_real_core_streams_its_events() {
    use azulejo_api::CoreDispatcher;
    use azulejo_engine::{Core, CoreOptions};
    let path = temp_socket();
    let _dir = TempDir(path.parent().unwrap().to_owned());
    let core = Arc::new(Core::start(CoreOptions {
        state_path: None,
        ..Default::default()
    }));
    let _server = Server::bind(&path)
        .unwrap()
        .spawn(Arc::new(CoreDispatcher::new(core)))
        .unwrap();
    let mut c = Client::connect(&path).unwrap();
    c.subscribe(Some(json!({"filter": ["tile.created", "focus."]})))
        .unwrap();
    let mut actor = Client::connect(&path).unwrap();
    actor
        .call("layout.split", Some(json!({"direction": "down"})))
        .unwrap();
    assert_eq!(
        names(&drain(&mut c, QUIET)),
        vec!["tile.created", "focus.changed"]
    );
}
