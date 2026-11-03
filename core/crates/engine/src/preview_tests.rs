//! Preview tiles in a running core, against real temp files.

use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use azulejo_protocol::{
    Command, Direction, ErrorCode, Event, LayoutSplit, PreviewContent, PreviewKind, PreviewOpen,
    Reply, TileClose, TileContent, TileId, TileTarget,
};

use crate::{Core, CoreOptions};

fn core(state: Option<&Path>) -> Core {
    Core::start(CoreOptions {
        state_path: state.map(Path::to_path_buf),
        api_socket: String::new(),
        log: Arc::new(|_| {}),
    })
}

fn split_preview(core: &Core, path: Option<&Path>) -> TileId {
    let outcome = core
        .execute(Command::LayoutSplit(LayoutSplit {
            tile: None,
            direction: Direction::Right,
            content: Some(TileContent::Preview {
                path: path.map(|p| p.display().to_string()),
            }),
        }))
        .unwrap();
    match outcome.reply {
        Reply::Created(c) => c.tile.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

/// The next `preview.updated` for `tile`, skipping other events.
fn next_preview(events: &mpsc::Receiver<Event>, tile: TileId) -> PreviewContent {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match events.recv_timeout(left) {
            Ok(Event::PreviewUpdated(c)) if c.tile == tile => return c,
            Ok(_) => {}
            Err(e) => panic!("no preview.updated for {tile}: {e}"),
        }
    }
}

fn no_preview(events: &mpsc::Receiver<Event>, wait: Duration) {
    let deadline = std::time::Instant::now() + wait;
    while let Ok(event) =
        events.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
    {
        assert!(
            !matches!(event, Event::PreviewUpdated(_)),
            "unexpected {event:?}"
        );
    }
}

fn get(core: &Core, tile: TileId) -> Result<PreviewContent, azulejo_protocol::Error> {
    core.execute(Command::PreviewGet(TileTarget { tile: Some(tile) }))
        .map(|o| match o.reply {
            Reply::Preview(c) => c,
            other => panic!("unexpected {other:?}"),
        })
}

#[test]
fn renders_on_open_and_on_change_and_stops_on_close() {
    let dir = tempfile::tempdir().unwrap();
    let plan = dir.path().join("plan.md");
    std::fs::write(&plan, "# One\n").unwrap();
    let core = core(None);
    let events = core.subscribe();

    let tile = split_preview(&core, Some(&plan));
    let first = next_preview(&events, tile);
    assert_eq!(first.kind, PreviewKind::Markdown);
    assert!(first.html.as_deref().unwrap().contains(">One<"));
    assert_eq!(
        get(&core, tile).unwrap(),
        first,
        "preview.get returns the latest"
    );

    // FSEvents needs a moment before a new watch reports anything.
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(&plan, "# Two\n").unwrap();
    let second = next_preview(&events, tile);
    assert!(second.html.as_deref().unwrap().contains(">Two<"));
    assert!(second.revision > first.revision);

    // Switching files re-renders and watches the new one.
    let code = dir.path().join("main.rs");
    std::fs::write(&code, "fn main() {}\n").unwrap();
    core.execute(Command::PreviewOpen(PreviewOpen {
        tile: Some(tile),
        path: code.display().to_string(),
    }))
    .unwrap();
    let third = next_preview(&events, tile);
    assert_eq!(third.kind, PreviewKind::Code);
    assert_eq!(third.language.as_deref(), Some("Rust"));
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(&plan, "# not watched any more\n").unwrap();
    no_preview(&events, Duration::from_millis(300));

    core.execute(Command::TileClose(TileClose {
        tile: Some(tile),
        kill: false,
    }))
    .unwrap();
    std::fs::write(&code, "fn main() { closed(); }\n").unwrap();
    no_preview(&events, Duration::from_millis(300));
}

#[test]
fn deleted_and_native_files() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("shot.png");
    std::fs::write(&image, b"png bytes").unwrap();
    let core = core(None);
    let events = core.subscribe();

    let tile = split_preview(&core, Some(&image));
    let shown = next_preview(&events, tile);
    assert_eq!(shown.kind, PreviewKind::Image);
    assert_eq!(
        shown.html, None,
        "the shell shows native kinds from the path"
    );
    assert_eq!(shown.path, image.display().to_string());

    std::thread::sleep(Duration::from_millis(300));
    std::fs::remove_file(&image).unwrap();
    let gone = next_preview(&events, tile);
    assert_eq!(gone.kind, PreviewKind::Missing);
    assert!(gone.html.unwrap().contains("File not found"));
}

#[test]
fn preview_get_errors() {
    let core = core(None);
    let empty = split_preview(&core, None);
    let err = get(&core, empty).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidState);

    let terminal = core
        .session()
        .unwrap()
        .tiles()
        .find(|(_, t)| matches!(t.content, TileContent::Terminal { .. }))
        .map(|(_, t)| t.id)
        .unwrap();
    let err = get(&core, terminal).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidState);
}

#[test]
fn restored_previews_render_and_watch_again() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state.json");
    let plan = dir.path().join("plan.md");
    std::fs::write(&plan, "# Before\n").unwrap();
    let tile = {
        let core = core(Some(&state));
        let tile = split_preview(&core, Some(&plan));
        core.shutdown();
        tile
    };

    let core = core(Some(&state));
    let events = core.subscribe();
    // The startup render may land before anyone subscribed: preview.get has it.
    let restored = get(&core, tile).unwrap();
    assert!(restored.html.unwrap().contains(">Before<"));

    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(&plan, "# After\n").unwrap();
    // The startup render can also arrive here, after subscribing: skip it.
    let mut changed = next_preview(&events, tile);
    if changed.html.as_deref().unwrap().contains(">Before<") {
        changed = next_preview(&events, tile);
    }
    assert!(changed.html.unwrap().contains(">After<"));
}
