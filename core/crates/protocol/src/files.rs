//! Files panels: params, listings and events.
//!
//! A files panel (`files.panel`, the sidebar's Files view) lists one folder
//! at a time. Shells ask for a folder's entries with `files.expand` when the
//! user opens it, which also starts watching it; `files.changed` then names
//! folders to list again.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use azulejo_model::TileId;

/// A folder of a files panel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesDir {
    /// The panel's id (`files.panel`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// Absolute, or relative to the panel's root; the root when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesReveal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// A file or folder under the panel's root: absolute, or relative to
    /// the panel's root.
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesToggleHidden {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// Show (true) or hide (false) hidden and ignored entries; toggles when
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show: Option<bool>,
}

/// Where `files.open` shows a path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FilesOpenTarget {
    /// A file opens in the editor: a new terminal tile to the right of the
    /// anchor, in the file's folder, runs the editor on it (`vim <path>`).
    /// It takes focus. A folder opens a terminal there instead, as
    /// `terminal` does. `preview`, from builds that had preview tiles, means
    /// this too.
    #[default]
    #[serde(alias = "preview")]
    Editor,
    /// A new terminal tile below the anchor, in the folder (a file's folder
    /// for a file). It takes focus.
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesOpen {
    /// The files panel (`files.panel`) the path comes from: the path opens
    /// beside the focused tile. Any tile works as the anchor instead when
    /// `path` is absolute; the focused tile when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// Absolute, or relative to the panel's root.
    pub path: String,
    #[serde(default)]
    pub target: FilesOpenTarget,
}

/// A files panel: a file tree outside the layout (the sidebar's Files view).
/// It answers `files.list`, `files.expand`, `files.collapse`,
/// `files.reveal`, `files.toggle_hidden` and `files.open` with its id as
/// `tile`, and its events carry that id.
///
/// With `machine`, the panel shows that herdr machine's files:
/// the core lists folders over the machine's SSH connection, off its
/// thread. A folder it hasn't listed yet comes back empty with `loading`,
/// and `files.changed` names the folder once its listing is in (or failed,
/// with `error`). Open folders are listed again every few seconds; nothing
/// is watched. `files.open` opens the editor and terminals on the focused
/// workspace's machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesPanel {
    /// The panel to point at `root`; a new panel when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<TileId>,
    /// The folder to show; `~` expands to the home folder. On a remote
    /// machine, an absolute path there.
    pub root: String,
    /// A herdr machine's ID (`machines.list`) whose files to show; Local
    /// when absent or `local`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesPanelClose {
    pub panel: TileId,
}

/// A path's git state, for a badge. Folders take the strongest state of the
/// changes beneath them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileGitState {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

/// One entry of a listed folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileEntry {
    pub name: String,
    /// Absolute.
    pub path: String,
    /// A folder, or a symlink to one.
    pub dir: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub symlink: bool,
    /// Starts with a dot. Only listed when hidden entries are shown.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Excluded by gitignore rules. Only listed when hidden entries are shown.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ignored: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<FileGitState>,
}

/// A folder's entries, from `files.list` or `files.expand`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FilesListing {
    pub tile: TileId,
    /// The panel's root, absolute and resolved (symlinks followed).
    pub root: String,
    /// The listed folder, absolute and resolved.
    pub dir: String,
    pub show_hidden: bool,
    /// Folders first, then natural name order (`a2` before `a10`).
    pub entries: Vec<FileEntry>,
    /// The folder had more entries than a listing returns; the rest are cut.
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    /// The herdr machine the files are on; absent for Local.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// A remote folder the core hasn't listed yet: `entries` is empty and
    /// `files.changed` names the folder when its listing is in.
    #[serde(default, skip_serializing_if = "is_false")]
    pub loading: bool,
    /// Why a remote folder couldn't be listed (the machine is unreachable,
    /// the folder is gone). `entries` then holds the last listing, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Payload of `files.changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FilesChanged {
    pub tile: TileId,
    /// Folders whose entries changed: list them again. Absolute, resolved.
    pub dirs: Vec<String>,
}

/// Payload of `files.revealed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FilesRevealed {
    pub tile: TileId,
    /// The entry to select. Absolute, resolved.
    pub path: String,
    /// The folders to expand, root first.
    pub dirs: Vec<String>,
}

/// Payload of `files.git`: the git state of every changed path in the
/// panel's work tree (and the folders above them). Replaces the previous one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FilesGit {
    pub tile: TileId,
    pub work_tree: String,
    pub paths: BTreeMap<String, FileGitState>,
}

fn is_false(b: &bool) -> bool {
    !*b
}
