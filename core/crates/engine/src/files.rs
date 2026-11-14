//! File browser tiles in a running core (F10): list folders off the engine's
//! thread, watch the folders each tile has open, and keep git status for
//! decorations.
//!
//! What a tile has open, and whether it shows hidden entries, lives here and
//! isn't saved: after a relaunch a tile shows its root, and shells expand
//! again what they remember.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use azulejo_files::{Change, DirWatcher, GitState, GitStatus};
use azulejo_model::{Session, TileContent, TileId};
use azulejo_protocol::{
    Command, Created, Error, ErrorCode, Event, FileEntry, FileGitState, FilesChanged, FilesGit,
    FilesListing, FilesRevealed, Reply,
};

use crate::driver::LogFn;
use crate::{Engine, Outcome};

/// How long a folder must be quiet after a change before shells hear of it.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(40);

/// Work finished off the engine's thread, handed back to it.
pub(crate) enum Delivery {
    /// Watched folders changed; each with its names' signature now (`None`
    /// when it can't be read any more).
    Changed(TileId, Vec<(Change, Option<u64>)>),
    /// A folder was listed; its signature then.
    Listed(TileId, PathBuf, u64),
    /// `git status` finished for a tile's root.
    Git(TileId, PathBuf, Option<GitStatus>),
}

pub(crate) type Deliver = Arc<dyn Fn(Delivery) + Send + Sync>;

/// A command answered off the engine's thread.
pub(crate) type Job = Box<dyn FnOnce() -> Result<Outcome, Error> + Send>;

pub(crate) enum Handled {
    Done(Box<Result<Outcome, Error>>),
    Later(Job),
}

struct TileState {
    /// The root as the session stores it (to notice changes).
    configured: PathBuf,
    /// Resolved: what listings and events use.
    root: PathBuf,
    show_hidden: bool,
    expanded: BTreeSet<PathBuf>,
    signatures: HashMap<PathBuf, u64>,
    git: Option<Arc<GitStatus>>,
    git_running: bool,
    git_again: bool,
}

pub(crate) struct Files {
    watcher: Option<DirWatcher<TileId>>,
    deliver: Deliver,
    log: LogFn,
    tiles: HashMap<TileId, TileState>,
    /// Files panels (`files.panel`): trees outside the layout, such as the
    /// sidebar's Files view. They live in `tiles` too but not in the session,
    /// so `sync` leaves them alone.
    panels: HashSet<TileId>,
}

