//! `files.create` and `files.delete`: making an empty file or folder and
//! deleting entries, on this machine or on a herdr machine. Like
//! `files.transfer`, the core runs them off its thread and replies when
//! they're done; shells use them for the sidebar's Files view, and the CLI
//! gets them for free.
//!
//! - **Create** never replaces anything. Locally the name is claimed by
//!   creating it (`O_EXCL`, `mkdir`); on a machine a small `/bin/sh`
//!   script checks first and makes it with `noclobber` or `mkdir`.
//! - **Delete** moves Local entries to the Trash (`NSFileManager`, so
//!   Finder's Put Back works) and removes a machine's for good (`rm -rf`).
//!   `AZJ_TRASH_DIR` points the Trash at a folder of a test's own.
//!
//! A path given with a files panel (`tile`) is relative to its root and on
//! its machine; deleting needs it inside that root.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use azulejo_herdr::{RemoteMachine, shell_quote};
use azulejo_protocol::{
    DeleteItem, Error, ErrorCode, FileKind, FilesCreate, FilesDelete, FilesSshUrl, LOCAL_MACHINE,
};

use crate::remote_files;
use crate::transfer::{self, Place};

/// How long a remote `rm -rf` may take: a big tree takes a while.
const DELETE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// The files panel a command named: its root (resolved) and machine.
#[derive(Debug, Clone)]
pub(crate) struct Panel {
    pub root: PathBuf,
    pub machine: Option<String>,
}

/// One `files.create`, ready to run.
#[derive(Debug, Clone)]
pub(crate) struct Create {
    pub path: PathBuf,
    pub kind: FileKind,
    pub place: Place,
}

/// One `files.delete`, ready to run.
#[derive(Debug, Clone)]
pub(crate) struct Delete {
    pub paths: Vec<PathBuf>,
    pub place: Place,
}

fn invalid(why: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidParams, why)
}

/// The machine and root paths resolve against: the panel's when there is
/// one (a `machine` that says otherwise is refused), else `machine`.
fn scope(machine: Option<String>, panel: Option<&Panel>) -> Result<Option<String>, Error> {
    // `Some(None)`: Local, said outright.
    let given = machine
        .filter(|m| !m.is_empty())
        .map(|m| Some(m).filter(|m| m != LOCAL_MACHINE));
    match panel {
        None => Ok(given.flatten()),
        Some(panel) if given.is_none() || given.as_ref() == Some(&panel.machine) => {
            Ok(panel.machine.clone())
        }
        Some(_) => Err(invalid(
            "the panel's files are on another machine; leave out `machine`",
        )),
    }
}

/// `path` made absolute (against the panel's root when it's relative) with
/// `.` and `..` worked out by name. `~` is this Mac's home, Local only.
fn absolute(path: &str, panel: Option<&Panel>, remote: bool) -> Result<PathBuf, Error> {
    let given = if remote {
        PathBuf::from(path)
    } else {
        crate::files::expand_home(path)
    };
    let joined = match panel {
        _ if given.is_absolute() => given,
        Some(panel) => panel.root.join(given),
        None => {
            return Err(invalid(format!(
                "{path} isn't an absolute path (or name a files panel as `tile`)"
            )));
        }
    };
    Ok(remote_files::normalize(&joined))
}

/// A name a new entry can have: the path's last part, not empty, `.` or
/// `..`, and short enough for a file system.
fn new_name(path: &Path) -> Result<&OsStr, Error> {
    let name = path
        .file_name()
        .ok_or_else(|| invalid(format!("{} has no name to make", path.display())))?;
    if name.len() > 255 {
        return Err(invalid("that name is too long"));
    }
    Ok(name)
}

/// Deleting `path` can't be meant: `/`, a folder right under it, or the
/// home folder.
fn too_broad(path: &Path, local: bool) -> bool {
    let parts = path
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    parts < 2 || (local && std::env::var_os("HOME").is_some_and(|h| Path::new(&h) == path))
}

/// `files.ssh_url`: the path, made absolute against the panel's root, as
/// an `ssh://` URL to its machine. Local paths have none; a machine another
/// app's SSH can't reach as Azulejo does is `unavailable`, saying why.
pub(crate) fn ssh_url(
    p: FilesSshUrl,
    panel: Option<&Panel>,
    machines: &HashMap<String, RemoteMachine>,
) -> Result<String, Error> {
    let machine = scope(p.machine, panel)?;
    let Place::Remote(remote) = transfer::place(machine.as_deref(), machines)? else {
        return Err(invalid(
            "that path is on this Mac; files.ssh_url is for a herdr machine's",
        ));
    };
    let path = absolute(&p.path, panel, true)?;
    let dest = remote
        .plain_destination()
        .map_err(|why| Error::new(ErrorCode::Unavailable, why))?;
    Ok(dest.url(&path.to_string_lossy()))
}

