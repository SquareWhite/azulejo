//! The watcher against the real file system: writes, editor-style atomic
//! saves and deletes each arrive as one debounced event within 200 ms.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use azulejo_preview::Watcher;

const DEBOUNCE: Duration = Duration::from_millis(50);
const BUDGET: Duration = Duration::from_millis(200);

struct Harness {
    _dir: tempfile::TempDir,
    file: PathBuf,
    watcher: Watcher<u32>,
    rx: mpsc::Receiver<(u32, PathBuf, Instant)>,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plan.md");
    std::fs::write(&file, "# one\n").unwrap();
    let (tx, rx) = mpsc::channel();
    let mut watcher = Watcher::new(DEBOUNCE, move |key, path| {
        let _ = tx.send((key, path, Instant::now()));
    })
    .unwrap();
    watcher.watch(7, &file).unwrap();
    // FSEvents needs a moment before the stream reports anything.
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}
    Harness {
        _dir: dir,
        file,
        watcher,
        rx,
    }
}

/// Waits for one event after `start`; returns its latency.
fn expect_event(h: &Harness, start: Instant) -> Duration {
    let (key, path, at) =
        h.rx.recv_timeout(Duration::from_secs(3))
            .expect("an event within 3 s");
    assert_eq!(key, 7);
    assert_eq!(path, h.file);
    at - start
}

fn assert_quiet(h: &Harness) {
    if let Ok(extra) = h.rx.recv_timeout(Duration::from_millis(250)) {
        panic!("expected one debounced event, got another: {extra:?}");
    }
}

#[test]
fn write_atomic_save_and_delete() {
    let h = harness();

    let start = Instant::now();
    std::fs::write(&h.file, "# two\n").unwrap();
    let write = expect_event(&h, start);
    assert_quiet(&h);

    // Save the way editors do: write a temp file, rename it over the original.
    let start = Instant::now();
    let tmp = h.file.with_extension("md.tmp~");
    std::fs::write(&tmp, "# three\n").unwrap();
    std::fs::rename(&tmp, &h.file).unwrap();
    let atomic = expect_event(&h, start);
    assert_quiet(&h);

    // The watch survives the rename: a plain write afterwards still reports.
    let start = Instant::now();
    std::fs::write(&h.file, "# four\n").unwrap();
    let after = expect_event(&h, start);
    assert_quiet(&h);

    let start = Instant::now();
    std::fs::remove_file(&h.file).unwrap();
    let delete = expect_event(&h, start);

    for (what, latency) in [
        ("write", write),
        ("atomic save", atomic),
        ("write after rename", after),
        ("delete", delete),
    ] {
        eprintln!("{what}: {latency:?}");
        assert!(
            latency <= BUDGET,
            "{what} took {latency:?}, budget {BUDGET:?}"
        );
    }
}

#[test]
fn bursts_are_debounced_and_other_files_ignored() {
    let h = harness();
    let start = Instant::now();
    for i in 0..10 {
        std::fs::write(&h.file, format!("# edit {i}\n")).unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    std::fs::write(h.file.with_file_name("other.md"), "x").unwrap();
    expect_event(&h, start);
    assert_quiet(&h);
}

#[test]
fn unwatch_and_shared_files() {
    let mut h = harness();
    // A second key on the same file.
    h.watcher.watch(8, &h.file).unwrap();
    std::fs::write(&h.file, "# shared\n").unwrap();
    let mut keys = vec![
        h.rx.recv_timeout(Duration::from_secs(3)).unwrap().0,
        h.rx.recv_timeout(Duration::from_secs(3)).unwrap().0,
    ];
    keys.sort_unstable();
    assert_eq!(keys, [7, 8]);

    h.watcher.unwatch(&7);
    h.watcher.unwatch(&8);
    assert_eq!(h.watcher.path(&7), None);
    std::fs::write(&h.file, "# unwatched\n").unwrap();
    assert!(h.rx.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn a_missing_file_can_be_watched_until_it_appears() {
    let h = harness();
    let mut h = h;
    let later = h.file.with_file_name("later.md");
    h.watcher.watch(7, &later).unwrap();
    std::fs::write(&later, "# here now\n").unwrap();
    let (key, path, _) = h.rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!((key, path), (7, later));
}
