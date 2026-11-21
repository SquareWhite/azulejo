//! Files panels (`files.panel`, the sidebar's Files view) in a running core:
//! list folders off the engine's thread, watch the folders each panel has
//! open, and keep git status for decorations. Once a panel's root turns out
//! to be in a work tree, the whole root is watched too (macOS), so an edit in
//! a closed folder still updates that folder's badge.
//!
//! Panels live outside the session. What a panel has open, and whether it
//! shows hidden entries, lives here and isn't saved: shells open their
//! panels again after a relaunch and expand again what they remember.
//!
//! A files panel can show a remote herdr machine's files instead.
//! Its folders are listed over the machine's SSH connection
//! ([`crate::remote_files`]) and kept in a cache by machine and folder: a
//! listing answers at once from the cache, or empty with `loading`, and
//! `files.changed` follows when the remote listing is in. Open folders are
//! listed again every [`REMOTE_POLL`]; there's no watching and no git status.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use azulejo_files::{Change, DirWatcher, GitState, GitStatus};
use azulejo_herdr::RemoteMachine;
use azulejo_model::TileId;
use azulejo_protocol::{
    Command, Created, Error, ErrorCode, Event, FileEntry, FileGitState, FilesChanged, FilesGit,
    FilesListing, FilesOpen, FilesOpenTarget, FilesRevealed, Reply,
};

use crate::Outcome;
use crate::driver::LogFn;
use crate::remote_files::{self, RemoteDir, RunError};

/// How often a remote panel lists its open folders again.
pub(crate) const REMOTE_POLL: Duration = Duration::from_secs(3);

/// A cached remote listing older than this is listed again when a shell asks
/// for it; the shell gets the cached one meanwhile.
const REMOTE_FRESH: Duration = Duration::from_secs(2);

/// After a remote listing failed, polls wait this long before trying again.
const REMOTE_RETRY: Duration = Duration::from_secs(10);

/// Most remote folders the cache keeps.
const REMOTE_CACHE: usize = 512;

/// How long a folder must be quiet after a change before shells hear of it.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(40);

/// The least time between two `git status` runs for one panel. The whole
/// work tree is watched, so a build writing into an ignored `target/` would
/// otherwise keep `git status` running back to back.
const GIT_GAP: Duration = Duration::from_millis(500);

/// Work finished off the engine's thread, handed back to it.
pub(crate) enum Delivery {
    /// Watched folders changed; each with its names' signature now (`None`
    /// when it can't be read any more).
    Changed(TileId, Vec<(Change, Option<u64>)>),
    /// A folder was listed; its signature then.
    Listed(TileId, PathBuf, u64),
    /// `git status` finished for a tile's root.
    Git(TileId, PathBuf, Option<GitStatus>),
    /// A remote listing finished (or failed) for a panel; `fetch` numbers
    /// the run.
    Remote {
        tile: TileId,
        fetch: u64,
        dirs: Vec<PathBuf>,
        result: remote_files::Listed,
    },
    /// Time for a remote panel (in its `generation`) to list its open
    /// folders again.
    Poll(TileId, u64),
}

pub(crate) type Deliver = Arc<dyn Fn(Delivery) + Send + Sync>;

/// A command answered off the engine's thread.
pub(crate) type Job = Box<dyn FnOnce() -> Result<Outcome, Error> + Send>;

pub(crate) enum Handled {
    Done(Box<Result<Outcome, Error>>),
    Later(Job),
}

struct PanelState {
    /// The root as `files.panel` named it (to notice changes).
    configured: PathBuf,
    /// Resolved: what listings and events use.
    root: PathBuf,
    show_hidden: bool,
    expanded: BTreeSet<PathBuf>,
    signatures: HashMap<PathBuf, u64>,
    git: Option<Arc<GitStatus>>,
    git_running: bool,
    git_again: bool,
    /// When the last `git status` started (or will, after `GIT_GAP`).
    git_started: Option<Instant>,
    /// A panel on a remote machine.
    remote: Option<RemoteState>,
}

impl PanelState {
    fn machine(&self) -> Option<&str> {
        self.remote.as_ref().map(|r| r.machine.as_str())
    }
}

/// What a remote panel is fetching and what went wrong.
struct RemoteState {
    /// herdr's machine ID.
    machine: String,
    /// Tells this panel's polls from an earlier root's.
    generation: u64,
    /// Cleared when the panel stops: its poller ends.
    alive: Arc<AtomicBool>,
    /// Running listings by fetch number, each with its cancel flag.
    running: HashMap<u64, Arc<AtomicBool>>,
    /// Folders a running listing covers.
    fetching: HashSet<PathBuf>,
    /// Folders whose last listing failed, and why.
    errors: HashMap<PathBuf, String>,
    /// When a listing last failed as a whole (unreachable, timed out).
    failed_at: Option<Instant>,
}

