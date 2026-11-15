//! One folder's entries: folders first, natural order, gitignore-aware.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::sort::natural_cmp;

/// One entry in a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    /// A folder, or a symlink to one.
    pub dir: bool,
    pub symlink: bool,
    /// Starts with a dot.
    pub hidden: bool,
    /// Excluded by `.gitignore`, `.ignore`, `.git/info/exclude` or the global
    /// git excludes (only inside a git work tree).
    pub ignored: bool,
}

/// A folder's entries, sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// More than `max` entries matched; the rest were dropped.
    pub truncated: bool,
    /// See [`signature`]: lets a watcher tell a real change from a replayed
    /// or content-only notification.
    pub signature: u64,
}

/// Lists `dir`. Hidden and ignored entries are left out unless `show_hidden`;
/// either way they're flagged. Folders come first, then natural name order.
///
/// Cheap on purpose: one `readdir` of `dir`, the ignore files in `dir` and
/// its parents, and a `stat` per shown symlink (to tell a link to a folder).
/// Nothing below `dir` is touched, so a folder of 20k subfolders, mount
/// points or protected folders lists as fast as its names can be read.
pub fn list(dir: &Path, show_hidden: bool, max: usize) -> io::Result<Listing> {
    let mut raw: Vec<(OsString, fs::FileType)> = Vec::new();
    for item in fs::read_dir(dir)? {
        let item = item?;
        // A file can vanish between readdir and stat; skip it.
        if let Ok(kind) = item.file_type() {
            raw.push((item.file_name(), kind));
        }
    }
    let signature = signature_of(raw.iter().map(|(n, k)| (n.as_os_str(), k.is_dir())));
    let rules = Rules::new(dir, raw.iter().map(|(n, _)| n.as_os_str()));

    let mut entries: Vec<Entry> = raw
        .into_iter()
        .filter_map(|(name, kind)| {
            let path = dir.join(&name);
            let symlink = kind.is_symlink();
            let hidden = name.as_encoded_bytes().first() == Some(&b'.');
            // Ignore rules see a symlink as a file, as git does.
            let ignored = rules.ignored(&path, kind.is_dir());
            if !show_hidden && (hidden || ignored) {
                return None;
            }
            let is_dir =
                kind.is_dir() || (symlink && fs::metadata(&path).is_ok_and(|m| m.is_dir()));
            Some(Entry {
                name: name.to_string_lossy().into_owned(),
                path,
                dir: is_dir,
                symlink,
                hidden,
                ignored,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| natural_cmp(&a.name, &b.name))
    });
    let truncated = entries.len() > max;
    entries.truncate(max);
    Ok(Listing {
        entries,
        truncated,
        signature,
    })
}

/// A cheap fingerprint of a folder's names and which are folders. File
/// contents don't count: the tree shows names only. `None` when the folder
/// can't be read (gone, or no permission).
pub fn signature(dir: &Path) -> Option<u64> {
    let raw: Vec<(OsString, bool)> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| {
            let is_dir = e.file_type().is_ok_and(|k| k.is_dir());
            (e.file_name(), is_dir)
        })
        .collect();
    Some(signature_of(raw.iter().map(|(n, d)| (n.as_os_str(), *d))))
}

fn signature_of<'a>(items: impl Iterator<Item = (&'a std::ffi::OsStr, bool)>) -> u64 {
    let mut items: Vec<_> = items.collect();
    items.sort_unstable();
    let mut h = DefaultHasher::new();
    items.hash(&mut h);
    h.finish()
}

/// The ignore rules for one folder's entries: `.ignore` and `.gitignore` in
/// the folder and each parent, `.git/info/exclude` and the global git
/// excludes, with the precedence the `ignore` crate's walker gives them: a
/// match in a deeper folder wins, `.ignore` beats `.gitignore`, which beats
/// the excludes, and git's rules only apply inside a work tree, up to its
/// root. Built here rather than by walking `dir` with `ignore::Walk`, which
/// reads the folder a second time and sets up a matcher per subfolder (10x
/// slower on a folder of 20k subfolders).
struct Rules {
    /// The listed folder first, then each parent up to `/`.
    layers: Vec<Layer>,
    global: Gitignore,
    in_work_tree: bool,
}

struct Layer {
    ignore: Gitignore,
    git_ignore: Gitignore,
    git_exclude: Gitignore,
    /// Holds `.git` (or `.jj`): a work tree's root.
    repo_root: bool,
}

