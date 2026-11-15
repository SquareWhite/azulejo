//! Files tiles in a running core, against real temp folders.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use azulejo_protocol::{
    Command, Direction, ErrorCode, Event, FilesDir, FilesListing, FilesOpen, FilesOpenTarget,
    FilesPanel, FilesPanelClose, FilesReveal, FilesSetRoot, FilesToggleHidden, LayoutSplit, Reply,
    TileContent, TileId,
};

use crate::{Core, CoreOptions};

fn core() -> Core {
    Core::start(CoreOptions {
        state_path: None,
        api_socket: String::new(),
        log: Arc::new(|_| {}),
    })
}

fn split_files(core: &Core, root: &Path) -> TileId {
    let outcome = core
        .execute(Command::LayoutSplit(LayoutSplit {
            tile: None,
            direction: Direction::Right,
            content: Some(TileContent::Files {
                root: Some(root.display().to_string()),
            }),
        }))
        .unwrap();
    match outcome.reply {
        Reply::Created(c) => c.tile.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

fn expand(core: &Core, tile: TileId, dir: Option<&str>) -> FilesListing {
    match core
        .execute(Command::FilesExpand(FilesDir {
            tile: Some(tile),
            dir: dir.map(str::to_owned),
        }))
        .unwrap()
        .reply
    {
        Reply::Files(l) => l,
        other => panic!("unexpected {other:?}"),
    }
}

fn names(l: &FilesListing) -> Vec<String> {
    l.entries
        .iter()
        .map(|e| {
            if e.dir {
                format!("{}/", e.name)
            } else {
                e.name.clone()
            }
        })
        .collect()
}

/// The next event matching `pred` within `wait`.
fn next(
    events: &mpsc::Receiver<Event>,
    wait: Duration,
    pred: impl Fn(&Event) -> bool,
) -> Option<Event> {
    let deadline = Instant::now() + wait;
    while let Ok(e) = events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        if pred(&e) {
            return Some(e);
        }
    }
    None
}

fn changed_dirs(e: &Event, tile: TileId) -> Option<Vec<String>> {
    match e {
        Event::FilesChanged(c) if c.tile == tile => Some(c.dirs.clone()),
        _ => None,
    }
}

fn tree() -> (tempfile::TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap().join("proj");
    fs::create_dir_all(root.join("src/deep")).unwrap();
    fs::write(root.join("README.md"), "# hi\n").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(root.join("src/deep/x.rs"), "").unwrap();
    fs::write(root.join(".env"), "").unwrap();
    (t, root)
}

#[test]
fn lists_root_and_folders_lazily() {
    let (_t, root) = tree();
    let core = core();
    let tile = split_files(&core, &root);
    let l = expand(&core, tile, None);
    assert_eq!(l.root, root.display().to_string());
    assert_eq!(l.dir, l.root);
    assert_eq!(names(&l), ["src/", "README.md"]);
    let src = expand(&core, tile, Some("src"));
    assert_eq!(names(&src), ["deep/", "main.rs"]);
    // A plain list works too, and folders that don't exist are errors.
    let listed = core
        .execute(Command::FilesList(FilesDir {
            tile: Some(tile),
            dir: Some(root.join("src/deep").display().to_string()),
        }))
        .unwrap();
    assert!(matches!(listed.reply, Reply::Files(l) if names(&l) == ["x.rs"]));
    let err = core
        .execute(Command::FilesExpand(FilesDir {
            tile: Some(tile),
            dir: Some("nope".into()),
        }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn changes_in_open_folders_arrive_quickly_and_only_once() {
    let (_t, root) = tree();
    let core = core();
    let events = core.subscribe();
    let tile = split_files(&core, &root);
    expand(&core, tile, None);
    expand(&core, tile, Some("src"));
    // Let FSEvents' replay of the setup pass.
    std::thread::sleep(Duration::from_millis(300));
    while events.try_recv().is_ok() {}

    let src = root.join("src").display().to_string();
    for step in ["create", "rename", "delete"] {
        let start = Instant::now();
        match step {
            "create" => fs::write(root.join("src/new.rs"), "").unwrap(),
            "rename" => fs::rename(root.join("src/new.rs"), root.join("src/renamed.rs")).unwrap(),
            _ => fs::remove_file(root.join("src/renamed.rs")).unwrap(),
        }
        let e = next(&events, Duration::from_secs(2), |e| {
            changed_dirs(e, tile).is_some()
        })
        .unwrap_or_else(|| panic!("no files.changed after {step}"));
        let took = start.elapsed();
        assert_eq!(changed_dirs(&e, tile).unwrap(), vec![src.clone()]);
        assert!(took < Duration::from_millis(200), "{step} took {took:?}");
        // Exactly one notification per change.
        assert!(
            next(&events, Duration::from_millis(250), |e| changed_dirs(
                e, tile
            )
            .is_some())
            .is_none(),
            "{step}: repeated files.changed"
        );
    }

    // Contents-only edits don't relist anything.
    fs::write(root.join("src/main.rs"), "fn main() { println!(); }\n").unwrap();
    assert!(
        next(&events, Duration::from_millis(300), |e| changed_dirs(
            e, tile
        )
        .is_some())
        .is_none()
    );

    // Unopened folders aren't watched.
    fs::write(root.join("src/deep/y.rs"), "").unwrap();
    assert!(
        next(&events, Duration::from_millis(300), |e| changed_dirs(
            e, tile
        )
        .is_some())
        .is_none()
    );

    // Collapsing stops watching.
    core.execute(Command::FilesCollapse(FilesDir {
        tile: Some(tile),
        dir: Some("src".into()),
    }))
    .unwrap();
    fs::write(root.join("src/z.rs"), "").unwrap();
    assert!(
        next(&events, Duration::from_millis(300), |e| changed_dirs(
            e, tile
        )
        .is_some())
        .is_none()
    );
}

#[test]
fn toggle_hidden_relists_open_folders() {
    let (_t, root) = tree();
    let core = core();
    let events = core.subscribe();
    let tile = split_files(&core, &root);
    assert!(!names(&expand(&core, tile, None)).contains(&".env".to_owned()));
    core.execute(Command::FilesToggleHidden(FilesToggleHidden {
        tile: Some(tile),
        show: None,
    }))
    .unwrap();
    let e = next(&events, Duration::from_secs(1), |e| {
        changed_dirs(e, tile).is_some()
    })
    .unwrap();
    assert_eq!(
        changed_dirs(&e, tile).unwrap(),
        vec![root.display().to_string()]
    );
    let l = expand(&core, tile, None);
    assert!(l.show_hidden);
    let env = l.entries.iter().find(|e| e.name == ".env").unwrap();
    assert!(env.hidden);
}

#[test]
fn reveal_opens_the_folders_down_to_a_path() {
    let (_t, root) = tree();
    let core = core();
    let events = core.subscribe();
    let tile = split_files(&core, &root);
    let out = core
        .execute(Command::FilesReveal(FilesReveal {
            tile: Some(tile),
            path: "src/deep/x.rs".into(),
        }))
        .unwrap();
    let revealed = out
        .events
        .iter()
        .chain(
            next(&events, Duration::from_secs(1), |e| {
                matches!(e, Event::FilesRevealed(_))
            })
            .iter(),
        )
        .find_map(|e| match e {
            Event::FilesRevealed(r) => Some(r.clone()),
            _ => None,
        })
        .unwrap();
    let d = |p: &str| root.join(p).display().to_string();
    assert_eq!(revealed.path, d("src/deep/x.rs"));
    assert_eq!(
        revealed.dirs,
        vec![root.display().to_string(), d("src"), d("src/deep")]
    );
    // Revealed folders are watched.
    std::thread::sleep(Duration::from_millis(300));
    while events.try_recv().is_ok() {}
    fs::write(root.join("src/deep/w.rs"), "").unwrap();
    assert!(
        next(&events, Duration::from_secs(1), |e| changed_dirs(e, tile)
            .is_some())
        .is_some()
    );
    // Outside the root is refused.
    let err = core
        .execute(Command::FilesReveal(FilesReveal {
            tile: Some(tile),
            path: "/".into(),
        }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParams);
}

#[test]
fn open_reuses_the_tabs_preview_tile_and_terminals_start_in_the_folder() {
    let (_t, root) = tree();
    let core = core();
    let tile = split_files(&core, &root);
    let open = |path: &str, target| match core
        .execute(Command::FilesOpen(FilesOpen {
            tile: Some(tile),
            path: path.into(),
            target,
        }))
        .unwrap()
        .reply
    {
        Reply::Created(c) => c.tile.unwrap(),
        other => panic!("unexpected {other:?}"),
    };
    let preview = open("README.md", FilesOpenTarget::Preview);
    let session = core.session().unwrap();
    assert_eq!(
        session.focused_tile(),
        Some(tile),
        "focus stays in the files tile"
    );
    let content = |s: &azulejo_protocol::Session, id| {
        s.tiles()
            .find(|(_, t)| t.id == id)
            .unwrap()
            .1
            .content
            .clone()
    };
    assert_eq!(
        content(&session, preview),
        TileContent::Preview {
            path: Some(root.join("README.md").display().to_string())
        }
    );
    let again = open("src/main.rs", FilesOpenTarget::Preview);
    assert_eq!(again, preview, "reuses the tab's preview tile");

    let term = open("src/main.rs", FilesOpenTarget::Terminal);
    let session = core.session().unwrap();
    assert_eq!(session.focused_tile(), Some(term));
    match content(&session, term) {
        TileContent::Terminal { cwd, .. } => {
            assert_eq!(cwd, Some(root.join("src").display().to_string()))
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn set_root_points_the_tile_elsewhere() {
    let (_t, root) = tree();
    let core = core();
    let tile = split_files(&core, &root);
    core.execute(Command::FilesSetRoot(FilesSetRoot {
        tile: Some(tile),
        root: root.join("src").display().to_string(),
    }))
    .unwrap();
    let l = expand(&core, tile, None);
    assert_eq!(names(&l), ["deep/", "main.rs"]);
    let err = core
        .execute(Command::FilesSetRoot(FilesSetRoot {
            tile: Some(tile),
            root: root.join("README.md").display().to_string(),
        }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParams);
}

#[test]
fn git_status_decorates_entries() {
    if std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let (_t, root) = tree();
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["-c", "user.email=t@t", "-c", "user.name=t", "add", "."]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "commit",
        "-qm",
        "init",
    ]);
    let core = core();
    let events = core.subscribe();
    let tile = split_files(&core, &root);
    expand(&core, tile, None);
    // The first status (clean) comes once the tile's watches are up; an
    // edit before then could land between that read and the watches.
    next(
        &events,
        Duration::from_secs(10),
        |e| matches!(e, Event::FilesGit(g) if g.tile == tile && g.paths.is_empty()),
    )
    .expect("files.git for the clean tree");
    // `src` isn't open: only the tree watch can see this edit.
    fs::write(root.join("src/main.rs"), "changed\n").unwrap();
    let main = root.join("src/main.rs").display().to_string();
    let src = root.join("src").display().to_string();
    let e = next(
        &events,
        Duration::from_secs(10),
        |e| matches!(e, Event::FilesGit(g) if g.tile == tile && g.paths.contains_key(&main)),
    )
    .expect("files.git with the change");
    let Event::FilesGit(g) = e else {
        unreachable!()
    };
    assert_eq!(g.paths[&src], azulejo_protocol::FileGitState::Modified);
    // Listings carry the decoration once status is known.
    let l = expand(&core, tile, None);
    let src_entry = l.entries.iter().find(|e| e.name == "src").unwrap();
    assert_eq!(
        src_entry.git,
        Some(azulejo_protocol::FileGitState::Modified)
    );
}

#[test]
fn closing_the_tile_stops_its_watches() {
    let (_t, root) = tree();
    let core = core();
    let events = core.subscribe();
    let tile = split_files(&core, &root);
    expand(&core, tile, None);
    core.execute(Command::TileClose(azulejo_protocol::TileClose {
        tile: Some(tile),
        kill: false,
    }))
    .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    while events.try_recv().is_ok() {}
    fs::write(root.join("after.txt"), "").unwrap();
    assert!(
        next(&events, Duration::from_millis(300), |e| changed_dirs(
            e, tile
        )
        .is_some())
        .is_none()
    );
    let err = expand_err(&core, tile);
    assert!(matches!(
        err.code,
        ErrorCode::NotFound | ErrorCode::InvalidState
    ));
}

fn open_panel(core: &Core, panel: Option<TileId>, root: &Path) -> TileId {
    match core
        .execute(Command::FilesPanel(FilesPanel {
            panel,
            root: root.display().to_string(),
        }))
        .unwrap()
        .reply
    {
        Reply::Created(c) => c.tile.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_panel_lists_watches_and_opens_outside_the_layout() {
    let (_t, root) = tree();
    let core = core();
    let events = core.subscribe();
    let before = core.session().unwrap();
    let panel = open_panel(&core, None, &root);
    // Not a tile: the session doesn't change.
    assert_eq!(core.session().unwrap(), before);
    assert_eq!(names(&expand(&core, panel, None)), ["src/", "README.md"]);
    assert_eq!(
        names(&expand(&core, panel, Some("src"))),
        ["deep/", "main.rs"]
    );

    std::thread::sleep(Duration::from_millis(300));
    while events.try_recv().is_ok() {}
    fs::write(root.join("src/new.rs"), "").unwrap();
    let e = next(&events, Duration::from_secs(2), |e| {
        changed_dirs(e, panel).is_some()
    })
    .expect("files.changed for the panel");
    assert_eq!(
        changed_dirs(&e, panel).unwrap(),
        vec![root.join("src").display().to_string()]
    );

    // A panel survives session changes that drop files tiles.
    let focused = core.session().unwrap().focused_tile().unwrap();
    core.execute(Command::LayoutSplit(LayoutSplit {
        tile: None,
        direction: Direction::Right,
        content: None,
    }))
    .unwrap();
    assert_eq!(names(&expand(&core, panel, None)), ["src/", "README.md"]);

    // Opening a file shows it beside the focused tile, which keeps focus.
    let focused_now = core.session().unwrap().focused_tile().unwrap();
    let preview = match core
        .execute(Command::FilesOpen(FilesOpen {
            tile: Some(panel),
            path: root.join("README.md").display().to_string(),
            target: FilesOpenTarget::Preview,
        }))
        .unwrap()
        .reply
    {
        Reply::Created(c) => c.tile.unwrap(),
        other => panic!("unexpected {other:?}"),
    };
    let session = core.session().unwrap();
    assert_ne!(preview, focused);
    assert_eq!(session.focused_tile(), Some(focused_now));
    assert!(session.tiles().any(|(_, t)| t.id == preview
        && t.content
            == TileContent::Preview {
                path: Some(root.join("README.md").display().to_string())
            }));

    // A relative path is relative to the panel's root, as for a tile.
    core.execute(Command::FilesOpen(FilesOpen {
        tile: Some(panel),
        path: "src/main.rs".into(),
        target: FilesOpenTarget::Preview,
    }))
    .unwrap();
    let session = core.session().unwrap();
    let shown = session
        .tiles()
        .find(|(_, t)| t.id == preview)
        .map(|(_, t)| t.content.clone());
    let Some(TileContent::Preview { path: Some(shown) }) = shown else {
        panic!("unexpected {shown:?}");
    };
    assert_eq!(
        std::path::Path::new(&shown).canonicalize().unwrap(),
        root.join("src/main.rs").canonicalize().unwrap()
    );

    // Pointing it elsewhere keeps its id; the same root is a no-op.
    assert_eq!(open_panel(&core, Some(panel), &root.join("src")), panel);
    assert_eq!(
        names(&expand(&core, panel, None)),
        ["deep/", "main.rs", "new.rs"]
    );
    assert_eq!(open_panel(&core, Some(panel), &root.join("src")), panel);

    // A file isn't a root.
    let err = core
        .execute(Command::FilesPanel(FilesPanel {
            panel: Some(panel),
            root: root.join("README.md").display().to_string(),
        }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParams);

    // Closing stops its watches and forgets it.
    core.execute(Command::FilesPanelClose(FilesPanelClose { panel }))
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    while events.try_recv().is_ok() {}
    fs::write(root.join("src/after.rs"), "").unwrap();
    assert!(
        next(&events, Duration::from_millis(300), |e| changed_dirs(
            e, panel
        )
        .is_some())
        .is_none()
    );
    assert!(expand_err(&core, panel).code != ErrorCode::Internal);
}

fn expand_err(core: &Core, tile: TileId) -> azulejo_protocol::Error {
    core.execute(Command::FilesExpand(FilesDir {
        tile: Some(tile),
        dir: None,
    }))
    .unwrap_err()
}