pub(crate) struct Files {
    watcher: Option<DirWatcher<TileId>>,
    deliver: Deliver,
    log: LogFn,
    /// Files panels (`files.panel`) by id: trees outside the layout, such
    /// as the sidebar's Files view.
    tiles: HashMap<TileId, PanelState>,
    /// herdr's connected remote machines, by ID, for remote panels.
    machines: HashMap<String, RemoteMachine>,
    /// Remote listings by machine and folder, sorted, with when they came.
    cache: HashMap<(String, PathBuf), (Instant, Arc<RemoteDir>)>,
    /// Numbers remote fetches and panel generations.
    counter: u64,
}

/// What `files.open` from a remote panel turns into.
pub(crate) enum RemoteOpen {
    /// A command for the engine, with a path it can use.
    Forward(FilesOpen),
    /// Answered here (an error).
    Done(Box<Result<Outcome, Error>>),
}

/// Commands this module answers (`files.open` changes the session and runs
/// in the engine).
pub(crate) fn is_runtime(command: &Command) -> bool {
    matches!(
        command,
        Command::FilesList(_)
            | Command::FilesExpand(_)
            | Command::FilesCollapse(_)
            | Command::FilesReveal(_)
            | Command::FilesToggleHidden(_)
            | Command::FilesPanel(_)
            | Command::FilesPanelClose(_)
    )
}

impl Files {
    pub(crate) fn new(deliver: Deliver, log: LogFn) -> Self {
        let watcher = {
            let deliver = deliver.clone();
            DirWatcher::new(DEBOUNCE, move |tile, changes: Vec<Change>| {
                let changes = changes
                    .into_iter()
                    .map(|c| {
                        let sig = azulejo_files::signature(&c.dir);
                        (c, sig)
                    })
                    .collect();
                deliver(Delivery::Changed(tile, changes));
            })
        };
        let watcher = match watcher {
            Ok(w) => Some(w),
            Err(e) => {
                log(format!(
                    "file watching unavailable ({e}); files panels won't update live"
                ));
                None
            }
        };
        Files {
            watcher,
            deliver,
            log,
            tiles: HashMap::new(),
            machines: HashMap::new(),
            cache: HashMap::new(),
            counter: 0,
        }
    }

    /// Whether `tile` names a files panel rather than a tile.
    pub(crate) fn is_panel(&self, tile: Option<TileId>) -> bool {
        tile.is_some_and(|t| self.tiles.contains_key(&t))
    }

    /// Whether `tile` names a files panel on a remote machine.
    pub(crate) fn is_remote_panel(&self, tile: Option<TileId>) -> bool {
        tile.and_then(|t| self.tiles.get(&t))
            .is_some_and(|st| st.remote.is_some())
    }

    /// A remote machine remote panels can list (`Some`, once it's
    /// connected) or can't any more (`None`). Panels on a machine that
    /// comes back list their open folders again at once.
    pub(crate) fn set_machine(&mut self, id: String, machine: Option<RemoteMachine>) -> Vec<Event> {
        let back = machine.is_some();
        match machine {
            Some(m) => self.machines.insert(id.clone(), m),
            None => self.machines.remove(&id),
        };
        if !back {
            return Vec::new();
        }
        let tiles: Vec<TileId> = self
            .tiles
            .iter()
            .filter(|(_, st)| st.machine() == Some(id.as_str()))
            .map(|(t, _)| *t)
            .collect();
        let mut events = Vec::new();
        for tile in tiles {
            let dirs = match self.tiles.get_mut(&tile) {
                Some(st) => {
                    if let Some(r) = st.remote.as_mut() {
                        r.failed_at = None;
                    }
                    st.expanded.iter().cloned().collect()
                }
                None => continue,
            };
            let failed = self.fetch(tile, dirs);
            events.extend(changed_event(tile, failed));
        }
        events
    }

    /// Starts showing `configured` for `id`: its root open and watched.
    fn start(&mut self, id: TileId, configured: PathBuf) {
        let root = configured
            .canonicalize()
            .unwrap_or_else(|_| configured.clone());
        self.tiles.insert(
            id,
            PanelState {
                configured,
                root: root.clone(),
                show_hidden: false,
                expanded: BTreeSet::new(),
                signatures: HashMap::new(),
                git: None,
                git_running: false,
                git_again: false,
                git_started: None,
                remote: None,
            },
        );
        self.watch(id, &root);
        self.refresh_git(id);
    }

