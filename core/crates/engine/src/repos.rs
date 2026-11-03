//! The git branch each workspace shows in the sidebar (F26).
//!
//! Which folder: the workspace's own `repo` when the user set one, else the
//! working directory of its focused terminal (then any terminal in it), else
//! a files tile's root, else the workspace's `cwd`. The core finds that
//! folder's repository once (`rev-parse`), watches the repository's git
//! folder (HEAD, index, refs written there) and reads the branch with one
//! `git status` per burst of changes, never taking git's locks. Commands
//! finishing in the workspace's terminals also trigger a read, since they're
//! what usually changes files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use azulejo_files::DirWatcher;
use azulejo_model::{GitInfo, Session, TileContent, Workspace, WorkspaceId};

use crate::driver::LogFn;
use crate::files::expand_home;
use crate::link::Internal;

/// How long the git folder must be quiet before the branch is read again.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(40);

/// Work finished off the engine's thread.
pub(crate) enum Delivery {
    /// The repository holding `dir`: its work tree and git folder, or `None`
    /// outside a repository.
    Found {
        workspace: WorkspaceId,
        dir: PathBuf,
        repo: Option<(PathBuf, PathBuf)>,
    },
    /// What `git status` said about `top`.
    Read {
        workspace: WorkspaceId,
        top: PathBuf,
        git: Option<GitInfo>,
    },
    /// The git folder of the workspace's repository changed.
    Changed(WorkspaceId),
}

pub(crate) type Deliver = Arc<dyn Fn(Delivery) + Send + Sync>;

#[derive(Default)]
struct State {
    /// The folder the workspace follows now.
    dir: Option<PathBuf>,
    /// Its repository: work tree and git folder.
    repo: Option<(PathBuf, PathBuf)>,
    /// What the workspace's terminals run, to notice commands finishing.
    activity: u64,
    finding: bool,
    reading: bool,
    read_again: bool,
}

pub(crate) struct Repos {
    watcher: Option<DirWatcher<WorkspaceId>>,
    deliver: Deliver,
    log: LogFn,
    workspaces: HashMap<WorkspaceId, State>,
}

impl Repos {
    pub(crate) fn new(deliver: Deliver, log: LogFn) -> Self {
        let watcher = {
            let deliver = deliver.clone();
            DirWatcher::new(DEBOUNCE, move |workspace, _| {
                deliver(Delivery::Changed(workspace));
            })
        };
        let watcher = match watcher {
            Ok(w) => Some(w),
            Err(e) => {
                log(format!(
                    "file watching unavailable ({e}); branches won't update live"
                ));
                None
            }
        };
        Repos {
            watcher,
            deliver,
            log,
            workspaces: HashMap::new(),
        }
    }

    /// Follows the session: a workspace whose folder changed looks for its
    /// repository again; one whose terminals finished a command reads its
    /// branch again; closed workspaces stop watching.
    pub(crate) fn sync(&mut self, session: &Session) {
        let wanted: HashMap<WorkspaceId, (Option<PathBuf>, u64)> = session
            .windows
            .iter()
            .flat_map(|w| &w.workspaces)
            .map(|ws| (ws.id, (repo_dir(ws), activity(ws))))
            .collect();
        let gone: Vec<WorkspaceId> = self
            .workspaces
            .keys()
            .filter(|id| !wanted.contains_key(id))
            .copied()
            .collect();
        for id in gone {
            self.workspaces.remove(&id);
            if let Some(w) = &mut self.watcher {
                w.unwatch_all(&id);
            }
        }
        for (id, (dir, activity)) in wanted {
            let st = self.workspaces.entry(id).or_default();
            let busy = st.activity != activity;
            st.activity = activity;
            if st.dir == dir {
                if busy {
                    self.read(id);
                }
                continue;
            }
            st.dir = dir.clone();
            match dir {
                // Still inside the same work tree: no need to look again.
                Some(d) if st.repo.as_ref().is_some_and(|(top, _)| d.starts_with(top)) => {}
                Some(d) => self.find(id, d),
                None => {
                    st.repo = None;
                    if let Some(w) = &mut self.watcher {
                        w.unwatch_all(&id);
                    }
                    (self.deliver)(Delivery::Read {
                        workspace: id,
                        top: PathBuf::new(),
                        git: None,
                    });
                }
            }
        }
    }