impl Rules {
    /// `names` are `dir`'s entries, so `dir` itself needs no probing.
    fn new<'a>(dir: &Path, names: impl Iterator<Item = &'a OsStr>) -> Rules {
        let mut here = Present::default();
        for name in names {
            match name.as_encoded_bytes() {
                b".ignore" => here.ignore = true,
                b".gitignore" => here.git_ignore = true,
                b".git" => here.git = true,
                b".jj" => here.jj = true,
                _ => {}
            }
        }
        let layers: Vec<Layer> = dir
            .ancestors()
            .enumerate()
            .map(|(i, d)| Layer::read(d, (i == 0).then_some(&here)))
            .collect();
        let in_work_tree = layers.iter().any(|l| l.repo_root);
        let global = if in_work_tree {
            GitignoreBuilder::new("").build_global().0
        } else {
            Gitignore::empty()
        };
        Rules {
            layers,
            global,
            in_work_tree,
        }
    }

    /// Whether the rules exclude `path`, an entry of the listed folder.
    fn ignored(&self, path: &Path, is_dir: bool) -> bool {
        let mut ignore = Match::None;
        let mut git_ignore = Match::None;
        let mut git_exclude = Match::None;
        let mut past_repo_root = false;
        for layer in &self.layers {
            if ignore.is_none() {
                ignore = layer.ignore.matched(path, is_dir);
            }
            if self.in_work_tree && !past_repo_root {
                if git_ignore.is_none() {
                    git_ignore = layer.git_ignore.matched(path, is_dir);
                }
                if git_exclude.is_none() {
                    git_exclude = layer.git_exclude.matched(path, is_dir);
                }
            }
            past_repo_root |= layer.repo_root;
        }
        let global = if self.in_work_tree {
            self.global.matched(path, is_dir)
        } else {
            Match::None
        };
        ignore.or(git_ignore).or(git_exclude).or(global).is_ignore()
    }
}

/// Which ignore files and repository markers a folder holds, known from its
/// entries.
#[derive(Default)]
struct Present {
    ignore: bool,
    git_ignore: bool,
    git: bool,
    jj: bool,
}

impl Layer {
    /// `present`: what `dir` holds, when its entries were read anyway;
    /// otherwise each file is probed.
    fn read(dir: &Path, present: Option<&Present>) -> Layer {
        let git = match present {
            Some(p) if !p.git => None,
            _ => fs::metadata(dir.join(".git")).ok().map(|m| m.file_type()),
        };
        let jj = present.map_or_else(|| dir.join(".jj").exists(), |p| p.jj);
        let ignore = present.is_none_or(|p| p.ignore);
        let git_ignore = present.is_none_or(|p| p.git_ignore);
        Layer {
            ignore: rules_file(dir, ignore.then(|| dir.join(".ignore"))),
            git_ignore: rules_file(dir, git_ignore.then(|| dir.join(".gitignore"))),
            git_exclude: rules_file(
                dir,
                git.and_then(|kind| common_git_dir(dir, kind))
                    .map(|g| g.join("info/exclude")),
            ),
            repo_root: git.is_some() || jj,
        }
    }
}

/// The rules in `file`, if it exists, relative to `root`.
fn rules_file(root: &Path, file: Option<PathBuf>) -> Gitignore {
    let Some(file) = file.filter(|f| f.is_file()) else {
        return Gitignore::empty();
    };
    let mut builder = GitignoreBuilder::new(root);
    // A bad line or an unreadable file costs that file's rules, nothing more.
    let _ = builder.add(file);
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// The folder holding `info/exclude` for the work tree rooted at `dir`:
/// `.git`, or for a linked work tree (`.git` is a file) the main one's.
fn common_git_dir(dir: &Path, kind: fs::FileType) -> Option<PathBuf> {
    let dot_git = dir.join(".git");
    if !kind.is_file() {
        return Some(dot_git);
    }
    let first_line = |p: &Path| {
        fs::read_to_string(p)
            .ok()
            .and_then(|s| s.lines().next().map(str::to_owned))
    };
    let line = first_line(&dot_git)?;
    let git_dir = dir.join(line.strip_prefix("gitdir: ")?);
    Some(match first_line(&git_dir.join("commondir")) {
        Some(common) => git_dir.join(common),
        None => git_dir,
    })
}