impl Create {
    pub(crate) fn new(
        p: FilesCreate,
        panel: Option<&Panel>,
        machines: &HashMap<String, RemoteMachine>,
    ) -> Result<Self, Error> {
        let machine = scope(p.machine, panel)?;
        let place = transfer::place(machine.as_deref(), machines)?;
        let path = absolute(&p.path, panel, machine.is_some())?;
        new_name(&path)?;
        Ok(Create {
            path,
            kind: p.kind,
            place,
        })
    }
}

impl Delete {
    pub(crate) fn new(
        p: FilesDelete,
        panel: Option<&Panel>,
        machines: &HashMap<String, RemoteMachine>,
    ) -> Result<Self, Error> {
        let machine = scope(p.machine, panel)?;
        let remote = machine.is_some();
        let mut paths = Vec::new();
        for given in &p.paths {
            let path = absolute(given, panel, remote)?;
            if let Some(panel) = panel
                && (!path.starts_with(&panel.root) || path == panel.root)
            {
                return Err(invalid(format!(
                    "{} isn't inside the panel's folder",
                    path.display()
                )));
            }
            if too_broad(&path, !remote) {
                return Err(invalid(format!("won't delete {}", path.display())));
            }
            paths.push(path);
        }
        let place = transfer::place(machine.as_deref(), machines)?;
        Ok(Delete { paths, place })
    }
}

// MARK: - Create

/// Makes the entry (blocking: call it off the engine's thread) and returns
/// its path, resolved on Local.
pub(crate) fn create(c: &Create) -> Result<PathBuf, Error> {
    match &c.place {
        Place::Local => create_local(&c.path, c.kind),
        Place::Remote(m) => create_remote(m, &c.path, c.kind).map(|()| c.path.clone()),
    }
}

fn create_local(path: &Path, kind: FileKind) -> Result<PathBuf, Error> {
    let name = new_name(path)?;
    let parent = path.parent().unwrap_or(Path::new("/"));
    let folder = fs::canonicalize(parent)
        .map_err(|e| invalid(format!("{}: {}", parent.display(), transfer::io_why(&e))))?;
    if !folder.is_dir() {
        return Err(invalid(format!("{} isn't a folder", parent.display())));
    }
    let dest = folder.join(name);
    let made = match kind {
        FileKind::File => fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
            .map(|_| ()),
        FileKind::Folder => fs::create_dir(&dest),
    };
    match made {
        Ok(()) => Ok(dest),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(taken(name)),
        Err(e) => Err(invalid(format!(
            "can't make {}: {}",
            name.to_string_lossy(),
            transfer::io_why(&e)
        ))),
    }
}

fn taken(name: &OsStr) -> Error {
    invalid(format!("{} is already there", name.to_string_lossy()))
}

/// Makes `$1` (`$2`: `file` or `folder`) in folder `$3`, failing when the
/// name is taken or the folder isn't one. Exit 3 with a message on stderr
/// for those; `mkdir` or the shell says why otherwise.
const MAKE: &str = concat!(
    r#"p=$1; k=$2; d=$3; "#,
    r#"if [ -e "$p" ] || [ -L "$p" ]; then printf '%s is already there\n' "${p##*/}" >&2; exit 3; fi; "#,
    r#"if [ ! -d "$d" ]; then printf '%s isn'\''t a folder\n' "$d" >&2; exit 3; fi; "#,
    r#"if [ "$k" = folder ]; then mkdir -- "$p" || exit 1; "#,
    r#"else (set -C; : > "$p") || exit 1; fi"#,
);

pub(crate) fn make_script(path: &Path, kind: FileKind) -> String {
    let parent = path.parent().unwrap_or(Path::new("/"));
    let kind = match kind {
        FileKind::File => "file",
        FileKind::Folder => "folder",
    };
    format!(
        "exec /bin/sh -c {} azj-mk {} {kind} {}",
        shell_quote(MAKE),
        shell_quote(&path.to_string_lossy()),
        shell_quote(&parent.to_string_lossy()),
    )
}