    /// Finished work; what to apply to the session.
    pub(crate) fn accept(&mut self, delivery: Delivery) -> Option<Internal> {
        match delivery {
            Delivery::Found {
                workspace,
                dir,
                repo,
            } => {
                let st = self.workspaces.get_mut(&workspace)?;
                st.finding = false;
                if st.dir.as_deref() != Some(dir.as_path()) {
                    // Moved on meanwhile: look for the current folder instead.
                    if let Some(now) = st.dir.clone() {
                        self.find(workspace, now);
                    }
                    return None;
                }
                let old_git_dir = st.repo.as_ref().map(|(_, g)| g.clone());
                st.repo = repo.clone();
                if let Some(w) = &mut self.watcher {
                    let new_git_dir = repo.as_ref().map(|(_, g)| g.clone());
                    if old_git_dir != new_git_dir {
                        w.unwatch_all(&workspace);
                        if let Some(g) = &new_git_dir
                            && let Err(e) = w.watch(workspace, g)
                        {
                            (self.log)(format!("can't watch {} ({e})", g.display()));
                        }
                    }
                }
                match repo {
                    Some(_) => {
                        self.read(workspace);
                        None
                    }
                    None => Some(Internal::WorkspaceGit {
                        workspace,
                        git: None,
                    }),
                }
            }
            Delivery::Read {
                workspace,
                top,
                git,
            } => {
                let st = self.workspaces.get_mut(&workspace)?;
                let current = st.repo.as_ref().map(|(t, _)| t.clone());
                // No repository any more: say so (sent by `sync`).
                if top.as_os_str().is_empty() {
                    return (current.is_none())
                        .then_some(Internal::WorkspaceGit { workspace, git });
                }
                st.reading = false;
                if st.read_again {
                    st.read_again = false;
                    self.read(workspace);
                }
                (current.as_deref() == Some(top.as_path()))
                    .then_some(Internal::WorkspaceGit { workspace, git })
            }
            Delivery::Changed(workspace) => {
                self.read(workspace);
                None
            }
        }
    }

    /// Looks for the repository holding `dir`, off this thread.
    fn find(&mut self, workspace: WorkspaceId, dir: PathBuf) {
        let Some(st) = self.workspaces.get_mut(&workspace) else {
            return;
        };
        if st.finding {
            // `accept` looks again for whatever `dir` is by then.
            return;
        }
        st.finding = true;
        let deliver = self.deliver.clone();
        let spawned = std::thread::Builder::new()
            .name("azulejo-repo-find".into())
            .spawn(move || {
                let repo = find_repo(&dir);
                deliver(Delivery::Found {
                    workspace,
                    dir,
                    repo,
                });
            });
        if spawned.is_err() {
            st.finding = false;
        }
    }

    /// Reads the branch of the workspace's repository, off this thread; at
    /// most one read at a time, plus one more if asked meanwhile.
    fn read(&mut self, workspace: WorkspaceId) {
        let Some(st) = self.workspaces.get_mut(&workspace) else {
            return;
        };
        let Some((top, _)) = st.repo.clone() else {
            return;
        };
        if st.reading {
            st.read_again = true;
            return;
        }
        st.reading = true;
        let deliver = self.deliver.clone();
        let spawned = std::thread::Builder::new()
            .name("azulejo-repo-read".into())
            .spawn(move || {
                let git = read_branch(&top);
                deliver(Delivery::Read {
                    workspace,
                    top,
                    git,
                });
            });
        if spawned.is_err() {
            st.reading = false;
        }
    }
}

