//! `files.transfer`: moves and copies files and folders into a folder, on
//! this machine or between Local and a herdr machine. No UI here: shells
//! ask, the core runs it off its thread and replies with one result per
//! source.
//!
//! - **Local to Local**: a rename when it can (a move on one volume), else a
//!   copy (and, for a move, removing the original once the copy is whole).
//! - **Local to a machine** (upload) and **a machine to Local** (download):
//!   `scp` over the machine's shared SSH connection
//!   ([`RemoteMachine::scp`]); a move removes the originals only after
//!   their transfer succeeded.
//!
//! Nothing is ever overwritten. A name already taken in the folder gets a
//! Finder-style number (`notes 2.txt`). Locally the new name is claimed by
//! creating it (`O_EXCL`, `mkdir`, `symlink`), so a file that appears
//! meanwhile isn't overwritten either; a remote folder is listed first and
//! its names compared ignoring case (the machine's file system may).

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use azulejo_herdr::{RemoteMachine, shell_quote};
use azulejo_protocol::{
    Error, ErrorCode, FilesTransfer, LOCAL_MACHINE, TransferItem, TransferMode,
};

use crate::remote_files::{self, RunError};

/// How long one `scp` may run: uploads can be big, and a dead link ends
/// sooner through SSH's own keepalives (`ServerAliveInterval`).
const SCP_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);

/// Where a transfer's sources or folder are.
#[derive(Debug, Clone)]
pub(crate) enum Place {
    Local,
    Remote(RemoteMachine),
}

/// One `files.transfer`, ready to run.
#[derive(Debug, Clone)]
pub(crate) struct Transfer {
    pub sources: Vec<PathBuf>,
    pub from: Place,
    pub folder: PathBuf,
    pub to: Place,
    pub mode: TransferMode,
}

impl Transfer {
    /// A `files.transfer` request with its machines looked up in
    /// `machines` (the ones the core can reach over SSH, by ID).
    pub(crate) fn new(
        p: FilesTransfer,
        machines: &HashMap<String, RemoteMachine>,
    ) -> Result<Self, Error> {
        let from = place(p.from.as_deref(), machines)?;
        let to = place(p.to.as_deref(), machines)?;
        if matches!((&from, &to), (Place::Remote(_), Place::Remote(_))) {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "transfers between remote machines aren't supported",
            ));
        }
        let folder = PathBuf::from(&p.folder);
        if !folder.is_absolute() {
            return Err(invalid(format!("{} isn't an absolute path", p.folder)));
        }
        if let Some(bad) = p.sources.iter().find(|s| !Path::new(s).is_absolute()) {
            return Err(invalid(format!("{bad} isn't an absolute path")));
        }
        Ok(Transfer {
            sources: p.sources.into_iter().map(PathBuf::from).collect(),
            from,
            folder,
            to,
            mode: p.mode,
        })
    }
}

/// Where machine `id` (`machines.list` ID; Local when absent or `local`) is,
/// among the ones the core can reach over SSH.
pub(crate) fn place(
    id: Option<&str>,
    machines: &HashMap<String, RemoteMachine>,
) -> Result<Place, Error> {
    match id {
        None | Some(LOCAL_MACHINE) => Ok(Place::Local),
        // Shells show the message as is, so it names no ID.
        Some(id) => machines.get(id).cloned().map(Place::Remote).ok_or_else(|| {
            Error::new(ErrorCode::Unavailable, "the machine isn't connected")
                .with_data(serde_json::json!({ "machine": id }))
        }),
    }
}

impl Place {
    /// The machine's ID; `None` for Local.
    pub(crate) fn machine(&self) -> Option<&str> {
        match self {
            Place::Local => None,
            Place::Remote(m) => Some(m.profile.id.as_str()),
        }
    }
}

fn invalid(why: String) -> Error {
    Error::new(ErrorCode::InvalidParams, why)
}

/// Runs a transfer (blocking: call it off the engine's thread). Fails as a
/// whole only when nothing could start (the folder isn't one, the machine
/// can't be reached); otherwise each source has its own result.
pub(crate) fn run(t: &Transfer) -> Result<Vec<TransferItem>, Error> {
    if t.sources.is_empty() {
        return Ok(Vec::new());
    }
    if !t.folder.is_absolute() {
        return Err(invalid(format!(
            "{} isn't an absolute path",
            t.folder.display()
        )));
    }
    match (&t.from, &t.to) {
        (Place::Local, Place::Local) => local(t),
        (Place::Local, Place::Remote(m)) => upload(t, m),
        (Place::Remote(m), Place::Local) => download(t, m),
        (Place::Remote(_), Place::Remote(_)) => Err(Error::new(
            ErrorCode::Unsupported,
            "transfers between remote machines aren't supported",
        )),
    }
}