fn create_remote(m: &RemoteMachine, path: &Path, kind: FileKind) -> Result<(), Error> {
    let never = AtomicBool::new(false);
    remote_files::run(
        &m.exec(make_script(path, kind)),
        &never,
        remote_files::TIMEOUT,
    )
    .map(|_| ())
    .map_err(|e| remote_error(m, e))
}

fn remote_error(m: &RemoteMachine, e: remote_files::RunError) -> Error {
    use remote_files::RunError;
    let code = match e {
        RunError::Unreachable(_) | RunError::TimedOut => ErrorCode::Unavailable,
        _ => ErrorCode::InvalidParams,
    };
    Error::new(code, e.message(&m.profile.label))
}

// MARK: - Delete

/// Moves a Local path to the Trash; where it went, or why not.
pub(crate) type TrashFn = dyn Fn(&Path) -> Result<PathBuf, String> + Send + Sync;

/// The Trash deletes go to: `AZJ_TRASH_DIR` when it's set (tests), else
/// the system's.
pub(crate) fn trash() -> &'static TrashFn {
    static TRASH: OnceLock<Box<TrashFn>> = OnceLock::new();
    TRASH
        .get_or_init(|| match std::env::var_os("AZJ_TRASH_DIR") {
            Some(dir) if !dir.is_empty() => {
                let dir = PathBuf::from(dir);
                Box::new(move |path: &Path| {
                    fs::create_dir_all(&dir).map_err(|e| transfer::io_why(&e))?;
                    let dir = fs::canonicalize(&dir).map_err(|e| transfer::io_why(&e))?;
                    transfer::move_local(path, &dir)
                })
            }
            _ => Box::new(system_trash),
        })
        .as_ref()
}

/// Deletes the paths (blocking: call it off the engine's thread): one item
/// each, in order. Fails as a whole only when the machine can't be reached.
pub(crate) fn delete(d: &Delete, trash: &TrashFn) -> Result<Vec<DeleteItem>, Error> {
    match &d.place {
        Place::Local => Ok(d.paths.iter().map(|p| delete_local(p, trash)).collect()),
        Place::Remote(m) => delete_remote(m, &d.paths),
    }
}

fn delete_local(path: &Path, trash: &TrashFn) -> DeleteItem {
    // The entry itself (a symlink, not what it points to), its folder
    // resolved.
    let result = transfer::resolved_source(path).and_then(|src| trash(&src));
    match result {
        Ok(to) => DeleteItem {
            path: path.display().to_string(),
            trashed: Some(to.display().to_string()).filter(|t| !t.is_empty()),
            error: None,
        },
        Err(why) => DeleteItem {
            path: path.display().to_string(),
            trashed: None,
            error: Some(why),
        },
    }
}

/// Removes each argument for good. Prints `O\t<n>` or `E\t<n>\t<why>` per
/// path (`n` counts from 1), NUL-separated.
const REMOVE: &str = concat!(
    r#"n=0; for p in "$@"; do n=$((n+1)); "#,
    r#"if [ ! -e "$p" ] && [ ! -L "$p" ]; then printf 'E\t%s\tno such file or folder\0' "$n"; continue; fi; "#,
    r#"rm -rf -- "$p" 2>/dev/null; "#,
    r#"if [ -e "$p" ] || [ -L "$p" ]; then printf 'E\t%s\tcouldn'\''t remove all of it\0' "$n"; "#,
    r#"else printf 'O\t%s\0' "$n"; fi; done"#,
);

pub(crate) fn remove_script(paths: &[PathBuf]) -> String {
    let mut s = format!("exec /bin/sh -c {} azj-rm", shell_quote(REMOVE));
    for p in paths {
        s.push(' ');
        s.push_str(&shell_quote(&p.to_string_lossy()));
    }
    s
}

/// The remove script's output as one item per path.
pub(crate) fn parse_removed(out: &[u8], paths: &[PathBuf]) -> Vec<DeleteItem> {
    let mut items: Vec<DeleteItem> = paths
        .iter()
        .map(|p| DeleteItem {
            path: p.display().to_string(),
            trashed: None,
            error: Some("the machine didn't say".into()),
        })
        .collect();
    for record in out.split(|b| *b == 0) {
        let text = String::from_utf8_lossy(record);
        let mut parts = text.splitn(3, '\t');
        let (Some(tag), Some(n)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Some(item) = n
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| items.get_mut(i))
        else {
            continue;
        };
        item.error = match tag {
            "O" => None,
            _ => Some(parts.next().unwrap_or("couldn't remove it").to_owned()),
        };
    }
    items
}