/// Commands this module answers (the rest of `files.*` change the session
/// and run in the engine).
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
                    "file watching unavailable ({e}); file tiles won't update live"
                ));
                None
            }
        };
        Files {
            watcher,
            deliver,
            log,
            tiles: HashMap::new(),
            panels: HashSet::new(),
        }
    }

    /// Whether `tile` names a files panel rather than a tile.
    pub(crate) fn is_panel(&self, tile: Option<TileId>) -> bool {
        tile.is_some_and(|t| self.panels.contains(&t))
    }

    /// Matches state to the session's files tiles: new tiles and new roots
    /// start fresh (root open and watched), closed tiles stop watching.
    pub(crate) fn sync(&mut self, session: &Session) {
        let wanted: HashMap<TileId, PathBuf> = session
            .tiles()
            .filter_map(|(_, tile)| match &tile.content {
                TileContent::Files { root } => Some((tile.id, configured_root(root.as_deref()))),
                _ => None,
            })
            .collect();
        let stale: Vec<TileId> = self
            .tiles
            .iter()
            .filter(|(id, st)| !self.panels.contains(id) && wanted.get(id) != Some(&st.configured))
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            self.stop(id);
        }
        for (id, configured) in wanted {
            if !self.tiles.contains_key(&id) {
                self.start(id, configured);
            }
        }
    }

    /// Starts showing `configured` for `id`: its root open and watched.
    fn start(&mut self, id: TileId, configured: PathBuf) {
        let root = configured
            .canonicalize()
            .unwrap_or_else(|_| configured.clone());
        self.tiles.insert(
            id,
            TileState {
                configured,
                root: root.clone(),
                show_hidden: false,
                expanded: BTreeSet::new(),
                signatures: HashMap::new(),
                git: None,
                git_running: false,
                git_again: false,
            },
        );
        self.watch(id, &root);
        self.refresh_git(id);
    }

    /// Forgets `id` and stops watching its folders.
    fn stop(&mut self, id: TileId) {
        self.tiles.remove(&id);
        if let Some(w) = &mut self.watcher {
            w.unwatch_all(&id);
        }
    }

    /// The files tile or panel a command is for: a panel by its id, else
    /// the tile the engine resolves (the focused one when absent).
    fn target(&self, engine: &Engine, tile: Option<TileId>) -> Result<TileId, Error> {
        match tile {
            Some(id) if self.panels.contains(&id) => Ok(id),
            _ => Ok(engine.files_root(tile)?.0),
        }
    }

    /// Runs a runtime `files.*` command for the tile `engine` resolves.
    pub(crate) fn handle(&mut self, engine: &Engine, command: Command) -> Handled {
        match self.handle_inner(engine, command) {
            Ok(h) => h,
            Err(e) => Handled::Done(Box::new(Err(e))),
        }
    }

    fn handle_inner(&mut self, engine: &Engine, command: Command) -> Result<Handled, Error> {
        match command {
            Command::FilesList(p) => {
                let tile = self.target(engine, p.tile)?;
                let dir = self.resolve(tile, p.dir.as_deref())?;
                Ok(Handled::Later(self.list_job(tile, dir)?))
            }
            Command::FilesExpand(p) => {
                let tile = self.target(engine, p.tile)?;
                let dir = self.resolve(tile, p.dir.as_deref())?;
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
                let tile = self.target(engine, p.tile)?;
                let dir = self.resolve_lenient(tile, p.dir.as_deref())?;
                self.collapse(tile, &dir);
                Ok(done(Reply::Ok, Vec::new()))
            }
            Command::FilesReveal(p) => {
                let tile = self.target(engine, p.tile)?;
                let path = self.resolve(tile, Some(&p.path))?;
                let st = self.state(tile)?;
                let root = st.root.clone();
                if !path.starts_with(&root) {
                    return Err(Error::new(
                        ErrorCode::InvalidParams,
                        format!(
                            "{} isn't under the tile's root {}; use files.set_root",
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
                let tile = self.target(engine, p.tile)?;
                let st = self.state_mut(tile)?;
                st.show_hidden = p.show.unwrap_or(!st.show_hidden);
                let dirs = st.expanded.iter().map(|d| display(d)).collect();
                let event = Event::FilesChanged(FilesChanged { tile, dirs });
                Ok(done(Reply::Ok, vec![event]))
            }
            Command::FilesPanel(p) => {
                let configured = configured_root(Some(&p.root));
                if !configured.is_dir() {
                    return Err(Error::new(
                        ErrorCode::InvalidParams,
                        format!("{} isn't a folder", configured.display()),
                    ));
                }
                let id = p.panel.unwrap_or_else(TileId::new);
                let same = self.panels.contains(&id)
                    && self
                        .tiles
                        .get(&id)
                        .is_some_and(|st| st.configured == configured);
                if !same {
                    self.stop(id);
                    self.panels.insert(id);
                    self.start(id, configured);
                }
                let created = Created {
                    tile: Some(id),
                    ..Created::default()
                };
                Ok(done(Reply::Created(created), Vec::new()))
            }
            Command::FilesPanelClose(p) => {
                if self.panels.remove(&p.panel) {
                    self.stop(p.panel);
                }
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
        }
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
        let root = st.root.clone();
        let deliver = self.deliver.clone();
        let spawned = std::thread::Builder::new()
            .name("azulejo-files-git".into())
            .spawn(move || {
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
        let again = std::mem::take(&mut st.git_again);
        let changed = st.git.as_deref() != status.as_ref();
        let first_git_dir = status
            .as_ref()
            .filter(|_| st.git.is_none())
            .map(|s| s.work_tree.join(".git"))
            .filter(|d| d.is_dir());
        st.git = status.map(Arc::new);
        let event = if changed {
            st.git.as_ref().map(|g| Event::FilesGit(git_event(tile, g)))
        } else {
            None
        };
        // Commits and staging change status without touching the work tree.
        if let (Some(dir), Some(w)) = (first_git_dir, &mut self.watcher)
            && let Err(e) = w.watch(tile, &dir)
        {
            (self.log)(format!("can't watch {} ({e})", dir.display()));
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
                }),
                events: Vec::new(),
            })
        }))
    }

    fn watch(&mut self, tile: TileId, dir: &Path) {
        let Some(st) = self.tiles.get_mut(&tile) else {
            return;
        };
        if !st.expanded.insert(dir.to_path_buf()) {
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
            if let Some(w) = &mut self.watcher {
                w.unwatch(&tile, &d);
            }
        }
    }

    fn state(&self, tile: TileId) -> Result<&TileState, Error> {
        self.tiles.get(&tile).ok_or_else(|| not_ready(tile))
    }

    fn state_mut(&mut self, tile: TileId) -> Result<&mut TileState, Error> {
        self.tiles.get_mut(&tile).ok_or_else(|| not_ready(tile))
    }

    /// An existing path for `tile`: absolute, or relative to its root;
    /// resolved (symlinks followed) so it matches listings and events.
    fn resolve(&self, tile: TileId, path: Option<&str>) -> Result<PathBuf, Error> {
        let joined = self.join(tile, path)?;
        joined
            .canonicalize()
            .map_err(|e| Error::new(ErrorCode::NotFound, format!("{}: {e}", joined.display())))
    }

    /// Like `resolve`, but a path that's gone still names what was open.
    fn resolve_lenient(&self, tile: TileId, path: Option<&str>) -> Result<PathBuf, Error> {
        let joined = self.join(tile, path)?;
        Ok(joined.canonicalize().unwrap_or(joined))
    }

    fn join(&self, tile: TileId, path: Option<&str>) -> Result<PathBuf, Error> {
        let root = &self.state(tile)?.root;
        Ok(match path {
            None | Some("") => root.clone(),
            Some(p) => {
                let p = expand_home(p);
                if p.is_absolute() { p } else { root.join(p) }
            }
        })
    }
}

fn done(reply: Reply, events: Vec<Event>) -> Handled {
    Handled::Done(Box::new(Ok(Outcome { reply, events })))
}

fn not_ready(tile: TileId) -> Error {
    Error::new(
        ErrorCode::InvalidState,
        format!("{tile} isn't a files tile the core is showing"),
    )
}

/// The folder a files tile shows: its root, or the home folder.
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