    /// Starts showing `root` on remote `machine` for panel `id`: its root
    /// open, listed now and every [`REMOTE_POLL`] after.
    fn start_remote(&mut self, id: TileId, root: PathBuf, machine: String) {
        self.counter += 1;
        let generation = self.counter;
        let alive = Arc::new(AtomicBool::new(true));
        {
            let alive = alive.clone();
            let deliver = self.deliver.clone();
            let _ = std::thread::Builder::new()
                .name("azulejo-files-poll".into())
                .spawn(move || {
                    loop {
                        std::thread::sleep(REMOTE_POLL);
                        if !alive.load(Ordering::Relaxed) {
                            return;
                        }
                        deliver(Delivery::Poll(id, generation));
                    }
                });
        }
        self.tiles.insert(
            id,
            PanelState {
                configured: root.clone(),
                root: root.clone(),
                show_hidden: false,
                expanded: BTreeSet::from([root.clone()]),
                signatures: HashMap::new(),
                git: None,
                git_running: false,
                git_again: false,
                git_started: None,
                remote: Some(RemoteState {
                    machine,
                    generation,
                    alive,
                    running: HashMap::new(),
                    fetching: HashSet::new(),
                    errors: HashMap::new(),
                    failed_at: None,
                }),
            },
        );
        self.fetch(id, vec![root]);
    }

    /// Forgets `id` and stops watching its folders; a remote panel's
    /// listings still running are cancelled.
    fn stop(&mut self, id: TileId) {
        if let Some(st) = self.tiles.remove(&id)
            && let Some(r) = st.remote
        {
            r.alive.store(false, Ordering::Relaxed);
            for cancel in r.running.values() {
                cancel.store(true, Ordering::Relaxed);
            }
        }
        if let Some(w) = &mut self.watcher {
            w.unwatch_all(&id);
        }
    }

    /// The panel a command is for.
    fn target(&self, tile: Option<TileId>) -> Result<TileId, Error> {
        match tile {
            Some(id) if self.tiles.contains_key(&id) => Ok(id),
            Some(id) => Err(not_ready(id)),
            None => Err(Error::new(
                ErrorCode::InvalidParams,
                "name a files panel (`tile`); open one with files.panel",
            )),
        }
    }

    /// Runs a runtime `files.*` command.
    pub(crate) fn handle(&mut self, command: Command) -> Handled {
        match self.handle_inner(command) {
            Ok(h) => h,
            Err(e) => Handled::Done(Box::new(Err(e))),
        }
    }

