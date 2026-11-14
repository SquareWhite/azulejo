//! Change notifications for watched files, debounced.
//!
//! Each file is watched through its folder, not by itself: editors often save
//! by writing a temporary file and renaming it over the original, which ends a
//! watch on the old file (inotify) but shows up as an event in the folder.

use std::collections::HashMap;
use std::hash::Hash;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

enum Msg {
    Fs(notify::Event),
    Stop,
}

struct Entry {
    /// What the caller asked to watch, handed back on change.
    original: PathBuf,
    /// The canonical folder plus the file name; what events are matched on.
    file: PathBuf,
    dir: PathBuf,
}

struct Watched<K> {
    keys: HashMap<K, Entry>,
    files: HashMap<PathBuf, Vec<K>>,
    dirs: HashMap<PathBuf, usize>,
}

/// Watches files under keys (e.g. tile IDs) and calls back with the key and
/// path once a file has been quiet for the debounce interval after a change,
/// creation, rename or deletion. Several keys may watch one file.
pub struct Watcher<K> {
    watched: Arc<Mutex<Watched<K>>>,
    notify: RecommendedWatcher,
    tx: mpsc::Sender<Msg>,
}

impl<K> Watcher<K>
where
    K: Clone + Eq + Hash + Send + 'static,
{
    pub fn new<F>(debounce: Duration, on_change: F) -> io::Result<Self>
    where
        F: Fn(K, PathBuf) + Send + 'static,
    {
        let watched = Arc::new(Mutex::new(Watched {
            keys: HashMap::new(),
            files: HashMap::new(),
            dirs: HashMap::new(),
        }));
        let (tx, rx) = mpsc::channel();
        let fs_tx = tx.clone();
        let notify = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                let _ = fs_tx.send(Msg::Fs(event));
            }
        })
        .map_err(io::Error::other)?;
        let state = watched.clone();
        thread::Builder::new()
            .name("azulejo-preview-watch".into())
            .spawn(move || debounce_loop(&rx, &state, debounce, on_change))?;
        Ok(Watcher {
            watched,
            notify,
            tx,
        })
    }

    /// Watches `path` under `key`, replacing what `key` watched before. The
    /// file needn't exist; its folder must.
    pub fn watch(&mut self, key: K, path: &Path) -> io::Result<()> {
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        let dir = parent.canonicalize()?;
        let file = dir.join(name);
        self.unwatch(&key);
        let mut w = self.watched.lock().expect("watch state");
        let count = w.dirs.entry(dir.clone()).or_insert(0);
        if *count == 0
            && let Err(e) = self.notify.watch(&dir, RecursiveMode::NonRecursive)
        {
            w.dirs.remove(&dir);
            return Err(io::Error::other(e));
        }
        *w.dirs.get_mut(&dir).expect("just inserted") += 1;
        w.files.entry(file.clone()).or_default().push(key.clone());
        w.keys.insert(
            key,
            Entry {
                original: path.to_path_buf(),
                file,
                dir,
            },
        );
        Ok(())
    }

    /// Stops watching for `key`; does nothing when it wasn't watching.
    pub fn unwatch(&mut self, key: &K) {
        let mut w = self.watched.lock().expect("watch state");
        let Some(entry) = w.keys.remove(key) else {
            return;
        };
        if let Some(keys) = w.files.get_mut(&entry.file) {
            keys.retain(|k| k != key);
            if keys.is_empty() {
                w.files.remove(&entry.file);
            }
        }
        if let Some(count) = w.dirs.get_mut(&entry.dir) {
            *count -= 1;
            if *count == 0 {
                w.dirs.remove(&entry.dir);
                let _ = self.notify.unwatch(&entry.dir);
            }
        }
    }

    /// The path `key` watches.
    pub fn path(&self, key: &K) -> Option<PathBuf> {
        let w = self.watched.lock().expect("watch state");
        w.keys.get(key).map(|e| e.original.clone())
    }
}

impl<K> Drop for Watcher<K> {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
    }
}

/// Reading a file isn't a change; finishing a write is.
fn relevant(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

fn debounce_loop<K, F>(
    rx: &mpsc::Receiver<Msg>,
    watched: &Mutex<Watched<K>>,
    debounce: Duration,
    on_change: F,
) where
    K: Clone + Eq + Hash,
    F: Fn(K, PathBuf),
{
    let mut pending: HashMap<K, Instant> = HashMap::new();
    loop {
        let next = pending.values().min().copied();
        let msg = match next {
            Some(deadline) => rx.recv_timeout(deadline.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match msg {
            Ok(Msg::Fs(event)) if relevant(&event.kind) || event.need_rescan() => {
                let due = Instant::now() + debounce;
                let w = watched.lock().expect("watch state");
                if event.need_rescan() {
                    for key in w.keys.keys() {
                        pending.insert(key.clone(), due);
                    }
                }
                for path in &event.paths {
                    for key in keys_for(&w, path) {
                        pending.insert(key, due);
                    }
                }
            }
            Ok(Msg::Fs(_)) | Err(RecvTimeoutError::Timeout) => {}
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        let ready: Vec<K> = pending
            .iter()
            .filter(|(_, due)| **due <= now)
            .map(|(k, _)| k.clone())
            .collect();
        if ready.is_empty() {
            continue;
        }
        let fire: Vec<(K, PathBuf)> = {
            let w = watched.lock().expect("watch state");
            ready
                .into_iter()
                .filter_map(|k| {
                    pending.remove(&k);
                    // Unwatched while waiting: drop it.
                    w.keys.get(&k).map(|e| (k, e.original.clone()))
                })
                .collect()
        };
        for (key, path) in fire {
            on_change(key, path);
        }
    }
}

/// The keys watching `path`, as an event reports it. Events usually carry
/// canonical paths; when they don't, the folder is canonicalised to match.
fn keys_for<K: Clone>(w: &Watched<K>, path: &Path) -> Vec<K> {
    if let Some(keys) = w.files.get(path) {
        return keys.clone();
    }
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Vec::new();
    };
    parent
        .canonicalize()
        .ok()
        .and_then(|dir| w.files.get(&dir.join(name)))
        .cloned()
        .unwrap_or_default()
}
