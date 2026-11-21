use std::os::unix::fs::PermissionsExt;
use std::time::SystemTime;

use azulejo_herdr::{MachineProfile, Ssh};

use super::*;

fn names(name: &str, n: usize) -> Vec<String> {
    candidates(OsStr::new(name))
        .take(n)
        .map(|c| c.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn numbers_names_the_way_finder_does() {
    assert_eq!(
        names("notes.txt", 3),
        ["notes.txt", "notes 2.txt", "notes 3.txt"]
    );
    assert_eq!(names("notes 2.txt", 2), ["notes 2.txt", "notes 3.txt"]);
    assert_eq!(names("folder", 2), ["folder", "folder 2"]);
    assert_eq!(names(".env", 2), [".env", ".env 2"]);
    assert_eq!(names("a.tar.gz", 2), ["a.tar.gz", "a.tar 2.gz"]);
    assert_eq!(names("v 007", 2), ["v 007", "v 007 2"]);
    assert_eq!(names("trailing.", 2), ["trailing.", "trailing. 2"]);
    assert_eq!(names("2", 2), ["2", "2 2"]);
    let taken: HashSet<String> = ["notes.txt", "notes 2.txt"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(unique(OsStr::new("Notes.TXT"), &taken), "Notes 3.TXT");
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).unwrap()
}

fn local(
    sources: &[&Path],
    folder: &Path,
    mode: TransferMode,
) -> Result<Vec<TransferItem>, String> {
    run(&Transfer {
        sources: sources.iter().map(|p| p.to_path_buf()).collect(),
        from: Place::Local,
        folder: folder.to_path_buf(),
        to: Place::Local,
        mode,
    })
    .map_err(|e| e.message)
}

/// The temp dir, resolved (`/var` is `/private/var` on macOS).
fn tmp() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap();
    (dir, path)
}

#[test]
fn copies_files_and_folders_without_overwriting() {
    let (_keep, t) = tmp();
    let src = t.join("src");
    let dst = t.join("dst");
    write(&src.join("notes.txt"), "new");
    write(&src.join("tree/a/b.txt"), "deep");
    std::os::unix::fs::symlink("a/b.txt", src.join("tree/link")).unwrap();
    write(&dst.join("notes.txt"), "old");
    write(&dst.join("NOTES 2.txt"), "case");
    fs::set_permissions(src.join("notes.txt"), fs::Permissions::from_mode(0o640)).unwrap();
    let past = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    fs::File::options()
        .write(true)
        .open(src.join("notes.txt"))
        .unwrap()
        .set_modified(past)
        .unwrap();

    let items = local(
        &[&src.join("notes.txt"), &src.join("tree")],
        &dst,
        TransferMode::Copy,
    )
    .unwrap();
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|i| i.error.is_none()), "{items:?}");
    // notes.txt and (on a case-insensitive disk) notes 2.txt are taken.
    let first = PathBuf::from(items[0].dest.as_ref().unwrap());
    assert_ne!(first, dst.join("notes.txt"));
    assert!(first.to_string_lossy().ends_with(".txt"));
    assert_eq!(read(&first), "new");
    assert_eq!(read(dst.join("notes.txt")), "old");
    assert_eq!(read(dst.join("NOTES 2.txt")), "case");
    let meta = fs::metadata(&first).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o640);
    assert_eq!(meta.modified().unwrap(), past);
    assert_eq!(
        items[1].dest.as_deref(),
        Some(dst.join("tree").to_str().unwrap())
    );
    assert_eq!(read(dst.join("tree/a/b.txt")), "deep");
    assert_eq!(
        fs::read_link(dst.join("tree/link")).unwrap(),
        Path::new("a/b.txt")
    );
    // The originals stay.
    assert_eq!(read(src.join("notes.txt")), "new");
    assert!(src.join("tree/a/b.txt").exists());

    // Again: the next free names.
    let again = local(&[&src.join("tree")], &dst, TransferMode::Copy).unwrap();
    assert_eq!(
        again[0].dest.as_deref(),
        Some(dst.join("tree 2").to_str().unwrap())
    );
}

