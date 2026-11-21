//! Files panels in a running core, against real temp folders.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use azulejo_protocol::{
    Command, Direction, ErrorCode, Event, FilesDir, FilesListing, FilesOpen, FilesOpenTarget,
    FilesPanel, FilesPanelClose, FilesReveal, FilesToggleHidden, LayoutSplit, Reply, TileContent,
    TileId,
};

use crate::{Core, CoreOptions};

fn core() -> Core {
    Core::start(CoreOptions {
        state_path: None,
        api_socket: String::new(),
        log: Arc::new(|_| {}),
    })
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
    let tile = open_panel(&core, None, &root);
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
    let tile = open_panel(&core, None, &root);
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
    let tile = open_panel(&core, None, &root);
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
    let tile = open_panel(&core, None, &root);
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

/// A tile's content by id.
fn content(s: &azulejo_protocol::Session, id: TileId) -> TileContent {
    s.tiles()
        .find(|(_, t)| t.id == id)
        .unwrap()
        .1
        .content
        .clone()
}

#[test]
fn files_open_in_the_editor_beside_the_focused_tile_and_folders_in_a_terminal() {
    let (_t, root) = tree();
    let core = core();
    let panel = open_panel(&core, None, &root);
    let open = |path: &str, target| match core
        .execute(Command::FilesOpen(FilesOpen {
            tile: Some(panel),
            path: path.into(),
            target,
        }))
        .unwrap()
        .reply
    {
        Reply::Created(c) => c.tile.unwrap(),
        other => panic!("unexpected {other:?}"),
    };
    let focused = core.session().unwrap().focused_tile().unwrap();
    let editor = open("README.md", FilesOpenTarget::Editor);
    let session = core.session().unwrap();
    assert_eq!(
        session.focused_tile(),
        Some(editor),
        "the editor takes focus"
    );
    let readme = root.join("README.md").display().to_string();
    assert!(
        matches!(
            content(&session, editor),
            TileContent::Terminal { cwd: Some(cwd), command: Some(command), .. }
                if cwd == root.display().to_string() && command == ["vim", readme.as_str()]
        ),
        "{:?}",
        content(&session, editor)
    );
    // To the right of the tile that had focus.
    let tab = session.focus().tab.unwrap();
    assert_eq!(
        session.tab(tab).unwrap().root.tiles(),
        vec![focused, editor]
    );

    // A folder opens a terminal there instead, as the terminal target does.
    let folder = open("src", FilesOpenTarget::Editor);
    let term = open("src/main.rs", FilesOpenTarget::Terminal);
    let session = core.session().unwrap();
    assert_eq!(session.focused_tile(), Some(term));
    for id in [folder, term] {
        assert!(
            matches!(
                content(&session, id),
                TileContent::Terminal { cwd: Some(cwd), command: None, .. }
                    if cwd == root.join("src").display().to_string()
            ),
            "{:?}",
            content(&session, id)
        );
    }

    // `preview`, from older shells, means the editor.
    let old: azulejo_protocol::FilesOpen = serde_json::from_value(serde_json::json!({
        "tile": panel, "path": "README.md", "target": "preview"
    }))
    .unwrap();
    assert_eq!(old.target, FilesOpenTarget::Editor);
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
    let tile = open_panel(&core, None, &root);
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

fn open_panel(core: &Core, panel: Option<TileId>, root: &Path) -> TileId {
    match core
        .execute(Command::FilesPanel(FilesPanel {
            panel,
            root: root.display().to_string(),
            machine: None,
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

    // A panel survives session changes.
    let focused = core.session().unwrap().focused_tile().unwrap();
    core.execute(Command::LayoutSplit(LayoutSplit {
        tile: None,
        direction: Direction::Right,
        content: None,
    }))
    .unwrap();
    assert_eq!(names(&expand(&core, panel, None)), ["src/", "README.md"]);

    // Opening a file edits it beside the focused tile; a relative path is
    // relative to the panel's root.
    let focused_now = core.session().unwrap().focused_tile().unwrap();
    let editor = match core
        .execute(Command::FilesOpen(FilesOpen {
            tile: Some(panel),
            path: "src/main.rs".into(),
            target: FilesOpenTarget::Editor,
        }))
        .unwrap()
        .reply
    {
        Reply::Created(c) => c.tile.unwrap(),
        other => panic!("unexpected {other:?}"),
    };
    let session = core.session().unwrap();
    assert_ne!(editor, focused);
    assert_ne!(editor, focused_now);
    let TileContent::Terminal {
        command: Some(command),
        ..
    } = content(&session, editor)
    else {
        panic!("unexpected {:?}", content(&session, editor));
    };
    assert_eq!(command[0], "vim");
    assert_eq!(
        std::path::Path::new(&command[1]).canonicalize().unwrap(),
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
            machine: None,
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

// MARK: Remote panels, through the ssh stand-in

const MACHINE: &str = "0123456789abcdef0123456789abcdef";

/// A "remote" machine whose ssh is scripts/ssh-standin.sh (copied into `dir`
/// so tests can cut its link), optionally waiting `delay` before each run.
fn standin_machine(dir: &Path, delay: Option<&str>) -> azulejo_herdr::RemoteMachine {
    use std::os::unix::fs::PermissionsExt;
    let standin = dir.join("standin");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/ssh-standin.sh"),
        &standin,
    )
    .unwrap();
    let ssh = match delay {
        None => standin,
        Some(delay) => {
            let wrapper = dir.join("slow-ssh");
            fs::write(
                &wrapper,
                format!(
                    "#!/bin/sh\nsleep {delay}\nexec {} \"$@\"\n",
                    standin.display()
                ),
            )
            .unwrap();
            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
            wrapper
        }
    };
    azulejo_herdr::RemoteMachine::new(
        azulejo_herdr::MachineProfile {
            id: MACHINE.into(),
            label: "standin".into(),
            target: "standin-host".into(),
            session: "azj-test".into(),
            enabled: true,
        },
        azulejo_herdr::Ssh {
            program: ssh,
            control_dir: dir.join("ctl"),
        },
    )
}

fn open_remote(core: &Core, panel: Option<TileId>, root: &Path, machine: &str) -> TileId {
    match core
        .execute(Command::FilesPanel(FilesPanel {
            panel,
            root: root.display().to_string(),
            machine: Some(machine.into()),
        }))
        .unwrap()
        .reply
    {
        Reply::Created(c) => c.tile.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

/// Waits for `files.changed` naming `dir` of `panel`.
fn changed_for(events: &mpsc::Receiver<Event>, panel: TileId, dir: &Path, wait: Duration) -> bool {
    let want = dir.display().to_string();
    next(events, wait, |e| {
        changed_dirs(e, panel).is_some_and(|d| d.contains(&want))
    })
    .is_some()
}

/// Expands `dir` of a remote panel until its listing is in (loading, then
/// `files.changed`), and returns it.
fn expand_remote(
    core: &Core,
    events: &mpsc::Receiver<Event>,
    panel: TileId,
    dir: &Path,
) -> FilesListing {
    let first = expand(core, panel, Some(&dir.display().to_string()));
    if !first.loading {
        return first;
    }
    assert!(first.entries.is_empty());
    assert!(
        changed_for(events, panel, dir, Duration::from_secs(5)),
        "files.changed once the remote listing is in"
    );
    let l = expand(core, panel, Some(&dir.display().to_string()));
    assert!(!l.loading);
    l
}

#[test]
fn a_remote_panel_lists_over_ssh_polls_and_opens() {
    let (t, root) = tree();
    let core = core();
    let events = core.subscribe();
    core.set_machine_files(MACHINE.into(), Some(standin_machine(t.path(), None)));
    let panel = open_remote(&core, None, &root, MACHINE);

    let l = expand_remote(&core, &events, panel, &root);
    assert_eq!(names(&l), ["src/", "README.md"]);
    assert_eq!(l.machine.as_deref(), Some(MACHINE));
    assert_eq!(l.root, root.display().to_string());
    assert!(l.error.is_none());
    let src = expand_remote(&core, &events, panel, &root.join("src"));
    assert_eq!(names(&src), ["deep/", "main.rs"]);
    assert_eq!(
        src.entries[1].path,
        root.join("src/main.rs").display().to_string()
    );
    // Asked again, it comes from the cache at once.
    let again = expand(&core, panel, Some("src"));
    assert!(!again.loading);
    assert_eq!(names(&again), ["deep/", "main.rs"]);

    // Hidden entries come with the listing; the toggle filters them.
    core.execute(Command::FilesToggleHidden(FilesToggleHidden {
        tile: Some(panel),
        show: Some(true),
    }))
    .unwrap();
    assert_eq!(
        names(&expand(&core, panel, None)),
        ["src/", ".env", "README.md"]
    );
    core.execute(Command::FilesToggleHidden(FilesToggleHidden {
        tile: Some(panel),
        show: Some(false),
    }))
    .unwrap();

    // No watching: polls pick up a new file in an open folder.
    while events.try_recv().is_ok() {}
    fs::write(root.join("src/new.rs"), "").unwrap();
    assert!(
        changed_for(&events, panel, &root.join("src"), Duration::from_secs(8)),
        "a poll lists src again"
    );
    assert_eq!(
        names(&expand(&core, panel, Some("src"))),
        ["deep/", "main.rs", "new.rs"]
    );

    // A terminal opens in the folder (a file's folder for a file).
    let outcome = core
        .execute(Command::FilesOpen(FilesOpen {
            tile: Some(panel),
            path: "src/main.rs".into(),
            target: FilesOpenTarget::Terminal,
        }))
        .unwrap();
    let Reply::Created(created) = outcome.reply else {
        panic!("unexpected {:?}", outcome.reply);
    };
    let session = core.session().unwrap();
    let terminal = session
        .tiles()
        .find(|(_, t)| Some(t.id) == created.tile)
        .map(|(_, t)| t.content.clone());
    let src_dir = root.join("src").display().to_string();
    assert!(
        matches!(&terminal, Some(TileContent::Terminal { cwd: Some(cwd), .. }) if *cwd == src_dir),
        "{terminal:?}"
    );

    // A file opens in the editor, with its path on the machine.
    let outcome = core
        .execute(Command::FilesOpen(FilesOpen {
            tile: Some(panel),
            path: root.join("README.md").display().to_string(),
            target: FilesOpenTarget::Editor,
        }))
        .unwrap();
    let Reply::Created(created) = outcome.reply else {
        panic!("unexpected {:?}", outcome.reply);
    };
    let session = core.session().unwrap();
    let readme = root.join("README.md").display().to_string();
    let editor = content(&session, created.tile.unwrap());
    assert!(
        matches!(&editor, TileContent::Terminal { command: Some(c), .. } if *c == ["vim", readme.as_str()]),
        "{editor:?}"
    );

    // Cutting the link: the open folders keep their listing and say why.
    while events.try_recv().is_ok() {}
    fs::write(t.path().join("standin.fail"), "").unwrap();
    assert!(
        changed_for(&events, panel, &root, Duration::from_secs(8)),
        "files.changed when the machine stops answering"
    );
    let l = expand(&core, panel, None);
    assert_eq!(names(&l), ["src/", "README.md"]);
    let error = l.error.expect("an error");
    assert!(error.starts_with("can't reach standin"), "{error}");

    // Back: the machine reconnecting lists again at once.
    fs::remove_file(t.path().join("standin.fail")).unwrap();
    while events.try_recv().is_ok() {}
    core.set_machine_files(MACHINE.into(), Some(standin_machine(t.path(), None)));
    assert!(
        changed_for(&events, panel, &root, Duration::from_secs(5)),
        "files.changed once it's back"
    );
    assert!(expand(&core, panel, None).error.is_none());

    // A folder that isn't there is an error, not a hang.
    let gone = root.join("gone");
    let first = expand(&core, panel, Some(&gone.display().to_string()));
    let l = if first.error.is_some() {
        first
    } else {
        assert!(changed_for(&events, panel, &gone, Duration::from_secs(5)));
        expand(&core, panel, Some(&gone.display().to_string()))
    };
    assert_eq!(l.error.as_deref(), Some("no such folder"));
    assert!(!l.loading);
    core.execute(Command::FilesPanelClose(FilesPanelClose { panel }))
        .unwrap();
}

#[test]
fn a_remote_panel_on_a_machine_that_isnt_connected_says_so() {
    let (_t, root) = tree();
    let core = core();
    let panel = open_remote(&core, None, &root, "fedcba9876543210fedcba9876543210");
    let l = expand(&core, panel, None);
    assert!(!l.loading);
    assert!(l.entries.is_empty());
    assert_eq!(l.error.as_deref(), Some("the machine isn't connected"));
    // A remote root must be absolute: there's no remote `~` here.
    let err = core
        .execute(Command::FilesPanel(FilesPanel {
            panel: Some(panel),
            root: "src".into(),
            machine: Some(MACHINE.into()),
        }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParams);
}

#[test]
fn moving_a_remote_panel_on_cancels_what_it_was_listing() {
    let (t, root) = tree();
    let core = core();
    let events = core.subscribe();
    core.set_machine_files(MACHINE.into(), Some(standin_machine(t.path(), Some("1"))));
    let panel = open_remote(&core, None, &root, MACHINE);
    assert!(expand(&core, panel, None).loading);
    // The user cd'd elsewhere before the slow listing came back.
    let src = root.join("src");
    assert_eq!(open_remote(&core, Some(panel), &src, MACHINE), panel);
    let first = expand(&core, panel, None);
    assert!(first.loading);
    assert_eq!(first.root, src.display().to_string());
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    while let Ok(e) = events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        if let Some(dirs) = changed_dirs(&e, panel) {
            seen.extend(dirs);
        }
    }
    // Only the new root's listing reached the panel.
    assert_eq!(seen, vec![src.display().to_string()]);
    assert_eq!(names(&expand(&core, panel, None)), ["deep/", "main.rs"]);
}