    fn handle_inner(&mut self, command: Command) -> Result<Handled, Error> {
        match command {
            Command::FilesList(p) => {
                let tile = self.target(p.tile)?;
                let dir = self.resolve(tile, p.dir.as_deref())?;
                if self.is_remote_panel(Some(tile)) {
                    let listing = self.remote_listing(tile, dir)?;
                    return Ok(done(Reply::Files(listing), Vec::new()));
                }
                Ok(Handled::Later(self.list_job(tile, dir)?))
            }
            Command::FilesExpand(p) => {
                let tile = self.target(p.tile)?;
                let dir = self.resolve(tile, p.dir.as_deref())?;
                if self.is_remote_panel(Some(tile)) {
                    self.watch(tile, &dir);
                    let listing = self.remote_listing(tile, dir)?;
                    return Ok(done(Reply::Files(listing), Vec::new()));
                }
                if !dir.is_dir() {
                    return Err(Error::new(
                        ErrorCode::InvalidParams,
                        format!("{} isn't a folder", dir.display()),
                    ));
                }
                self.watch(tile, &dir);
                Ok(Handled::Later(self.list_job(tile, dir)?))
            }
            Command::FilesCollapse(p) => {
                let tile = self.target(p.tile)?;
                let dir = self.resolve_lenient(tile, p.dir.as_deref())?;
                self.collapse(tile, &dir);
                Ok(done(Reply::Ok, Vec::new()))
            }
            Command::FilesReveal(p) => {
                let tile = self.target(p.tile)?;
                let path = self.resolve(tile, Some(&p.path))?;
                let st = self.state(tile)?;
                let root = st.root.clone();
                if !path.starts_with(&root) {
                    return Err(Error::new(
                        ErrorCode::InvalidParams,
                        format!(
                            "{} isn't under the panel's root {}; point the panel there with files.panel",
                            path.display(),
                            root.display()
                        ),
                    ));
                }
                let mut dirs: Vec<PathBuf> = path
                    .ancestors()
                    .skip(1)
                    .take_while(|d| d.starts_with(&root))
                    .map(Path::to_path_buf)
                    .collect();
                dirs.reverse();
                for d in &dirs {
                    self.watch(tile, d);
                }
                let event = Event::FilesRevealed(FilesRevealed {
                    tile,
                    path: display(&path),
                    dirs: dirs.iter().map(|d| display(d)).collect(),
                });
                Ok(done(Reply::Ok, vec![event]))
            }
            Command::FilesToggleHidden(p) => {
                let tile = self.target(p.tile)?;
                let st = self.state_mut(tile)?;
                st.show_hidden = p.show.unwrap_or(!st.show_hidden);
                let dirs = st.expanded.iter().map(|d| display(d)).collect();
                let event = Event::FilesChanged(FilesChanged { tile, dirs });
                Ok(done(Reply::Ok, vec![event]))
            }
            Command::FilesPanel(p) => {
                let machine = p
                    .machine
                    .filter(|m| !m.is_empty() && m != azulejo_protocol::LOCAL_MACHINE);
                let configured = match &machine {
                    // Remote: nothing to check here; a bad root shows as
                    // the listing's error.
                    Some(_) => {
                        let root = PathBuf::from(&p.root);
                        if !root.is_absolute() {
                            return Err(Error::new(
                                ErrorCode::InvalidParams,
                                format!("{} isn't an absolute path", p.root),
                            ));
                        }
                        remote_files::normalize(&root)
                    }
                    None => configured_root(Some(&p.root)),
                };
                if machine.is_none() && !configured.is_dir() {
                    return Err(Error::new(
                        ErrorCode::InvalidParams,
                        format!("{} isn't a folder", configured.display()),
                    ));
                }
                let id = p.panel.unwrap_or_else(TileId::new);
                let same = self.tiles.get(&id).is_some_and(|st| {
                    st.configured == configured && st.machine() == machine.as_deref()
                });
                if !same {
                    self.stop(id);
                    match machine {
                        Some(machine) => self.start_remote(id, configured, machine),
                        None => self.start(id, configured),
                    }
                }
                let created = Created {
                    tile: Some(id),
                    ..Created::default()
                };
                Ok(done(Reply::Created(created), Vec::new()))
            }
            Command::FilesPanelClose(p) => {
                self.stop(p.panel);
                Ok(done(Reply::Ok, Vec::new()))
            }
            other => Err(Error::new(
                ErrorCode::Internal,
                format!("{} isn't a runtime files command", other.method()),
            )),
        }
    }

    /// Something finished off the engine's thread; the events to publish.
    pub(crate) fn accept(&mut self, delivery: Delivery) -> Vec<Event> {
        match delivery {
            Delivery::Listed(tile, dir, sig) => {
                if let Some(st) = self.tiles.get_mut(&tile)
                    && st.expanded.contains(&dir)
                {
                    st.signatures.insert(dir, sig);
                }
                Vec::new()
            }
            Delivery::Changed(tile, changes) => self.changed(tile, changes),
            Delivery::Git(tile, root, status) => self.git_done(tile, &root, status),
            Delivery::Remote {
                tile,
                fetch,
                dirs,
                result,
            } => self.remote_done(tile, fetch, dirs, result),
            Delivery::Poll(tile, generation) => self.poll(tile, generation),
        }
    }

    // MARK: Remote panels

    /// Lists `dirs` of remote panel `tile` off this thread, skipping those
    /// a running listing already covers. Returns the folders that failed at
    /// once (the machine isn't connected), whose error changed.
    fn fetch(&mut self, tile: TileId, dirs: Vec<PathBuf>) -> Vec<PathBuf> {
        let Some(r) = self.tiles.get_mut(&tile).and_then(|st| st.remote.as_mut()) else {
            return Vec::new();
        };
        let dirs: Vec<PathBuf> = dirs
            .into_iter()
            .filter(|d| !r.fetching.contains(d))
            .collect();
        if dirs.is_empty() {
            return Vec::new();
        }
        let Some(machine) = self.machines.get(&r.machine) else {
            let why = "the machine isn't connected".to_owned();
            return dirs
                .into_iter()
                .filter(|d| r.errors.insert(d.clone(), why.clone()).as_ref() != Some(&why))
                .collect();
        };
        self.counter += 1;
        let fetch = self.counter;
        let cancel = Arc::new(AtomicBool::new(false));
        let command = machine.exec(remote_files::list_script(&dirs, remote_files::MAX_ENTRIES));
        let deliver = self.deliver.clone();
        let spawned = std::thread::Builder::new()
            .name("azulejo-files-remote".into())
            .spawn({
                let cancel = cancel.clone();
                let dirs = dirs.clone();
                move || {
                    let result = remote_files::run(&command, &cancel, remote_files::TIMEOUT)
                        .map(|out| remote_files::parse_listing(&out, &dirs));
                    deliver(Delivery::Remote {
                        tile,
                        fetch,
                        dirs,
                        result,
                    });
                }
            });
        if spawned.is_err() {
            let why = "couldn't start a thread to list the folder".to_owned();
            for d in dirs {
                r.errors.insert(d, why.clone());
            }
            return Vec::new();
        }
        r.running.insert(fetch, cancel);
        r.fetching.extend(dirs);
        Vec::new()
    }