#[test]
fn moves_by_renaming_and_leaves_things_already_there() {
    let (_keep, t) = tmp();
    let src = t.join("src");
    let dst = t.join("dst");
    write(&src.join("a.txt"), "a");
    write(&src.join("dir/x"), "x");
    write(&dst.join("a.txt"), "taken");
    let items = local(
        &[&src.join("a.txt"), &src.join("dir")],
        &dst,
        TransferMode::Move,
    )
    .unwrap();
    assert_eq!(
        items[0].dest.as_deref(),
        Some(dst.join("a 2.txt").to_str().unwrap())
    );
    assert_eq!(
        items[1].dest.as_deref(),
        Some(dst.join("dir").to_str().unwrap())
    );
    assert_eq!(read(dst.join("a 2.txt")), "a");
    assert_eq!(read(dst.join("a.txt")), "taken");
    assert_eq!(read(dst.join("dir/x")), "x");
    assert!(!src.join("a.txt").exists());
    assert!(!src.join("dir").exists());

    // Into the folder it's in: nothing happens.
    let same = local(&[&dst.join("a.txt")], &dst, TransferMode::Move).unwrap();
    assert_eq!(
        same[0].dest.as_deref(),
        Some(dst.join("a.txt").to_str().unwrap())
    );
    assert!(same[0].error.is_none());
    assert!(!dst.join("a 3.txt").exists());

    // Copying into the same folder makes a numbered copy.
    let copy = local(&[&dst.join("a.txt")], &dst, TransferMode::Copy).unwrap();
    assert_eq!(
        copy[0].dest.as_deref(),
        Some(dst.join("a 3.txt").to_str().unwrap())
    );
}

#[test]
fn refuses_what_it_cant_do_and_carries_on() {
    let (_keep, t) = tmp();
    write(&t.join("dir/inner/f"), "f");
    write(&t.join("ok.txt"), "ok");
    let items = local(
        &[&t.join("missing"), &t.join("dir"), &t.join("ok.txt")],
        &t.join("dir/inner"),
        TransferMode::Move,
    )
    .unwrap();
    assert!(
        items[0].error.as_deref().unwrap().contains("no such file"),
        "{items:?}"
    );
    assert_eq!(
        items[1].error.as_deref(),
        Some("can't move a folder into itself")
    );
    assert!(items[2].error.is_none());
    assert!(t.join("dir/inner/ok.txt").exists());

    let copy = local(&[&t.join("dir")], &t.join("dir/inner"), TransferMode::Copy).unwrap();
    assert_eq!(
        copy[0].error.as_deref(),
        Some("can't copy a folder into itself")
    );

    let err = local(
        &[&t.join("dir")],
        &t.join("dir/inner/f"),
        TransferMode::Copy,
    )
    .unwrap_err();
    assert!(err.ends_with("isn't a folder"), "{err}");
    let err = local(&[&t.join("dir")], Path::new("relative"), TransferMode::Copy).unwrap_err();
    assert!(err.contains("absolute"), "{err}");
}

// MARK: - Through the SSH stand-in

