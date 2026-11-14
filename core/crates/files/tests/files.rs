use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use azulejo_files::{Change, DirWatcher, MAX_ENTRIES, list, signature};

fn names(dir: &Path, show_hidden: bool) -> Vec<String> {
    list(dir, show_hidden, MAX_ENTRIES)
        .unwrap()
        .entries
        .into_iter()
        .map(|e| {
            if e.dir {
                format!("{}/", e.name)
            } else {
                e.name
            }
        })
        .collect()
}

/// A tiny work tree: `.git` is enough for the ignore rules to apply.
fn repo() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    fs::create_dir(r.join(".git")).unwrap();
    fs::write(r.join(".gitignore"), "target/\n*.log\n/build\n").unwrap();
    for d in ["src", "target", "build", "docs", "src/deep"] {
        fs::create_dir_all(r.join(d)).unwrap();
    }
    for f in [
        "README.md",
        "a10.txt",
        "a2.txt",
        "B.txt",
        "debug.log",
        ".env",
        "src/main.rs",
        "src/out.log",
        "src/deep/x.rs",
    ] {
        fs::write(r.join(f), "x").unwrap();
    }
    t
}

#[test]
fn folders_first_natural_order_and_ignore_rules() {
    let t = repo();
    assert_eq!(
        names(t.path(), false),
        ["docs/", "src/", "a2.txt", "a10.txt", "B.txt", "README.md"]
    );
    // Rules from the root .gitignore apply below it.
    assert_eq!(names(&t.path().join("src"), false), ["deep/", "main.rs"]);
}

#[test]
fn show_hidden_includes_and_flags_hidden_and_ignored() {
    let t = repo();
    let l = list(t.path(), true, MAX_ENTRIES).unwrap();
    let flag = |n: &str| {
        let e = l.entries.iter().find(|e| e.name == n).unwrap();
        (e.hidden, e.ignored)
    };
    assert_eq!(flag(".env"), (true, false));
    assert_eq!(flag(".git"), (true, false));
    assert_eq!(flag("target"), (false, true));
    assert_eq!(flag("build"), (false, true));
    assert_eq!(flag("debug.log"), (false, true));
    assert_eq!(flag("src"), (false, false));
}

#[test]
fn ignore_rules_only_apply_in_a_work_tree() {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join(".gitignore"), "*.log\n").unwrap();
    fs::write(t.path().join("a.log"), "").unwrap();
    assert_eq!(names(t.path(), false), ["a.log"]);
}

#[test]
fn truncates_and_signature_tracks_names_not_contents() {
    let t = tempfile::tempdir().unwrap();
    for i in 0..10 {
        fs::write(t.path().join(format!("f{i}")), "").unwrap();
    }
    let l = list(t.path(), false, 4).unwrap();
    assert!(l.truncated);
    assert_eq!(l.entries.len(), 4);
    let before = signature(t.path()).unwrap();
    assert_eq!(before, l.signature);
    fs::write(t.path().join("f3"), "new contents").unwrap();
    assert_eq!(signature(t.path()).unwrap(), before);
    fs::rename(t.path().join("f3"), t.path().join("g3")).unwrap();
    assert_ne!(signature(t.path()).unwrap(), before);
    assert!(signature(&t.path().join("missing")).is_none());
}

#[test]
fn symlinked_folders_count_as_folders() {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("real")).unwrap();
    std::os::unix::fs::symlink(t.path().join("real"), t.path().join("link")).unwrap();
    let l = list(t.path(), false, MAX_ENTRIES).unwrap();
    let link = l.entries.iter().find(|e| e.name == "link").unwrap();
    assert!(link.dir && link.symlink);
}

type Changes = mpsc::Receiver<(u32, Vec<Change>, Instant)>;

fn watcher() -> (DirWatcher<u32>, Changes) {
    let (tx, rx) = mpsc::channel();
    let w = DirWatcher::new(Duration::from_millis(50), move |k, changes| {
        let _ = tx.send((k, changes, Instant::now()));
    })
    .unwrap();
    (w, rx)
}

/// Waits for a change to `dir`, skipping FSEvents' replay of earlier ones.
fn wait_for(rx: &Changes, dir: &Path, pred: impl Fn(&Change) -> bool) -> (u32, Change, Instant) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while let Ok((k, changes, at)) =
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
    {
        if let Some(c) = changes.into_iter().find(|c| c.dir == dir && pred(c)) {
            return (k, c, at);
        }
    }
    panic!("no change for {}", dir.display());
}

fn settle(rx: &Changes) {
    std::thread::sleep(Duration::from_millis(250));
    while rx.try_recv().is_ok() {}
}

#[test]
fn create_rename_delete_each_report_the_folder_quickly() {
    let t = tempfile::tempdir().unwrap();
    let dir: PathBuf = t.path().join("watched");
    fs::create_dir(&dir).unwrap();
    let (mut w, rx) = watcher();
    w.watch(7, &dir).unwrap();
    settle(&rx);

    for step in ["create", "rename", "delete"] {
        let start = Instant::now();
        match step {
            "create" => fs::write(dir.join("a.txt"), "").unwrap(),
            "rename" => fs::rename(dir.join("a.txt"), dir.join("b.txt")).unwrap(),
            _ => fs::remove_file(dir.join("b.txt")).unwrap(),
        }
        let (k, _change, at) = wait_for(&rx, &dir, |c| !c.contents_only);
        assert_eq!(k, 7);
        let took = at - start;
        assert!(took < Duration::from_millis(200), "{step} took {took:?}");
        settle(&rx);
    }
}

#[test]
fn ignore_file_edits_flag_rules_for_folders_below() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("r");
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(root.join(".gitignore"), "").unwrap();
    let (mut w, rx) = watcher();
    w.watch(1, &root).unwrap();
    w.watch(1, &root.join("sub")).unwrap();
    settle(&rx);
    fs::write(root.join(".gitignore"), "*.tmp\n").unwrap();
    let (_, c, _) = wait_for(&rx, &root.join("sub"), |c| c.rules);
    assert!(!c.contents_only);
}

#[test]
fn unwatch_stops_changes() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("d");
    fs::create_dir(&dir).unwrap();
    let (mut w, rx) = watcher();
    w.watch(3, &dir).unwrap();
    assert_eq!(w.watched(&3), vec![dir.clone()]);
    w.unwatch_all(&3);
    assert!(w.watched(&3).is_empty());
    settle(&rx);
    fs::write(dir.join("x"), "").unwrap();
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}