    /// A remote listing came back: cache it, note errors, and name the open
    /// folders whose listing or error changed.
    fn remote_done(
        &mut self,
        tile: TileId,
        fetch: u64,
        dirs: Vec<PathBuf>,
        result: remote_files::Listed,
    ) -> Vec<Event> {
        let Some(st) = self.tiles.get_mut(&tile) else {
            return Vec::new();
        };
        let Some(r) = st.remote.as_mut() else {
            return Vec::new();
        };
        // Cancelled, or from before the panel moved on.
        if r.running.remove(&fetch).is_none() {
            return Vec::new();
        }
        for d in &dirs {
            r.fetching.remove(d);
        }
        let now = Instant::now();
        let mut changed = Vec::new();
        match result {
            Err(RunError::Cancelled) => {}
            Err(e) => {
                let label = self
                    .machines
                    .get(&r.machine)
                    .map_or_else(|| "the machine".to_owned(), |m| m.profile.label.clone());
                let why = e.message(&label);
                r.failed_at = Some(now);
                for d in dirs {
                    if r.errors.insert(d.clone(), why.clone()).as_ref() != Some(&why) {
                        changed.push(d);
                    }
                }
            }
            Ok(results) => {
                r.failed_at = None;
                for (d, listing) in results {
                    match listing {
                        Ok(mut listing) => {
                            sort_remote(&mut listing);
                            let key = (r.machine.clone(), d.clone());
                            let had_error = r.errors.remove(&d).is_some();
                            let same = self
                                .cache
                                .get(&key)
                                .is_some_and(|(_, old)| **old == listing);
                            self.cache.insert(key, (now, Arc::new(listing)));
                            if !same || had_error {
                                changed.push(d);
                            }
                        }
                        Err(why) => {
                            if r.errors.insert(d.clone(), why.clone()).as_ref() != Some(&why) {
                                changed.push(d);
                            }
                        }
                    }
                }
            }
        }
        changed.retain(|d| st.expanded.contains(d));
        self.trim_cache();
        changed_event(tile, changed).into_iter().collect()
    }

    /// A remote panel's poll: list its open folders again, unless a listing
    /// still runs or the last one failed a moment ago.
    fn poll(&mut self, tile: TileId, generation: u64) -> Vec<Event> {
        let Some(st) = self.tiles.get(&tile) else {
            return Vec::new();
        };
        let Some(r) = &st.remote else {
            return Vec::new();
        };
        if r.generation != generation
            || !r.running.is_empty()
            || r.failed_at.is_some_and(|t| t.elapsed() < REMOTE_RETRY)
        {
            return Vec::new();
        }
        let dirs = st.expanded.iter().cloned().collect();
        let failed = self.fetch(tile, dirs);
        changed_event(tile, failed).into_iter().collect()
    }

    /// A remote folder's listing for a shell: from the cache (listed again
    /// in the background when it's old), else empty and `loading`.
    fn remote_listing(&mut self, tile: TileId, dir: PathBuf) -> Result<FilesListing, Error> {
        let st = self.state(tile)?;
        let Some(r) = &st.remote else {
            return Err(not_ready(tile));
        };
        let key = (r.machine.clone(), dir.clone());
        let cached = self.cache.get(&key).cloned();
        let stale = cached
            .as_ref()
            .is_none_or(|(at, _)| at.elapsed() > REMOTE_FRESH);
        // A folder that failed with nothing cached isn't retried on every
        // read (that would loop with `files.changed`); polls retry it.
        if stale && (cached.is_some() || !r.errors.contains_key(&dir)) {
            self.fetch(tile, vec![dir.clone()]);
        }
        let st = self.state(tile)?;
        let Some(r) = &st.remote else {
            return Err(not_ready(tile));
        };
        let show_hidden = st.show_hidden;
        let (entries, truncated) = match &cached {
            Some((_, listing)) => (
                listing
                    .entries
                    .iter()
                    .filter(|e| show_hidden || !e.name.starts_with('.'))
                    .map(|e| FileEntry {
                        path: display(&dir.join(&e.name)),
                        name: e.name.clone(),
                        dir: e.dir,
                        symlink: e.symlink,
                        hidden: e.name.starts_with('.'),
                        ignored: false,
                        git: None,
                    })
                    .collect(),
                listing.truncated,
            ),
            None => (Vec::new(), false),
        };
        Ok(FilesListing {
            tile,
            root: display(&st.root),
            dir: display(&dir),
            show_hidden,
            entries,
            truncated,
            machine: Some(r.machine.clone()),
            loading: cached.is_none() && r.fetching.contains(&dir),
            error: r.errors.get(&dir).cloned(),
        })
    }

