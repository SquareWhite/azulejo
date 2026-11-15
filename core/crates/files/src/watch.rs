//! Watches folders under keys, debounced.
//!
//! Each expanded folder is watched on its own, non-recursively, so a tile
//! with three folders open costs three watches however big the repository is.
//! A tree watch ([`DirWatcher::watch_tree`]) only says that something below
//! a folder changed, for git status: on macOS that's one FSEvents path
//! whatever the tree's size; elsewhere (inotify needs a watch per folder) it
//! covers the folder itself only.

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

/// Something changed in a watched folder, or anywhere below a watched tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The folder as the caller passed it to [`DirWatcher::watch`] or
    /// [`DirWatcher::watch_tree`].
    pub dir: PathBuf,
    /// An ignore file changed (`.gitignore`, `.ignore`) in this folder or
    /// above: what's visible may differ although no name did.
    pub rules: bool,
    /// Only file contents changed, as far as the events say: names are
    /// probably the same, but git status may not be.
    pub contents_only: bool,
}

enum Msg {
    Fs(notify::Event),
    Stop,
}

struct State<K> {
    /// Canonical folder → (key, folder as given).
    dirs: HashMap<PathBuf, Vec<(K, PathBuf)>>,
    /// Canonical tree root → (key, folder as given).
    trees: HashMap<PathBuf, Vec<(K, PathBuf)>>,
    /// How each canonical folder is registered with `notify`.
    registered: HashMap<PathBuf, RecursiveMode>,
}

/// How a tree registers with `notify`: recursive FSEvents costs one path
/// however big the tree is; recursive inotify costs a watch per folder.
const TREE_MODE: RecursiveMode = if cfg!(target_os = "macos") {
    RecursiveMode::Recursive
} else {
    RecursiveMode::NonRecursive
};

#[derive(Default, Clone, Copy)]
struct Pending {
    due: Option<Instant>,
    rules: bool,
    names: bool,
}

/// Watches folders under keys (e.g. tile IDs) and calls back with each key's
/// changed folders once they've been quiet for the debounce interval.
pub struct DirWatcher<K> {
    state: Arc<Mutex<State<K>>>,
    notify: RecommendedWatcher,
    tx: mpsc::Sender<Msg>,
}

impl<K> DirWatcher<K>
where
    K: Clone + Eq + Hash + Send + 'static,
{
    pub fn new<F>(debounce: Duration, on_change: F) -> io::Result<Self>
    where
        F: Fn(K, Vec<Change>) + Send + 'static,
    {
        let state = Arc::new(Mutex::new(State {
            dirs: HashMap::new(),
            trees: HashMap::new(),
            registered: HashMap::new(),
        }));
        let (tx, rx) = mpsc::channel();
        let fs_tx = tx.clone();
        let notify = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                let _ = fs_tx.send(Msg::Fs(event));
            }
        })
        .map_err(io::Error::other)?;
        let shared = state.clone();
        thread::Builder::new()
            .name("azulejo-files-watch".into())
            .spawn(move || debounce_loop(&rx, &shared, debounce, on_change))?;
        Ok(DirWatcher { state, notify, tx })
    }

    /// Watches `dir` under `key`. Watching the same folder twice under one
    /// key is a no-op.
    pub fn watch(&mut self, key: K, dir: &Path) -> io::Result<()> {
        self.add(false, key, dir)
    }

    /// Watches everything below `dir` under `key`, reporting any change in
    /// the tree as a [`Change`] to `dir` itself (see the module docs for
    /// platforms where that's `dir` only). [`unwatch_all`](Self::unwatch_all)
    /// stops it.
    pub fn watch_tree(&mut self, key: K, dir: &Path) -> io::Result<()> {
        self.add(true, key, dir)
    }

    fn add(&mut self, tree: bool, key: K, dir: &Path) -> io::Result<()> {
        let canon = dir.canonicalize()?;
        let mut s = self.state.lock().expect("watch state");
        let map = if tree { &mut s.trees } else { &mut s.dirs };
        let keys = map.entry(canon.clone()).or_default();
        if keys.iter().any(|(k, _)| *k == key) {
            return Ok(());
        }
        keys.push((key.clone(), dir.to_path_buf()));
        if let Err(e) = register(&mut self.notify, &mut s, &canon) {
            drop_key(&mut self.notify, &mut s, tree, &key, &canon);
            return Err(e);
        }
        Ok(())
    }

    /// Stops watching the folder `dir` for `key` (a tree watch on it stays).
    pub fn unwatch(&mut self, key: &K, dir: &Path) {
        let state = self.state.clone();
        let mut s = state.lock().expect("watch state");
        let canon = match dir.canonicalize() {
            Ok(c) => c,
            // Gone: find it by the path it was watched under.
            Err(_) => match s
                .dirs
                .iter()
                .find(|(_, keys)| keys.iter().any(|(k, d)| k == key && d == dir))
            {
                Some((c, _)) => c.clone(),
                None => return,
            },
        };
        drop_key(&mut self.notify, &mut s, false, key, &canon);
    }

    /// Stops every watch `key` has, trees included.
    pub fn unwatch_all(&mut self, key: &K) {
        let state = self.state.clone();
        let mut s = state.lock().expect("watch state");
        for tree in [false, true] {
            let map = if tree { &s.trees } else { &s.dirs };
            let dirs: Vec<PathBuf> = map
                .iter()
                .filter(|(_, keys)| keys.iter().any(|(k, _)| k == key))
                .map(|(c, _)| c.clone())
                .collect();
            for canon in dirs {
                drop_key(&mut self.notify, &mut s, tree, key, &canon);
            }
        }
    }

    /// The folders `key` watches, as given.
    pub fn watched(&self, key: &K) -> Vec<PathBuf> {
        let s = self.state.lock().expect("watch state");
        s.dirs
            .values()
            .flat_map(|keys| {
                keys.iter()
                    .filter(|(k, _)| k == key)
                    .map(|(_, d)| d.clone())
            })
            .collect()
    }
}

