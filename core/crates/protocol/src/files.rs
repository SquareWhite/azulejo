//! File browser tiles (F10): params, listings and events.
//!
//! A files tile lists one folder at a time. Shells ask for a folder's entries
//! with `files.expand` when the user opens it, which also starts watching it;
//! `files.changed` then names folders to list again.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use azulejo_model::TileId;

/// A folder of a files tile.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesDir {
    /// A files tile; defaults to the focused tile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// Absolute, or relative to the tile's root; the root when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesSetRoot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// The folder to show; `~` expands to the home folder.
    pub root: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesReveal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// A file or folder under the tile's root: absolute or relative to it.
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
    /// The tab's first preview tile shows it; without one, a new preview tile
    /// opens to the right of the files tile. Focus stays in the files tile.
    #[default]
    Preview,
    /// A new terminal tile below the files tile, in the folder (a file's
    /// folder for a file). It takes focus.
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesOpen {
    /// The files tile the path comes from; defaults to the focused tile. A
    /// files panel (`files.panel`) works too: the path then opens beside the
    /// focused tile. Any other tile is used as the anchor when `path` is
    /// absolute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// Absolute, or relative to the tile's root.
    pub path: String,
    #[serde(default)]
    pub target: FilesOpenTarget,
}

/// A files panel: a file tree outside the layout (the sidebar's Files view).
/// It answers `files.list`, `files.expand`, `files.collapse`,
/// `files.reveal`, `files.toggle_hidden` and `files.open` like a files tile,
/// with its id as `tile`, and its events carry that id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilesPanel {
    /// The panel to point at `root`; a new panel when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<TileId>,
    /// The folder to show; `~` expands to the home folder.
    pub root: String,
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
    /// The tile's root, absolute and resolved (symlinks followed).
    pub root: String,
    /// The listed folder, absolute and resolved.
    pub dir: String,
    pub show_hidden: bool,
    /// Folders first, then natural name order (`a2` before `a10`).
    pub entries: Vec<FileEntry>,
    /// The folder had more entries than a listing returns; the rest are cut.
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
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
/// tile's work tree (and the folders above them). Replaces the previous one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FilesGit {
    pub tile: TileId,
    pub work_tree: String,
    pub paths: BTreeMap<String, FileGitState>,
}

fn is_false(b: &bool) -> bool {
    !*b
}
