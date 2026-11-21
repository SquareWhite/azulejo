use std::sync::Arc;

use azulejo_herdr::{MachineProfile, Ssh};
use azulejo_protocol::{Command, Reply};

use super::*;
use crate::{Core, CoreOptions};

/// The temp dir, resolved (`/var` is `/private/var` on macOS).
fn tmp() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap();
    (dir, path)
}

const ID: &str = "0123456789abcdef0123456789abcdef";

fn machine(dir: &Path) -> RemoteMachine {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/ssh-standin.sh");
    let ssh = dir.join("ssh");
    fs::copy(src, &ssh).unwrap();
    RemoteMachine::new(
        MachineProfile {
            id: ID.into(),
            label: "box".into(),
            target: "standin-host".into(),
            session: "unused".into(),
            enabled: true,
        },
        Ssh {
            program: ssh,
            control_dir: dir.join("ctl"),
        },
    )
}

fn make(path: &Path, kind: FileKind, place: Place) -> Result<PathBuf, String> {
    create(&Create {
        path: path.to_path_buf(),
        kind,
        place,
    })
    .map_err(|e| e.message)
}

/// A Trash of the test's own: a folder things move into.
fn test_trash(dir: PathBuf) -> impl Fn(&Path) -> Result<PathBuf, String> + Send + Sync {
    move |path: &Path| {
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        transfer::move_local(path, &dir)
    }
}

#[test]
fn makes_files_and_folders_and_never_replaces_anything() {
    let (_keep, t) = tmp();
    let file = make(&t.join("notes.txt"), FileKind::File, Place::Local).unwrap();
    assert_eq!(file, t.join("notes.txt"));
    assert_eq!(fs::read(&file).unwrap(), b"");
    let dir = make(&t.join("src"), FileKind::Folder, Place::Local).unwrap();
    assert!(dir.is_dir());

    fs::write(&file, "kept").unwrap();
    let err = make(&t.join("notes.txt"), FileKind::File, Place::Local).unwrap_err();
    assert_eq!(err, "notes.txt is already there");
    assert_eq!(fs::read_to_string(&file).unwrap(), "kept");
    let err = make(&t.join("notes.txt"), FileKind::Folder, Place::Local).unwrap_err();
    assert_eq!(err, "notes.txt is already there");
    let err = make(&t.join("src"), FileKind::File, Place::Local).unwrap_err();
    assert_eq!(err, "src is already there");
    // A dangling link takes its name too.
    std::os::unix::fs::symlink(t.join("gone"), t.join("link")).unwrap();
    let err = make(&t.join("link"), FileKind::File, Place::Local).unwrap_err();
    assert_eq!(err, "link is already there");
    assert!(!t.join("gone").exists());

    let err = make(&t.join("nowhere/x"), FileKind::File, Place::Local).unwrap_err();
    assert!(err.contains("no such file or folder"), "{err}");
    let err = make(&file.join("x"), FileKind::File, Place::Local).unwrap_err();
    assert!(err.contains("isn't a folder"), "{err}");
}

#[test]
fn takes_paths_against_a_panel_and_refuses_odd_ones() {
    let machines = HashMap::new();
    let panel = Panel {
        root: PathBuf::from("/srv/app"),
        machine: None,
    };
    let c = Create::new(
        FilesCreate {
            tile: None,
            path: "src/../lib.rs".into(),
            kind: FileKind::File,
            machine: None,
        },
        Some(&panel),
        &machines,
    )
    .unwrap();
    assert_eq!(c.path, PathBuf::from("/srv/app/lib.rs"));

    let bad = |path: &str| {
        Create::new(
            FilesCreate {
                tile: None,
                path: path.into(),
                kind: FileKind::File,
                machine: None,
            },
            None,
            &machines,
        )
        .unwrap_err()
        .message
    };
    assert!(bad("relative.txt").contains("absolute"));
    assert!(bad("/").contains("no name"));
    assert!(bad(&format!("/tmp/{}", "x".repeat(300))).contains("too long"));

    let remote_panel = Panel {
        root: PathBuf::from("/srv/app"),
        machine: Some(ID.into()),
    };
    let err = Create::new(
        FilesCreate {
            tile: None,
            path: "x".into(),
            kind: FileKind::File,
            machine: Some("local".into()),
        },
        Some(&remote_panel),
        &machines,
    )
    .unwrap_err();
    assert!(err.message.contains("another machine"), "{}", err.message);
    let err = Create::new(
        FilesCreate {
            tile: None,
            path: "x".into(),
            kind: FileKind::File,
            machine: None,
        },
        Some(&remote_panel),
        &machines,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable);

    let del = |paths: &[&str], panel: Option<&Panel>| {
        Delete::new(
            FilesDelete {
                tile: None,
                paths: paths.iter().map(|p| p.to_string()).collect(),
                machine: None,
            },
            panel,
            &machines,
        )
    };
    assert_eq!(
        del(&["a.txt", "src/b"], Some(&panel)).unwrap().paths,
        [
            PathBuf::from("/srv/app/a.txt"),
            PathBuf::from("/srv/app/src/b")
        ]
    );
    for (paths, why) in [
        (&["/srv/app"][..], "inside the panel"),
        (&["../other"][..], "inside the panel"),
        (&["/etc/hosts"][..], "inside the panel"),
    ] {
        let err = del(paths, Some(&panel)).unwrap_err();
        assert!(err.message.contains(why), "{paths:?}: {}", err.message);
    }
    for path in ["/", "/usr", "/tmp/.."] {
        let err = del(&[path], None).unwrap_err();
        assert!(
            err.message.starts_with("won't delete"),
            "{path}: {}",
            err.message
        );
    }
}

