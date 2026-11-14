//! Git status for decorations, from one `git status` run per change burst.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A path's state in the work tree, simplified for a badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GitState {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl GitState {
    fn from_porcelain(x: u8, y: u8) -> GitState {
        match (x, y) {
            (b'?', b'?') => GitState::Untracked,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => GitState::Conflicted,
            (b'R', _) | (_, b'R') => GitState::Renamed,
            (b'A', _) => GitState::Added,
            (b'D', _) | (_, b'D') => GitState::Deleted,
            _ => GitState::Modified,
        }
    }

    /// How a folder shows changes beneath it: the strongest state wins.
    fn rank(self) -> u8 {
        match self {
            GitState::Conflicted => 5,
            GitState::Modified | GitState::Renamed | GitState::Deleted => 4,
            GitState::Added => 3,
            GitState::Untracked => 2,
        }
    }
}

/// Changed paths in a work tree, absolute, plus every folder above them up
/// to the work tree root (so a collapsed folder can show that it holds
/// changes).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
    pub work_tree: PathBuf,
    pub paths: HashMap<PathBuf, GitState>,
}

/// Runs `git status` for the work tree holding `dir`. `None` when `dir`
/// isn't in a work tree or git isn't installed.
pub fn status(dir: &Path) -> Option<GitStatus> {
    let top = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let work_tree = PathBuf::from(String::from_utf8_lossy(&top.stdout).trim_end());
    let work_tree = work_tree.canonicalize().unwrap_or(work_tree);
    let out = Command::new("git")
        .arg("-C")
        .arg(&work_tree)
        // No locks: a status for decorations shouldn't block the user's git.
        .args([
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=normal",
            "--ignore-submodules=dirty",
        ])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(parse(&work_tree, &out.stdout))
}

fn parse(work_tree: &Path, porcelain: &[u8]) -> GitStatus {
    let mut paths: HashMap<PathBuf, GitState> = HashMap::new();
    let mut records = porcelain.split(|b| *b == 0).filter(|r| !r.is_empty());
    while let Some(rec) = records.next() {
        if rec.len() < 4 {
            continue;
        }
        let state = GitState::from_porcelain(rec[0], rec[1]);
        // A rename's record is followed by the original path; skip it.
        if matches!(rec[0], b'R' | b'C') {
            records.next();
        }
        let rel = String::from_utf8_lossy(&rec[3..]);
        let rel = rel.trim_end_matches('/');
        let path = work_tree.join(rel);
        mark(&mut paths, path.clone(), state);
        let mut up = path.parent();
        while let Some(dir) = up {
            if !dir.starts_with(work_tree) || dir == work_tree {
                break;
            }
            mark(&mut paths, dir.to_path_buf(), state);
            up = dir.parent();
        }
    }
    GitStatus {
        work_tree: work_tree.to_path_buf(),
        paths,
    }
}

fn mark(paths: &mut HashMap<PathBuf, GitState>, path: PathBuf, state: GitState) {
    paths
        .entry(path)
        .and_modify(|s| {
            if state.rank() > s.rank() {
                *s = state;
            }
        })
        .or_insert(state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_and_marks_folders() {
        let out = b" M src/a.rs\0?? notes/\0R  new.rs\0old.rs\0UU src/deep/c.rs\0";
        let s = parse(Path::new("/w"), out);
        assert_eq!(s.paths[Path::new("/w/src/a.rs")], GitState::Modified);
        assert_eq!(s.paths[Path::new("/w/notes")], GitState::Untracked);
        assert_eq!(s.paths[Path::new("/w/new.rs")], GitState::Renamed);
        assert!(!s.paths.contains_key(Path::new("/w/old.rs")));
        assert_eq!(s.paths[Path::new("/w/src/deep")], GitState::Conflicted);
        // The strongest state beneath a folder wins.
        assert_eq!(s.paths[Path::new("/w/src")], GitState::Conflicted);
        assert!(!s.paths.contains_key(Path::new("/w")));
    }
}