fn drop_key<K: PartialEq>(
    notify: &mut RecommendedWatcher,
    s: &mut State<K>,
    tree: bool,
    key: &K,
    canon: &Path,
) {
    let map = if tree { &mut s.trees } else { &mut s.dirs };
    let Some(keys) = map.get_mut(canon) else {
        return;
    };
    keys.retain(|(k, _)| k != key);
    if keys.is_empty() {
        map.remove(canon);
        // Narrowing or dropping a registration can't fail in a way worth
        // reporting: at worst notify had already forgotten the folder.
        let _ = register(notify, s, canon);
    }
}

/// Registers `canon` with `notify` as its watches need: recursively while
/// any key watches it as a tree, else on its own, else not at all.
fn register<K>(notify: &mut RecommendedWatcher, s: &mut State<K>, canon: &Path) -> io::Result<()> {
    let want = if s.trees.contains_key(canon) {
        Some(TREE_MODE)
    } else if s.dirs.contains_key(canon) {
        Some(RecursiveMode::NonRecursive)
    } else {
        None
    };
    let have = s.registered.get(canon).copied();
    if want == have {
        return Ok(());
    }
    if have.is_some() {
        s.registered.remove(canon);
        let _ = notify.unwatch(canon);
    }
    if let Some(mode) = want {
        notify.watch(canon, mode).map_err(io::Error::other)?;
        s.registered.insert(canon.to_path_buf(), mode);
    }
    Ok(())
}

impl<K> Drop for DirWatcher<K> {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
    }
}

fn is_rules_file(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some(".gitignore" | ".ignore")
    )
}

/// Data changed inside a file, nothing about names. Anything we can't tell
/// counts as a name change.
fn contents_only(kind: &EventKind) -> bool {
    use notify::event::ModifyKind;
    matches!(
        kind,
        EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_))
            | EventKind::Access(AccessKind::Close(AccessMode::Write))
    )
}

fn debounce_loop<K, F>(
    rx: &mpsc::Receiver<Msg>,
    state: &Mutex<State<K>>,
    debounce: Duration,
    on_change: F,
) where
    K: Clone + Eq + Hash,
    F: Fn(K, Vec<Change>),
{
    let mut pending: HashMap<(K, PathBuf), Pending> = HashMap::new();
    loop {
        let next = pending.values().filter_map(|p| p.due).min();
        let msg = match next {
            Some(deadline) => rx.recv_timeout(deadline.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match msg {
            Ok(Msg::Fs(event)) => {
                let reading = matches!(event.kind, EventKind::Access(_))
                    && !matches!(
                        event.kind,
                        EventKind::Access(AccessKind::Close(AccessMode::Write))
                    );
                if !reading || event.need_rescan() {
                    let due = Instant::now() + debounce;
                    let s = state.lock().expect("watch state");
                    let names = !contents_only(&event.kind) || event.need_rescan();
                    let mut hit = |key: &K, dir: &PathBuf, rules: bool| {
                        let p = pending.entry((key.clone(), dir.clone())).or_default();
                        p.due = Some(due);
                        p.rules |= rules;
                        p.names |= names;
                    };
                    if event.need_rescan() {
                        for keys in s.dirs.values().chain(s.trees.values()) {
                            for (k, d) in keys {
                                hit(k, d, true);
                            }
                        }
                    }
                    for path in &event.paths {
                        let rules = is_rules_file(path);
                        // The entry's folder, and the entry itself when it's
                        // a watched folder (deleted or renamed).
                        let parent = path.parent().map(Path::to_path_buf);
                        for dir in parent.iter().chain(std::iter::once(path)) {
                            for (k, d) in lookup(&s, dir) {
                                hit(&k, &d, rules);
                            }
                        }
                        // Anything below a tree is a change to the tree.
                        for (canon, keys) in &s.trees {
                            if path.starts_with(canon) {
                                for (k, d) in keys {
                                    hit(k, d, false);
                                }
                            }
                        }
                        // New ignore rules affect watched folders below too.
                        if rules && let Some(parent) = &parent {
                            for (canon, keys) in &s.dirs {
                                if canon != parent && canon.starts_with(parent) {
                                    for (k, d) in keys {
                                        hit(k, d, true);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        let ready: Vec<(K, PathBuf)> = pending
            .iter()
            .filter(|(_, p)| p.due.is_some_and(|d| d <= now))
            .map(|(k, _)| k.clone())
            .collect();
        if ready.is_empty() {
            continue;
        }
        let mut by_key: Vec<(K, Vec<Change>)> = Vec::new();
        for (key, dir) in ready {
            let p = pending
                .remove(&(key.clone(), dir.clone()))
                .unwrap_or_default();
            let change = Change {
                dir,
                rules: p.rules,
                contents_only: !p.names && !p.rules,
            };
            match by_key.iter_mut().find(|(k, _)| *k == key) {
                Some((_, changes)) => changes.push(change),
                None => by_key.push((key, vec![change])),
            }
        }
        for (key, changes) in by_key {
            on_change(key, changes);
        }
    }
}

/// Keys watching `dir`, as an event reports it. Events usually carry
/// canonical paths; when they don't, the folder is canonicalised to match.
fn lookup<K: Clone>(s: &State<K>, dir: &Path) -> Vec<(K, PathBuf)> {
    if let Some(keys) = s.dirs.get(dir) {
        return keys.clone();
    }
    dir.canonicalize()
        .ok()
        .and_then(|c| s.dirs.get(&c))
        .cloned()
        .unwrap_or_default()
}