fn item(source: &Path, result: Result<PathBuf, String>) -> TransferItem {
    match result {
        Ok(dest) => TransferItem {
            source: source.display().to_string(),
            dest: Some(dest.display().to_string()),
            error: None,
        },
        Err(why) => TransferItem {
            source: source.display().to_string(),
            dest: None,
            error: Some(why),
        },
    }
}

/// `notes.txt`, `notes 2.txt`, `notes 3.txt`, …: the names to try for
/// `name`, Finder's way. A name that already ends in a number counts on
/// from it (`notes 2.txt` → `notes 3.txt`); the extension stays (a leading
/// dot isn't one: `.env` → `.env 2`).
pub(crate) fn candidates(name: &OsStr) -> impl Iterator<Item = OsString> {
    let text = name.to_string_lossy().into_owned();
    let raw = name.to_os_string();
    let (stem, ext) = match text.rfind('.') {
        Some(i) if i > 0 && i + 1 < text.len() => (text[..i].to_owned(), text[i..].to_owned()),
        _ => (text.clone(), String::new()),
    };
    let (base, start) = match stem.rsplit_once(' ') {
        Some((base, n))
            if !base.is_empty()
                && !n.is_empty()
                && n.len() < 9
                && n.bytes().all(|b| b.is_ascii_digit())
                && !n.starts_with('0') =>
        {
            (base.to_owned(), n.parse::<u64>().unwrap_or(1) + 1)
        }
        _ => (stem, 2),
    };
    // The name as it is first (bytes kept when it isn't UTF-8), then numbered.
    std::iter::once(raw).chain((start..).map(move |n| OsString::from(format!("{base} {n}{ext}"))))
}

/// The first of `name`'s candidates not in `taken` (compared lower-cased).
fn unique(name: &OsStr, taken: &HashSet<String>) -> OsString {
    candidates(name)
        .find(|c| !taken.contains(&c.to_string_lossy().to_lowercase()))
        .unwrap_or_else(|| name.to_os_string())
}

fn file_name(path: &Path) -> Result<&OsStr, String> {
    path.file_name()
        .ok_or_else(|| format!("{} has no name", path.display()))
}

pub(crate) fn io_why(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "no such file or folder".into(),
        io::ErrorKind::PermissionDenied => "permission denied".into(),
        _ => {
            let text = e.to_string();
            // "Read-only file system (os error 30)" → without the number.
            text.split(" (os error").next().unwrap_or(&text).to_owned()
        }
    }
}

// MARK: - Local

fn local(t: &Transfer) -> Result<Vec<TransferItem>, Error> {
    let folder = local_folder(&t.folder)?;
    Ok(t.sources
        .iter()
        .map(|src| {
            let result = match t.mode {
                TransferMode::Move => move_local(src, &folder),
                TransferMode::Copy => copy_local(src, &folder),
            };
            item(src, result)
        })
        .collect())
}

/// The folder, resolved (`/tmp` → `/private/tmp`), or why it can't take
/// anything.
fn local_folder(folder: &Path) -> Result<PathBuf, Error> {
    let resolved = fs::canonicalize(folder)
        .map_err(|e| invalid(format!("{}: {}", folder.display(), io_why(&e))))?;
    if !resolved.is_dir() {
        return Err(invalid(format!("{} isn't a folder", folder.display())));
    }
    Ok(resolved)
}

/// `src` with its folder resolved and its own name kept (a symlink stays the
/// link, not what it points to).
pub(crate) fn resolved_source(src: &Path) -> Result<PathBuf, String> {
    let name = file_name(src)?;
    let parent = src.parent().unwrap_or(Path::new("/"));
    let parent =
        fs::canonicalize(parent).map_err(|e| format!("{}: {}", src.display(), io_why(&e)))?;
    let src = parent.join(name);
    fs::symlink_metadata(&src).map_err(|e| format!("{}: {}", src.display(), io_why(&e)))?;
    Ok(src)
}

/// What a new name is claimed as: an empty file or an empty folder.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Claim {
    File,
    Dir,
}