fn machine(dir: &Path) -> RemoteMachine {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/ssh-standin.sh");
    let ssh = dir.join("ssh");
    fs::copy(src, &ssh).unwrap();
    RemoteMachine::new(
        MachineProfile {
            id: "0123456789abcdef0123456789abcdef".into(),
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

fn between(
    sources: &[&Path],
    from: Place,
    folder: &Path,
    to: Place,
    mode: TransferMode,
) -> Result<Vec<TransferItem>, String> {
    run(&Transfer {
        sources: sources.iter().map(|p| p.to_path_buf()).collect(),
        from,
        folder: folder.to_path_buf(),
        to,
        mode,
    })
    .map_err(|e| e.message)
}

#[test]
fn uploads_with_scp_and_removes_originals_only_after() {
    let (_keep, t) = tmp();
    let m = machine(&t);
    let src = t.join("local");
    let remote = t.join("remote dir");
    let odd = "it's a [draft] *.txt";
    write(&src.join(odd), "odd");
    write(&src.join("tree/inner.md"), "inner");
    write(&remote.join(odd), "there already");

    let items = between(
        &[&src.join(odd), &src.join("tree")],
        Place::Local,
        &remote,
        Place::Remote(m.clone()),
        TransferMode::Copy,
    )
    .unwrap();
    assert!(items.iter().all(|i| i.error.is_none()), "{items:?}");
    let renamed = remote.join("it's a [draft] *.txt".replace(" *.txt", " * 2.txt"));
    assert_eq!(items[0].dest.as_deref(), Some(renamed.to_str().unwrap()));
    assert_eq!(read(&renamed), "odd");
    assert_eq!(read(remote.join(odd)), "there already");
    assert_eq!(read(remote.join("tree/inner.md")), "inner");
    assert!(src.join(odd).exists());

    // A move: uploaded as `tree 2`, then the local folder goes.
    let moved = between(
        &[&src.join("tree")],
        Place::Local,
        &remote,
        Place::Remote(m.clone()),
        TransferMode::Move,
    )
    .unwrap();
    assert_eq!(
        moved[0].dest.as_deref(),
        Some(remote.join("tree 2").to_str().unwrap())
    );
    assert_eq!(read(remote.join("tree 2/inner.md")), "inner");
    assert!(!src.join("tree").exists());

    // The link is cut: nothing moves, the original stays.
    fs::write(t.join("ssh.fail"), "").unwrap();
    let err = between(
        &[&src.join(odd)],
        Place::Local,
        &remote,
        Place::Remote(m.clone()),
        TransferMode::Move,
    )
    .unwrap_err();
    assert!(err.starts_with("can't reach box"), "{err}");
    assert!(src.join(odd).exists());
    fs::remove_file(t.join("ssh.fail")).unwrap();

    // No such folder there.
    let err = between(
        &[&src.join(odd)],
        Place::Local,
        &remote.join("nope"),
        Place::Remote(m),
        TransferMode::Copy,
    )
    .unwrap_err();
    assert!(err.ends_with("no such folder"), "{err}");
}

#[test]
fn downloads_with_scp() {
    let (_keep, t) = tmp();
    let m = machine(&t);
    let remote = t.join("remote");
    let here = t.join("here");
    let odd = "glob [x] *?.txt";
    write(&remote.join(odd), "odd");
    write(&remote.join("glob x a.txt"), "decoy");
    write(&remote.join("dir/f"), "f");
    write(&here.join("dir/old"), "old");

    let items = between(
        &[&remote.join(odd), &remote.join("dir")],
        Place::Remote(m.clone()),
        &here,
        Place::Local,
        TransferMode::Move,
    )
    .unwrap();
    assert!(items.iter().all(|i| i.error.is_none()), "{items:?}");
    assert_eq!(read(here.join(odd)), "odd");
    assert_eq!(
        items[1].dest.as_deref(),
        Some(here.join("dir 2").to_str().unwrap())
    );
    assert_eq!(read(here.join("dir 2/f")), "f");
    assert_eq!(read(here.join("dir/old")), "old");
    assert!(!remote.join(odd).exists());
    assert!(!remote.join("dir").exists());
    assert!(remote.join("glob x a.txt").exists());

    let missing = between(
        &[&remote.join("gone.txt")],
        Place::Remote(m),
        &here,
        Place::Local,
        TransferMode::Copy,
    )
    .unwrap();
    assert!(
        missing[0].error.is_some() && missing[0].dest.is_none(),
        "{missing:?}"
    );
}

#[test]
fn a_running_core_answers_files_transfer() {
    use std::sync::Arc;

    use azulejo_protocol::{Command, FilesTransfer, Reply};

    use crate::{Core, CoreOptions};

    let (_keep, t) = tmp();
    write(&t.join("a.txt"), "a");
    write(&t.join("b.txt"), "b");
    fs::create_dir_all(t.join("local")).unwrap();
    fs::create_dir_all(t.join("remote")).unwrap();
    let core = Core::start(CoreOptions {
        state_path: None,
        api_socket: String::new(),
        log: Arc::new(|_| {}),
    });
    let id = "0123456789abcdef0123456789abcdef";
    let ask = |source: &str, folder: &Path, to: Option<&str>, mode| {
        core.execute(Command::FilesTransfer(FilesTransfer {
            sources: vec![t.join(source).display().to_string()],
            from: None,
            folder: folder.display().to_string(),
            to: to.map(str::to_owned),
            mode,
        }))
    };
    let items = |r: Result<crate::Outcome, Error>| match r.unwrap().reply {
        Reply::Transferred { items } => items,
        other => panic!("{other:?}"),
    };

    let local = items(ask(
        "a.txt",
        &t.join("local"),
        Some("local"),
        TransferMode::Move,
    ));
    assert_eq!(
        local[0].dest.as_deref(),
        Some(t.join("local/a.txt").to_str().unwrap())
    );
    assert!(!t.join("a.txt").exists());

    // Not connected yet, then through the stand-in.
    let err = ask("b.txt", &t.join("remote"), Some(id), TransferMode::Copy).unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable);
    core.set_machine_files(id.into(), Some(machine(&t)));
    let remote = items(ask(
        "b.txt",
        &t.join("remote"),
        Some(id),
        TransferMode::Copy,
    ));
    assert_eq!(
        remote[0].dest.as_deref(),
        Some(t.join("remote/b.txt").to_str().unwrap())
    );
    assert_eq!(read(t.join("remote/b.txt")), "b");
    assert!(t.join("b.txt").exists());

    let err = core
        .execute(Command::FilesTransfer(FilesTransfer {
            sources: vec!["relative.txt".into()],
            from: None,
            folder: t.display().to_string(),
            to: None,
            mode: TransferMode::Copy,
        }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParams);
}