fn delete_remote(m: &RemoteMachine, paths: &[PathBuf]) -> Result<Vec<DeleteItem>, Error> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let never = AtomicBool::new(false);
    let out = remote_files::run(&m.exec(remove_script(paths)), &never, DELETE_TIMEOUT)
        .map_err(|e| remote_error(m, e))?;
    Ok(parse_removed(&out, paths))
}

/// The folders holding `paths`, once each: what changed when they were
/// made or deleted.
pub(crate) fn parents(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = paths
        .iter()
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(target_os = "macos")]
fn system_trash(path: &Path) -> Result<PathBuf, String> {
    mac::trash(path)
}

#[cfg(not(target_os = "macos"))]
fn system_trash(_path: &Path) -> Result<PathBuf, String> {
    Err("moving to the Trash isn't supported here".into())
}

/// `-[NSFileManager trashItemAtURL:resultingItemURL:error:]` through the
/// Objective-C runtime: what Finder's Move to Trash does, Put Back
/// included. Safe on any thread (`defaultManager`), in a pool of its own.
#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{CStr, OsStr, c_char, c_void};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr::null_mut;

    type Id = *mut c_void;
    type Sel = *mut c_void;

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }

    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {}

    pub(super) fn trash(path: &Path) -> Result<PathBuf, String> {
        // SAFETY: each message goes to an object of the class that declares
        // it, with that method's argument and return types; the pool keeps
        // the autoreleased objects alive until we're done reading them.
        unsafe {
            let pool = objc_autoreleasePoolPush();
            let result = trash_in_pool(path);
            objc_autoreleasePoolPop(pool);
            result
        }
    }

    unsafe fn trash_in_pool(path: &Path) -> Result<PathBuf, String> {
        unsafe {
            let msg = objc_msgSend as unsafe extern "C" fn();
            let sel = |name: &CStr| sel_registerName(name.as_ptr());
            let send: unsafe extern "C" fn(Id, Sel) -> Id = std::mem::transmute(msg);
            let fs_string: unsafe extern "C" fn(Id, Sel, *const c_char, usize) -> Id =
                std::mem::transmute(msg);
            let send_id: unsafe extern "C" fn(Id, Sel, Id) -> Id = std::mem::transmute(msg);
            let trash_item: unsafe extern "C" fn(Id, Sel, Id, *mut Id, *mut Id) -> i8 =
                std::mem::transmute(msg);
            let c_string: unsafe extern "C" fn(Id, Sel) -> *const c_char = std::mem::transmute(msg);

            let manager = send(
                objc_getClass(c"NSFileManager".as_ptr()),
                sel(c"defaultManager"),
            );
            if manager.is_null() {
                return Err("no file manager".into());
            }
            let bytes = path.as_os_str().as_bytes();
            let string = fs_string(
                manager,
                sel(c"stringWithFileSystemRepresentation:length:"),
                bytes.as_ptr().cast(),
                bytes.len(),
            );
            if string.is_null() {
                return Err("that path can't be named".into());
            }
            let url = send_id(
                objc_getClass(c"NSURL".as_ptr()),
                sel(c"fileURLWithPath:"),
                string,
            );
            if url.is_null() {
                return Err("that path can't be named".into());
            }
            let mut out: Id = null_mut();
            let mut error: Id = null_mut();
            let ok = trash_item(
                manager,
                sel(c"trashItemAtURL:resultingItemURL:error:"),
                url,
                &mut out,
                &mut error,
            );
            if ok != 0 {
                let p = if out.is_null() {
                    std::ptr::null()
                } else {
                    c_string(out, sel(c"fileSystemRepresentation"))
                };
                return Ok(if p.is_null() {
                    PathBuf::new()
                } else {
                    PathBuf::from(OsStr::from_bytes(CStr::from_ptr(p).to_bytes()))
                });
            }
            let mut why = "couldn't move it to the Trash".to_owned();
            if !error.is_null() {
                let text = send(error, sel(c"localizedDescription"));
                if !text.is_null() {
                    let p = c_string(text, sel(c"UTF8String"));
                    if !p.is_null() {
                        why = CStr::from_ptr(p).to_string_lossy().into_owned();
                    }
                }
            }
            Err(why)
        }
    }
}

#[cfg(test)]
#[path = "file_ops_tests.rs"]
mod tests;