/// Claims the first free name for `name` in `folder` by creating it.
fn claim(folder: &Path, name: &OsStr, kind: Claim) -> io::Result<PathBuf> {
    for candidate in candidates(name).take(10_000) {
        let path = folder.join(&candidate);
        let made = match kind {
            Claim::File => fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map(|_| ()),
            Claim::Dir => fs::create_dir(&path),
        };
        match made {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free name left",
    ))
}

/// Removes what [`claim`] made (and anything a failed copy left in it).
fn unclaim(path: &Path) {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => {
            let _ = fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = fs::remove_file(path);
        }
        Err(_) => {}
    }
}

pub(crate) fn move_local(src: &Path, folder: &Path) -> Result<PathBuf, String> {
    let src = resolved_source(src)?;
    if src.parent() == Some(folder) {
        // Already there: nothing to do (Finder does nothing either).
        return Ok(src);
    }
    if folder.starts_with(&src) {
        return Err("can't move a folder into itself".into());
    }
    let meta = fs::symlink_metadata(&src).map_err(|e| io_why(&e))?;
    let kind = if meta.is_dir() {
        Claim::Dir
    } else {
        Claim::File
    };
    let dest = claim(folder, file_name(&src)?, kind).map_err(|e| io_why(&e))?;
    // A rename replaces the empty file or folder just claimed, atomically.
    match fs::rename(&src, &dest) {
        Ok(()) => Ok(dest),
        Err(e) if is_cross_device(&e) => {
            if let Err(e) = copy_tree(&src, &dest, true) {
                unclaim(&dest);
                return Err(io_why(&e));
            }
            remove(&src)
                .map_err(|e| format!("copied, but couldn't remove the original: {}", io_why(&e)))?;
            Ok(dest)
        }
        Err(e) => {
            unclaim(&dest);
            Err(io_why(&e))
        }
    }
}

fn is_cross_device(e: &io::Error) -> bool {
    // EXDEV is 18 on macOS and Linux.
    e.kind() == io::ErrorKind::CrossesDevices || e.raw_os_error() == Some(18)
}

fn copy_local(src: &Path, folder: &Path) -> Result<PathBuf, String> {
    let src = resolved_source(src)?;
    if folder.starts_with(&src) {
        return Err("can't copy a folder into itself".into());
    }
    let meta = fs::symlink_metadata(&src).map_err(|e| io_why(&e))?;
    let name = file_name(&src)?;
    if meta.file_type().is_symlink() {
        // A link is copied as a link; making it claims the name.
        let target = fs::read_link(&src).map_err(|e| io_why(&e))?;
        for candidate in candidates(name).take(10_000) {
            let dest = folder.join(&candidate);
            match std::os::unix::fs::symlink(&target, &dest) {
                Ok(()) => return Ok(dest),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io_why(&e)),
            }
        }
        return Err("no free name left".into());
    }
    let kind = if meta.is_dir() {
        Claim::Dir
    } else {
        Claim::File
    };
    let dest = claim(folder, name, kind).map_err(|e| io_why(&e))?;
    match copy_tree(&src, &dest, true) {
        Ok(()) => Ok(dest),
        Err(e) => {
            unclaim(&dest);
            Err(io_why(&e))
        }
    }
}

/// Copies `src` to `dest`: a file's bytes, permissions and modification
/// time, a folder with everything in it, a symlink as a link. `claimed`:
/// `dest` already exists as the empty file or folder [`claim`] made.
fn copy_tree(src: &Path, dest: &Path, claimed: bool) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    let kind = meta.file_type();
    if kind.is_symlink() {
        if claimed {
            fs::remove_file(dest)?;
        }
        return std::os::unix::fs::symlink(fs::read_link(src)?, dest);
    }
    if kind.is_dir() {
        if !claimed {
            fs::create_dir(dest)?;
        }
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dest.join(entry.file_name()), false)?;
        }
        fs::set_permissions(dest, meta.permissions())?;
        return Ok(());
    }
    if !claimed {
        // Inside a new folder nothing else is there, but never overwrite.
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dest)?;
    }
    fs::copy(src, dest)?;
    if let Ok(modified) = meta.modified() {
        let _ = fs::OpenOptions::new()
            .write(true)
            .open(dest)
            .and_then(|f| f.set_modified(modified));
    }
    Ok(())
}