    /// Drops the oldest remote listings past [`REMOTE_CACHE`].
    fn trim_cache(&mut self) {
        if self.cache.len() <= REMOTE_CACHE {
            return;
        }
        let mut ages: Vec<(Instant, (String, PathBuf))> = self
            .cache
            .iter()
            .map(|(k, (at, _))| (*at, k.clone()))
            .collect();
        ages.sort_by_key(|(at, _)| *at);
        for (_, key) in ages.into_iter().take(self.cache.len() - REMOTE_CACHE / 2) {
            self.cache.remove(&key);
        }
    }

    /// Whether a remote panel's listings say `path` is a folder.
    fn remote_is_dir(&self, tile: TileId, path: &Path) -> Option<bool> {
        let st = self.tiles.get(&tile)?;
        let machine = st.machine()?;
        if path == st.root || st.expanded.contains(path) {
            return Some(true);
        }
        let parent = path.parent()?;
        let name = path.file_name()?.to_string_lossy();
        let (_, listing) = self
            .cache
            .get(&(machine.to_owned(), parent.to_path_buf()))?;
        listing
            .entries
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.dir)
    }

    /// `files.open` from a remote panel: the editor or a terminal opens
    /// beside the focused tile, on the focused workspace's machine, with
    /// the path as the panel's listings know it.
    pub(crate) fn remote_open(&mut self, p: FilesOpen) -> RemoteOpen {
        let Some(panel) = p.tile else {
            return RemoteOpen::Forward(p);
        };
        let path = match self.resolve(panel, Some(&p.path)) {
            Ok(path) => path,
            Err(e) => return RemoteOpen::Done(Box::new(Err(e))),
        };
        let is_dir = self.remote_is_dir(panel, &path);
        // A remote path can't be checked on this Mac: say here what it is.
        let target = match (p.target, is_dir) {
            (FilesOpenTarget::Editor, Some(true)) => FilesOpenTarget::Terminal,
            (target, _) => target,
        };
        let path = match (target, is_dir) {
            (FilesOpenTarget::Terminal, Some(false)) => {
                path.parent().map_or(path.clone(), Path::to_path_buf)
            }
            _ => path,
        };
        RemoteOpen::Forward(FilesOpen {
            tile: None,
            path: display(&path),
            target,
        })
    }

    fn changed(&mut self, tile: TileId, changes: Vec<(Change, Option<u64>)>) -> Vec<Event> {
        let Some(st) = self.tiles.get_mut(&tile) else {
            return Vec::new();
        };
        let git_dir = st.git.as_ref().map(|g| g.work_tree.join(".git"));
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut gone: Vec<PathBuf> = Vec::new();
        for (change, sig) in &changes {
            if git_dir.as_deref() == Some(change.dir.as_path()) {
                continue;
            }
            let names_changed = match sig {
                None => {
                    gone.push(change.dir.clone());
                    true
                }
                Some(sig) => st.signatures.insert(change.dir.clone(), *sig) != Some(*sig),
            };
            if (names_changed || change.rules) && st.expanded.contains(&change.dir) {
                dirs.push(change.dir.clone());
            }
        }
        for d in gone {
            self.collapse(tile, &d);
        }
        // Any change, contents included, can change git status.
        self.refresh_git(tile);
        if dirs.is_empty() {
            return Vec::new();
        }
        dirs.sort();
        dirs.dedup();
        vec![Event::FilesChanged(FilesChanged {
            tile,
            dirs: dirs.iter().map(|d| display(d)).collect(),
        })]
    }

    fn refresh_git(&mut self, tile: TileId) {
        let Some(st) = self.tiles.get_mut(&tile) else {
            return;
        };
        if st.git_running {
            st.git_again = true;
            return;
        }
        st.git_running = true;
        let now = Instant::now();
        let wait = st.git_started.map_or(Duration::ZERO, |t| {
            (t + GIT_GAP).saturating_duration_since(now)
        });
        st.git_started = Some(now + wait);
        let root = st.root.clone();
        let deliver = self.deliver.clone();
        let spawned = std::thread::Builder::new()
            .name("azulejo-files-git".into())
            .spawn(move || {
                std::thread::sleep(wait);
                let status = azulejo_files::git_status(&root);
                deliver(Delivery::Git(tile, root, status));
            });
        if spawned.is_err() {
            st.git_running = false;
        }
    }

    fn git_done(&mut self, tile: TileId, root: &Path, status: Option<GitStatus>) -> Vec<Event> {
        let Some(st) = self.tiles.get_mut(&tile) else {
            return Vec::new();
        };
        if st.root != root {
            return Vec::new();
        }
        st.git_running = false;
        let mut again = std::mem::take(&mut st.git_again);
        let changed = st.git.as_deref() != status.as_ref();
        let first = st.git.is_none() && status.is_some();
        let git_dir = status
            .as_ref()
            .map(|s| s.work_tree.join(".git"))
            .filter(|d| first && d.is_dir());
        st.git = status.map(Arc::new);
        let event = if changed {
            st.git.as_ref().map(|g| Event::FilesGit(git_event(tile, g)))
        } else {
            None
        };
        if first && let Some(w) = &mut self.watcher {
            // Edits in folders that aren't open change status too.
            if let Err(e) = w.watch_tree(tile, root) {
                (self.log)(format!("can't watch {} ({e})", root.display()));
            }
            // Commits and staging change status without touching the work tree.
            if let Some(dir) = git_dir
                && let Err(e) = w.watch(tile, &dir)
            {
                (self.log)(format!("can't watch {} ({e})", dir.display()));
            }
            // What changed between that read and the watches starting went
            // unseen: read once more.
            again = true;
        }
        if again {
            self.refresh_git(tile);
        }
        event.into_iter().collect()
    }

    fn list_job(&self, tile: TileId, dir: PathBuf) -> Result<Job, Error> {
        let st = self.state(tile)?;
        let show_hidden = st.show_hidden;
        let root = st.root.clone();
        let git = st.git.clone();
        let deliver = self.deliver.clone();
        Ok(Box::new(move || {
            if let Some(delay) = list_delay() {
                std::thread::sleep(delay);
            }
            let listing = azulejo_files::list(&dir, show_hidden, azulejo_files::MAX_ENTRIES)
                .map_err(|e| {
                    Error::new(
                        ErrorCode::NotFound,
                        format!("can't list {}: {e}", dir.display()),
                    )
                })?;
            deliver(Delivery::Listed(tile, dir.clone(), listing.signature));
            let entries = listing
                .entries
                .into_iter()
                .map(|e| FileEntry {
                    git: git
                        .as_ref()
                        .and_then(|g| g.paths.get(&e.path))
                        .map(|s| git_state(*s)),
                    name: e.name,
                    path: display(&e.path),
                    dir: e.dir,
                    symlink: e.symlink,
                    hidden: e.hidden,
                    ignored: e.ignored,
                })
                .collect();
            Ok(Outcome {
                reply: Reply::Files(FilesListing {
                    tile,
                    root: display(&root),
                    dir: display(&dir),
                    show_hidden,
                    entries,
                    truncated: listing.truncated,
                    machine: None,
                    loading: false,
                    error: None,
                }),
                events: Vec::new(),
            })
        }))
    }

    fn watch(&mut self, tile: TileId, dir: &Path) {
        let Some(st) = self.tiles.get_mut(&tile) else {
            return;
        };
        if !st.expanded.insert(dir.to_path_buf()) || st.remote.is_some() {
            return;
        }
        if let Some(w) = &mut self.watcher
            && let Err(e) = w.watch(tile, dir)
        {
            (self.log)(format!(
                "can't watch {} for {tile} ({e}); it won't update live",
                dir.display()
            ));
        }
    }

    /// Closes `dir` and every open folder below it. The root stays open.
    fn collapse(&mut self, tile: TileId, dir: &Path) {
        let Some(st) = self.tiles.get_mut(&tile) else {
            return;
        };
        let closing: Vec<PathBuf> = st
            .expanded
            .iter()
            .filter(|d| d.starts_with(dir) && **d != st.root)
            .cloned()
            .collect();
        for d in closing {
            st.expanded.remove(&d);
            st.signatures.remove(&d);
            if st.remote.is_none()
                && let Some(w) = &mut self.watcher
            {
                w.unwatch(&tile, &d);
            }
        }
    }

    fn state(&self, tile: TileId) -> Result<&PanelState, Error> {
        self.tiles.get(&tile).ok_or_else(|| not_ready(tile))
    }

    fn state_mut(&mut self, tile: TileId) -> Result<&mut PanelState, Error> {
        self.tiles.get_mut(&tile).ok_or_else(|| not_ready(tile))
    }

    /// An existing path for `tile`: absolute, or relative to its root;
    /// resolved (symlinks followed) so it matches listings and events.
    /// A remote panel's paths are worked out by name instead.
    fn resolve(&self, tile: TileId, path: Option<&str>) -> Result<PathBuf, Error> {
        let joined = self.join(tile, path)?;
        if self.is_remote_panel(Some(tile)) {
            return Ok(remote_files::normalize(&joined));
        }
        joined
            .canonicalize()
            .map_err(|e| Error::new(ErrorCode::NotFound, format!("{}: {e}", joined.display())))
    }

    /// Like `resolve`, but a path that's gone still names what was open.
    fn resolve_lenient(&self, tile: TileId, path: Option<&str>) -> Result<PathBuf, Error> {
        let joined = self.join(tile, path)?;
        if self.is_remote_panel(Some(tile)) {
            return Ok(remote_files::normalize(&joined));
        }
        Ok(joined.canonicalize().unwrap_or(joined))
    }

    /// `path` made absolute against a files panel's root; `path` unchanged
    /// when the core isn't showing `tile`.
    pub(crate) fn absolute(&self, tile: TileId, path: &str) -> String {
        self.join(tile, Some(path))
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.to_owned())
    }

    fn join(&self, tile: TileId, path: Option<&str>) -> Result<PathBuf, Error> {
        let st = self.state(tile)?;
        let root = &st.root;
        Ok(match path {
            None | Some("") => root.clone(),
            Some(p) => {
                // `~` is this machine's home, not a remote one's.
                let p = if st.remote.is_some() {
                    PathBuf::from(p)
                } else {
                    expand_home(p)
                };
                if p.is_absolute() { p } else { root.join(p) }
            }
        })
    }
}