/// The folder whose repository `ws` shows; see the module docs.
///
/// Remote workspaces (ADR 0011) have none: their paths are on another
/// machine, and reading git there over the machine's SSH link isn't built
/// yet (PLAN 2.14 follow-up, Deferred checks). Without this, a remote cwd
/// that happens to exist locally would show the wrong branch.
pub(crate) fn repo_dir(ws: &Workspace) -> Option<PathBuf> {
    if ws.machine.is_some() {
        return None;
    }
    let existing = |p: &str| {
        let p = expand_home(p);
        p.is_dir().then_some(p)
    };
    if let Some(repo) = ws.repo.as_deref() {
        return existing(repo);
    }
    let terminal_dir = |tile: &azulejo_model::Tile| match &tile.content {
        TileContent::Terminal { cwd, .. } => tile
            .pane
            .as_ref()
            .and_then(|m| m.cwd.clone())
            .or_else(|| cwd.clone()),
        _ => None,
    };
    let active = ws
        .active_tab
        .and_then(|id| ws.tabs.iter().find(|t| t.id == id));
    let focused = active
        .and_then(|tab| tab.tiles.iter().find(|t| t.id == tab.focused))
        .and_then(terminal_dir);
    let any_terminal = || {
        active
            .into_iter()
            .chain(ws.tabs.iter())
            .flat_map(|tab| &tab.tiles)
            .find_map(terminal_dir)
    };
    let files_root = || {
        ws.tabs
            .iter()
            .flat_map(|tab| &tab.tiles)
            .find_map(|t| match &t.content {
                TileContent::Files { root: Some(r) } => Some(r.clone()),
                _ => None,
            })
    };
    focused
        .or_else(any_terminal)
        .or_else(files_root)
        .or_else(|| ws.cwd.clone())
        .and_then(|d| existing(&d))
}

/// A number that changes when what the workspace's terminals run changes.
fn activity(ws: &Workspace) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for tile in ws.tabs.iter().flat_map(|t| &t.tiles) {
        if let Some(meta) = &tile.pane {
            tile.id.hash(&mut h);
            meta.command.hash(&mut h);
            meta.title.hash(&mut h);
        }
    }
    h.finish()
}

/// The work tree and git folder of the repository holding `dir`.
pub(crate) fn find_repo(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel", "--absolute-git-dir"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let top = PathBuf::from(lines.next()?.trim());
    let git_dir = PathBuf::from(lines.next()?.trim());
    let canon = |p: PathBuf| p.canonicalize().unwrap_or(p);
    Some((canon(top), canon(git_dir)))
}

/// The branch (or detached commit) and whether tracked files changed, from
/// one `git status`. Untracked files aren't looked at: that's the expensive
/// part in a big repository, and they don't make a branch "dirty" here.
pub(crate) fn read_branch(top: &Path) -> Option<GitInfo> {
    let out = Command::new("git")
        .arg("-C")
        .arg(top)
        .args([
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=no",
            "--ignore-submodules=dirty",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(parse_status(top, &String::from_utf8_lossy(&out.stdout)))
}

fn parse_status(top: &Path, text: &str) -> GitInfo {
    let mut head = None;
    let mut oid = None;
    let mut dirty = false;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("# branch.head ") {
            head = Some(v.trim().to_owned());
        } else if let Some(v) = line.strip_prefix("# branch.oid ") {
            oid = Some(v.trim().to_owned());
        } else if !line.starts_with('#') && !line.is_empty() {
            dirty = true;
        }
    }
    let detached = head.as_deref() == Some("(detached)");
    let branch = if detached {
        oid.filter(|o| o != "(initial)")
            .map(|o| o.chars().take(7).collect())
    } else {
        head
    };
    GitInfo {
        repo: top.display().to_string(),
        branch,
        detached,
        dirty,
    }
}

#[cfg(test)]
#[path = "repos_tests.rs"]
mod tests;