/// Removes a file, a link or a whole folder.
fn remove(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

// MARK: - Between machines

/// The names in a remote folder, lower-cased, or why it can't be listed.
fn remote_names(m: &RemoteMachine, folder: &Path) -> Result<HashSet<String>, Error> {
    let dirs = vec![folder.to_path_buf()];
    let never = AtomicBool::new(false);
    let out = remote_files::run(
        &m.exec(remote_files::list_script(&dirs, MAX_NAMES)),
        &never,
        remote_files::TIMEOUT,
    )
    .map_err(|e| {
        let code = match e {
            RunError::Unreachable(_) | RunError::TimedOut => ErrorCode::Unavailable,
            _ => ErrorCode::InvalidParams,
        };
        Error::new(code, e.message(&m.profile.label))
    })?;
    let listing = remote_files::parse_listing(&out, &dirs);
    match listing.into_iter().next() {
        Some((_, Ok(dir))) => Ok(dir
            .entries
            .into_iter()
            .map(|e| e.name.to_lowercase())
            .collect()),
        Some((_, Err(why))) => Err(invalid(format!("{}: {why}", folder.display()))),
        None => Err(invalid(format!("{}: can't list it", folder.display()))),
    }
}

fn scp(m: &RemoteMachine, from: String, to: String) -> Result<(), String> {
    let never = AtomicBool::new(false);
    remote_files::run(&m.scp(vec![from, to]), &never, SCP_TIMEOUT)
        .map(|_| ())
        .map_err(|e| e.message(&m.profile.label))
}

/// Most names a remote folder's listing reads to find a free one.
const MAX_NAMES: usize = 10_000_000;

/// A source that failed: why, and where its copy is when only the step
/// after the transfer failed (a move's original couldn't be removed).
struct Failed {
    dest: Option<PathBuf>,
    why: String,
}

impl From<String> for Failed {
    fn from(why: String) -> Self {
        Failed { dest: None, why }
    }
}

impl From<&str> for Failed {
    fn from(why: &str) -> Self {
        why.to_owned().into()
    }
}

fn outcome(source: &Path, result: Result<PathBuf, Failed>) -> TransferItem {
    match result {
        Ok(dest) => item(source, Ok(dest)),
        Err(f) => TransferItem {
            source: source.display().to_string(),
            dest: f.dest.map(|d| d.display().to_string()),
            error: Some(f.why),
        },
    }
}

fn upload(t: &Transfer, m: &RemoteMachine) -> Result<Vec<TransferItem>, Error> {
    let folder = remote_files::normalize(&t.folder);
    let mut taken = remote_names(m, &folder)?;
    let mut items = Vec::new();
    for src in &t.sources {
        let result = (|| -> Result<PathBuf, Failed> {
            let src = resolved_source(src)?;
            let name = unique(file_name(&src)?, &taken);
            let dest = folder.join(&name);
            scp(
                m,
                src.display().to_string(),
                m.scp_path(&dest.to_string_lossy()),
            )?;
            taken.insert(name.to_string_lossy().to_lowercase());
            if t.mode == TransferMode::Move
                && let Err(e) = remove(&src)
            {
                return Err(Failed {
                    why: format!("uploaded, but couldn't remove the original: {}", io_why(&e)),
                    dest: Some(dest),
                });
            }
            Ok(dest)
        })();
        items.push(outcome(src, result));
    }
    Ok(items)
}

/// A remote path as scp's SFTP mode takes a source: it expands globs in
/// it, so `*?[]\` are escaped.
fn glob_escape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn download(t: &Transfer, m: &RemoteMachine) -> Result<Vec<TransferItem>, Error> {
    let folder = local_folder(&t.folder)?;
    let mut items = Vec::new();
    for src in &t.sources {
        let src = remote_files::normalize(src);
        let result = (|| -> Result<PathBuf, Failed> {
            let name = file_name(&src)?;
            // A download can't claim its name first (a folder would land
            // inside the claimed one), so it takes a name nothing has.
            let dest = candidates(name)
                .take(10_000)
                .map(|c| folder.join(c))
                .find(|p| fs::symlink_metadata(p).is_err())
                .ok_or("no free name left")?;
            scp(
                m,
                m.scp_path(&glob_escape(&src.to_string_lossy())),
                dest.display().to_string(),
            )?;
            if t.mode == TransferMode::Move {
                let never = AtomicBool::new(false);
                let script = format!("rm -rf -- {}", shell_quote(&src.to_string_lossy()));
                if let Err(e) = remote_files::run(&m.exec(script), &never, remote_files::TIMEOUT) {
                    return Err(Failed {
                        why: format!(
                            "downloaded, but couldn't remove the original: {}",
                            e.message(&m.profile.label)
                        ),
                        dest: Some(dest),
                    });
                }
            }
            Ok(dest)
        })();
        items.push(outcome(&src, result));
    }
    Ok(items)
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