/// Test hook: `AZJ_FILES_LIST_DELAY_MS` holds every listing back that long,
/// standing in for a slow folder (a network volume, a permission prompt) so
/// the shell's automation can check that nothing waits for one.
fn list_delay() -> Option<Duration> {
    static DELAY: OnceLock<Option<Duration>> = OnceLock::new();
    *DELAY.get_or_init(|| {
        std::env::var("AZJ_FILES_LIST_DELAY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|ms| *ms > 0)
            .map(Duration::from_millis)
    })
}

fn done(reply: Reply, events: Vec<Event>) -> Handled {
    Handled::Done(Box::new(Ok(Outcome { reply, events })))
}

/// `files.changed` for `dirs` of `tile`, unless there are none.
fn changed_event(tile: TileId, mut dirs: Vec<PathBuf>) -> Option<Event> {
    if dirs.is_empty() {
        return None;
    }
    dirs.sort();
    dirs.dedup();
    Some(Event::FilesChanged(FilesChanged {
        tile,
        dirs: dirs.iter().map(|d| display(d)).collect(),
    }))
}

/// A remote listing in a listing's order: folders first, then natural name
/// order.
fn sort_remote(listing: &mut RemoteDir) {
    listing.entries.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| azulejo_files::natural_cmp(&a.name, &b.name))
    });
}

fn not_ready(tile: TileId) -> Error {
    Error::new(
        ErrorCode::InvalidState,
        "that isn't a files panel the core is showing",
    )
    .with_data(serde_json::json!({ "tile": tile }))
}

/// The folder a files panel shows: its root, or the home folder.
pub(crate) fn configured_root(root: Option<&str>) -> PathBuf {
    match root {
        Some(r) if !r.is_empty() => expand_home(r),
        _ => home(),
    }
}

pub(crate) fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return home();
    }
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(path),
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn display(p: &Path) -> String {
    p.display().to_string()
}

fn git_state(s: GitState) -> FileGitState {
    match s {
        GitState::Modified => FileGitState::Modified,
        GitState::Added => FileGitState::Added,
        GitState::Deleted => FileGitState::Deleted,
        GitState::Renamed => FileGitState::Renamed,
        GitState::Untracked => FileGitState::Untracked,
        GitState::Conflicted => FileGitState::Conflicted,
    }
}

fn git_event(tile: TileId, g: &GitStatus) -> FilesGit {
    FilesGit {
        tile,
        work_tree: display(&g.work_tree),
        paths: g
            .paths
            .iter()
            .map(|(p, s)| (display(p), git_state(*s)))
            .collect(),
    }
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