#[test]
fn deletes_into_the_trash_one_by_one() {
    let (_keep, t) = tmp();
    let trash = test_trash(t.join("trash"));
    fs::create_dir_all(t.join("proj/src")).unwrap();
    fs::write(t.join("proj/src/a.rs"), "a").unwrap();
    fs::write(t.join("proj/notes.txt"), "n").unwrap();
    fs::write(t.join("trash-me.txt"), "first").unwrap();
    std::os::unix::fs::symlink(t.join("proj/notes.txt"), t.join("proj/link")).unwrap();
    let d = Delete {
        paths: vec![
            t.join("proj/src"),
            t.join("proj/link"),
            t.join("proj/missing"),
            t.join("proj/notes.txt"),
        ],
        place: Place::Local,
    };
    let items = delete(&d, &trash).unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0].trashed.as_deref(), t.join("trash/src").to_str());
    assert!(items[0].error.is_none());
    assert!(t.join("trash/src/a.rs").is_file());
    // The link goes, not what it points to.
    assert!(
        fs::symlink_metadata(t.join("trash/link"))
            .unwrap()
            .is_symlink()
    );
    assert!(items[2].error.as_deref().unwrap().contains("no such file"));
    assert!(items[2].trashed.is_none());
    assert!(items[3].error.is_none());
    assert_eq!(fs::read_to_string(t.join("trash/notes.txt")).unwrap(), "n");
    assert_eq!(fs::read_dir(t.join("proj")).unwrap().count(), 0);

    // A name already in the Trash gets a number.
    fs::write(t.join("proj/notes.txt"), "again").unwrap();
    let items = delete(
        &Delete {
            paths: vec![t.join("proj/notes.txt")],
            place: Place::Local,
        },
        &trash,
    )
    .unwrap();
    assert_eq!(
        items[0].trashed.as_deref(),
        t.join("trash/notes 2.txt").to_str()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_system_trash_says_why_it_cant() {
    // Only the failing path: a test never puts anything in the user's Trash.
    let (_keep, t) = tmp();
    let err = system_trash(&t.join("not-there.txt")).unwrap_err();
    assert!(err.contains("not-there.txt"), "{err}");
}

#[test]
fn makes_and_removes_on_a_machine_through_ssh() {
    let (_keep, t) = tmp();
    let m = machine(&t);
    let place = || Place::Remote(m.clone());
    let root = t.join("remote");
    fs::create_dir_all(&root).unwrap();
    let odd = root.join("it's a \"file\" $HOME.txt");
    assert_eq!(make(&odd, FileKind::File, place()).unwrap(), odd);
    assert!(odd.is_file());
    make(&root.join("src"), FileKind::Folder, place()).unwrap();
    assert!(root.join("src").is_dir());
    fs::write(root.join("src/lib.rs"), "x").unwrap();

    let err = make(&odd, FileKind::File, place()).unwrap_err();
    assert_eq!(err, "it's a \"file\" $HOME.txt is already there");
    let err = make(&root.join("src"), FileKind::Folder, place()).unwrap_err();
    assert_eq!(err, "src is already there");
    let err = make(&root.join("nope/x"), FileKind::File, place()).unwrap_err();
    assert!(err.ends_with("nope isn't a folder"), "{err}");

    let d = Delete {
        paths: vec![root.join("src"), root.join("gone"), odd.clone()],
        place: place(),
    };
    let items = delete(&d, &test_trash(t.join("never"))).unwrap();
    assert!(items[0].error.is_none(), "{items:?}");
    assert_eq!(items[1].error.as_deref(), Some("no such file or folder"));
    assert!(items[2].error.is_none(), "{items:?}");
    assert!(!root.join("src").exists() && !odd.exists());
    // Removed for good: nothing went to a Trash.
    assert!(!t.join("never").exists());

    // The link is cut: nothing happens, and the error says why.
    fs::write(t.join("ssh.fail"), "").unwrap();
    let err = make(&root.join("later.txt"), FileKind::File, place()).unwrap_err();
    assert!(err.starts_with("can't reach box"), "{err}");
    let err = delete(
        &Delete {
            paths: vec![root.clone()],
            place: place(),
        },
        &test_trash(t.join("never")),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable);
    assert!(root.exists());
}

#[test]
fn parses_what_the_remove_script_says() {
    let paths = vec![
        PathBuf::from("/a"),
        PathBuf::from("/b"),
        PathBuf::from("/c"),
    ];
    let items = parse_removed(b"O\t1\0E\t2\tcouldn't remove all of it\0", &paths);
    assert!(items[0].error.is_none());
    assert_eq!(items[1].error.as_deref(), Some("couldn't remove all of it"));
    assert_eq!(items[2].error.as_deref(), Some("the machine didn't say"));
    assert_eq!(parents(&paths), [PathBuf::from("/")]);
}

#[test]
fn a_running_core_answers_files_create_and_delete() {
    let (_keep, t) = tmp();
    fs::create_dir_all(t.join("remote")).unwrap();
    let core = Core::start(CoreOptions {
        state_path: None,
        api_socket: String::new(),
        log: Arc::new(|_| {}),
    });
    let made = |path: PathBuf, machine: Option<&str>| {
        core.execute(Command::FilesCreate(FilesCreate {
            tile: None,
            path: path.display().to_string(),
            kind: FileKind::Folder,
            machine: machine.map(str::to_owned),
        }))
        .map(|o| o.reply)
    };
    assert_eq!(
        made(t.join("here"), None).unwrap(),
        Reply::FileCreated {
            path: t.join("here").display().to_string()
        }
    );
    assert!(t.join("here").is_dir());
    let err = made(t.join("remote/x"), Some(ID)).unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable);

    core.set_machine_files(ID.into(), Some(machine(&t)));
    made(t.join("remote/x"), Some(ID)).unwrap();
    assert!(t.join("remote/x").is_dir());
    let reply = core
        .execute(Command::FilesDelete(FilesDelete {
            tile: None,
            paths: vec![t.join("remote/x").display().to_string()],
            machine: Some(ID.into()),
        }))
        .unwrap()
        .reply;
    let Reply::Deleted { items } = reply else {
        panic!("{reply:?}");
    };
    assert!(items[0].error.is_none(), "{items:?}");
    assert!(!t.join("remote/x").exists());

    let reply = core
        .execute(Command::FilesSshUrl(FilesSshUrl {
            tile: None,
            path: "/srv/a b.rs".into(),
            machine: Some(ID.into()),
        }))
        .unwrap()
        .reply;
    assert_eq!(
        reply,
        Reply::SshUrl {
            url: "ssh://standin-host/srv/a%20b.rs".into()
        }
    );
}

