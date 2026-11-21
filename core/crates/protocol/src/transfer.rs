//! Moving and copying files into a folder, here or on a herdr machine:
//! `files.transfer`. Shells use it for drops (a terminal's folder) and the
//! file tree; the core does the work off its thread and replies when it's
//! done, one result per source. `files.create` and `files.delete` make and
//! delete entries the same way, here or on a machine.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What `files.transfer` does with the originals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransferMode {
    /// The originals stay.
    #[default]
    Copy,
    /// The originals go once their copy is in place (between machines: once
    /// the upload or download succeeded).
    Move,
}

/// Moves or copies files and folders into a folder. Nothing is ever
/// overwritten: a name already taken there gets a Finder-style number
/// (`notes 2.txt`, `notes 3.txt`, …).
///
/// Between machines it runs `scp` over the machine's shared SSH
/// connection: Local to a herdr machine (upload) or back (download). Both
/// ends on one remote machine, or on two, isn't supported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesTransfer {
    /// Absolute paths of the files and folders, on `from`.
    pub sources: Vec<String>,
    /// The machine the sources are on (`machines.list` ID); Local when
    /// absent or `local`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The folder they go into: absolute, on `to`.
    pub folder: String,
    /// The machine the folder is on; Local when absent or `local`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default)]
    pub mode: TransferMode,
}

/// What became of one source of `files.transfer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TransferItem {
    pub source: String,
    /// Where it is now, on the folder's machine; absent when it failed.
    /// Moving something into the folder it's already in leaves it there
    /// (`dest` is `source`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dest: Option<String>,
    /// Why it failed. With `dest` set, the copy is in place but something
    /// after it failed (a move's original couldn't be removed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What `files.create` makes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    /// An empty file.
    #[default]
    File,
    /// An empty folder.
    Folder,
}

/// Makes an empty file or folder. Nothing is ever replaced: a name that's
/// taken (by a file, a folder or a link) fails with `invalid_params`, and
/// so does a folder that isn't there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesCreate {
    /// A files panel (`files.panel`): `path` may be relative to its root,
    /// and a remote panel's entry is made on its machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<azulejo_model::TileId>,
    /// The new entry: absolute, or relative to the panel's root. Its last
    /// part is the name (not `.` or `..`).
    pub path: String,
    #[serde(default)]
    pub kind: FileKind,
    /// The machine to make it on (`machines.list` ID); Local when absent or
    /// `local`. Leave it out with `tile`: a panel's entries are on its own
    /// machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// Deletes files and folders. On Local they go to the Trash (Finder's Put
/// Back works); on a herdr machine they're removed for good (`rm -rf` over
/// its SSH connection), so shells ask first. With `tile`, every path must
/// be inside that panel's root (not the root itself).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesDelete {
    /// A files panel (`files.panel`): paths may be relative to its root,
    /// and a remote panel's are on its machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<azulejo_model::TileId>,
    /// Absolute, or relative to the panel's root.
    pub paths: Vec<String>,
    /// The machine the paths are on (`machines.list` ID); Local when absent
    /// or `local`. Leave it out with `tile`: a panel's entries are on its
    /// own machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// What became of one path of `files.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeleteItem {
    pub path: String,
    /// Where it is in the Trash (Local only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trashed: Option<String>,
    /// Why it couldn't be deleted; absent when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
