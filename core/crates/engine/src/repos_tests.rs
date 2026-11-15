//! Workspace branches (F26) against real temp repositories.

use std::path::Path;
use std::process::Command as Proc;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use azulejo_model::{PaneMeta, Tile, TileContent};
use azulejo_protocol::{Command, Event, WorkspaceSetRepo};

use super::*;
use crate::{Core, CoreOptions};

fn git(dir: &Path, args: &[&str]) {
    let ok = Proc::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?} failed in {}", dir.display());
}

/// A repository with one commit on `main`.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    git(dir.path(), &["add", "a.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);
    dir
}

fn canon(p: &Path) -> String {
    p.canonicalize().unwrap().display().to_string()
}

#[test]
fn reads_branch_detached_dirty_and_new_repositories() {
    let dir = repo();
    let top = find_repo(dir.path()).unwrap().0;
    let info = read_branch(&top).unwrap();
    assert_eq!(info.branch.as_deref(), Some("main"));
    assert!(!info.detached && !info.dirty);
    assert_eq!(info.repo, canon(dir.path()));

    // Tracked changes are dirty; untracked files aren't.
    std::fs::write(dir.path().join("new.txt"), "x").unwrap();
    assert!(!read_branch(&top).unwrap().dirty);
    std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
    assert!(read_branch(&top).unwrap().dirty);

    git(dir.path(), &["checkout", "-q", "--detach"]);
    let info = read_branch(&top).unwrap();
    assert!(info.detached);
    assert_eq!(info.branch.as_ref().map(String::len), Some(7));

    // A repository without commits still names its branch.
    let fresh = tempfile::tempdir().unwrap();
    git(fresh.path(), &["init", "-q"]);
    let top = find_repo(fresh.path()).unwrap().0;
    let info = read_branch(&top).unwrap();
    assert_eq!(info.branch.as_deref(), Some("main"));
    assert!(!info.detached);

    // Outside any repository there's nothing.
    let plain = tempfile::tempdir().unwrap();
    assert_eq!(find_repo(plain.path()), None);
}

#[test]
fn finds_the_folder_a_workspace_follows() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let mut session = Session::new();
    let (_, ws) = session.create_window("w", None);
    let mut term = Tile::new(TileContent::terminal());
    term.pane = Some(PaneMeta {
        cwd: Some(a.path().display().to_string()),
        ..PaneMeta::default()
    });
    session.create_tab(ws, term, None).unwrap();
    let w = session.workspace(ws).unwrap().clone();
    assert_eq!(repo_dir(&w).as_deref(), Some(a.path()));

    // An explicit folder wins.
    session
        .set_workspace_repo(ws, Some(b.path().display().to_string()))
        .unwrap();
    let w = session.workspace(ws).unwrap().clone();
    assert_eq!(repo_dir(&w).as_deref(), Some(b.path()));

    // A folder that doesn't exist gives nothing.
    session
        .set_workspace_repo(ws, Some("/no/such/folder".into()))
        .unwrap();
    assert_eq!(repo_dir(session.workspace(ws).unwrap()), None);
}

fn core() -> Core {
    Core::start(CoreOptions {
        state_path: None,
        api_socket: String::new(),
        log: Arc::new(|_| {}),
    })
}

/// The next `workspace.git` matching `pred`, with how long it took.
fn next_git(
    events: &mpsc::Receiver<Event>,
    pred: impl Fn(&Option<GitInfo>) -> bool,
) -> (Option<GitInfo>, Duration) {
    let start = Instant::now();
    let deadline = start + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(Event::WorkspaceGit { git, .. }) if pred(&git) => return (git, start.elapsed()),
            Ok(_) => {}
            Err(e) => panic!("no matching workspace.git: {e}"),
        }
    }
}

#[test]
fn branch_switches_reach_the_sidebar_quickly() {
    let dir = repo();
    let core = core();
    let events = core.subscribe();
    core.execute(Command::WorkspaceSetRepo(WorkspaceSetRepo {
        workspace: None,
        path: Some(dir.path().display().to_string()),
    }))
    .unwrap();
    // The git folder is watched before the first read starts, so once that
    // read arrives every later checkout is seen.
    let (got, _) = next_git(&events, |g| g.is_some());
    assert_eq!(got.unwrap().branch.as_deref(), Some("main"));

    let mut worst = Duration::ZERO;
    for branch in ["feature", "main", "other"] {
        let args: &[&str] = if branch == "main" {
            &["checkout", "-q", "main"]
        } else {
            &["checkout", "-q", "-b", branch]
        };
        git(dir.path(), args);
        // From the checkout finishing: how long `git checkout` itself takes
        // on a busy machine isn't the core's latency.
        let (got, took) = next_git(&events, |g| {
            g.as_ref().and_then(|g| g.branch.as_deref()) == Some(branch)
        });
        assert!(got.is_some());
        worst = worst.max(took);
    }
    eprintln!("branch switch → workspace.git: worst {worst:?}");
    // Typically ~50 ms (debounce plus one `git status`). A lost watch never
    // delivers at all; this bound catches a slow path while leaving room
    // for a loaded machine.
    assert!(worst < Duration::from_secs(1), "took {worst:?}");

    // The session carries it for shells that start later.
    let session = core.session().unwrap();
    let ws = &session.windows[0].workspaces[0];
    assert_eq!(
        ws.git.as_ref().and_then(|g| g.branch.as_deref()),
        Some("other")
    );

    // Pointing outside a repository clears it.
    let plain = tempfile::tempdir().unwrap();
    core.execute(Command::WorkspaceSetRepo(WorkspaceSetRepo {
        workspace: None,
        path: Some(plain.path().display().to_string()),
    }))
    .unwrap();
    let (got, _) = next_git(&events, |g| g.is_none());
    assert_eq!(got, None);
}

#[test]
fn status_parses_porcelain_v2() {
    let top = Path::new("/r");
    let s = parse_status(
        top,
        "# branch.oid 0123456789abcdef\n# branch.head main\n# branch.upstream origin/main\n",
    );
    assert_eq!(s.branch.as_deref(), Some("main"));
    assert!(!s.dirty && !s.detached);
    let s = parse_status(
        top,
        "# branch.oid 0123456789abcdef\n# branch.head (detached)\n1 .M N... 100644 100644 100644 a b f.txt\n",
    );
    assert_eq!(s.branch.as_deref(), Some("0123456"));
    assert!(s.detached && s.dirty);
}