#[test]
fn a_remote_path_as_an_ssh_url() {
    let (_keep, t) = tmp();
    let mut machines = HashMap::from([(ID.to_owned(), machine(&t))]);
    let panel = Panel {
        root: PathBuf::from("/srv/my app"),
        machine: Some(ID.into()),
    };
    let url = |path: &str, panel: Option<&Panel>, machine: Option<&str>, machines: &_| {
        ssh_url(
            FilesSshUrl {
                tile: None,
                path: path.into(),
                machine: machine.map(str::to_owned),
            },
            panel,
            machines,
        )
    };
    // Against the panel's root, on its machine, the host as saved.
    assert_eq!(
        url("src/../notes #1.md", Some(&panel), None, &machines).unwrap(),
        "ssh://standin-host/srv/my%20app/notes%20%231.md"
    );
    assert_eq!(
        url("/etc/hosts", None, Some(ID), &machines).unwrap(),
        "ssh://standin-host/etc/hosts"
    );
    // A user and port in the target come along.
    machines.get_mut(ID).unwrap().profile.target = "ssh://me@box.example.invalid:2222".into();
    assert_eq!(
        url("/tmp/a b", Some(&panel), None, &machines).unwrap(),
        "ssh://me@box.example.invalid:2222/tmp/a%20b"
    );

    // Local has no URL; a machine that isn't connected, none either.
    let local = Panel {
        root: t.clone(),
        machine: None,
    };
    let err = url("x", Some(&local), None, &machines).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParams);
    let err = url(
        "/x",
        None,
        Some("fedcba9876543210fedcba9876543210"),
        &machines,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable);
    // A `[herdr] ssh` wrapper: other apps can't follow it.
    machines.get_mut(ID).unwrap().ssh.program = t.join("tunnel.sh");
    let err = url("/x", Some(&panel), None, &machines).unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable);
    assert!(err.message.contains("tunnel.sh"), "{}", err.message);
}

#[test]
fn the_editor_command_takes_arguments() {
    let path = Path::new("/srv/my notes.md");
    assert_eq!(
        crate::editor_command("vim", path),
        ["vim", "/srv/my notes.md"]
    );
    assert_eq!(
        crate::editor_command("  ", path),
        ["vim", "/srv/my notes.md"]
    );
    assert_eq!(
        crate::editor_command("/opt/bin/hx", path),
        ["/opt/bin/hx", "/srv/my notes.md"]
    );
    assert_eq!(
        crate::editor_command("nvim -p", path),
        ["nvim -p '/srv/my notes.md'"]
    );
}
