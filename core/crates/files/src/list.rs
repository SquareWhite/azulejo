//! One folder's entries: folders first, natural order, gitignore-aware.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

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
    let allowed = not_ignored(dir);

    let mut entries: Vec<Entry> = raw
        .into_iter()
        .filter_map(|(name, kind)| {
            let path = dir.join(&name);
            let symlink = kind.is_symlink();
            let is_dir =
                kind.is_dir() || (symlink && fs::metadata(&path).is_ok_and(|m| m.is_dir()));
            let hidden = name.as_encoded_bytes().first() == Some(&b'.');
            let ignored = !allowed.contains(&name);
            if !show_hidden && (hidden || ignored) {
                return None;
            }
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

/// Names in `dir` that no ignore rule excludes. The walker reads ignore files
/// from `dir` and its parents up to the repository root, so a rule in the
/// root `.gitignore` applies three folders down.
fn not_ignored(dir: &Path) -> HashSet<OsString> {
    ignore::WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(false)
        .parents(true)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.depth() == 1)
        .map(|e| e.file_name().to_os_string())
        .collect()
}
