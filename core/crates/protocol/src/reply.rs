//! What a successful command returns to its caller. Changes are reported
//! separately, as events.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use azulejo_model::{Frame, Session, TabId, Tile, TileId, WindowId, WorkspaceId};

use crate::event::{LayoutState, PreviewContent, TabInfo, WorkspaceInfo};

// One reply per request, so an unboxed tile (with its header and pane
// metadata) is fine.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    /// Done; nothing to return.
    Ok,
    Pong {
        api_version: u32,
    },
    Session {
        session: Session,
    },
    Layout(LayoutState),
    /// IDs of what the command created.
    Created(Created),
    Tiles {
        tiles: Vec<TileInfo>,
    },
    Tile(TileInfo),
    Text {
        text: String,
    },
    Windows {
        windows: Vec<WindowSummary>,
    },
    Workspaces {
        workspaces: Vec<WorkspaceSummary>,
    },
    Tabs {
        tabs: Vec<TabSummary>,
    },
    /// Where a terminal's view sits after `terminal.scroll`.
    Scroll(ScrollState),
    /// A preview tile's content, from `preview.get`.
    Preview(PreviewContent),
    /// A folder of a files tile, from `files.list` or `files.expand`.
    Files(crate::files::FilesListing),
    /// Lines that matched `terminal.search`.
    Matches {
        matches: Vec<SearchMatch>,
        /// herdr cut the history short; older lines weren't searched.
        #[serde(default)]
        truncated: bool,
    },
    /// An event stream started; notifications carry this `subscription`.
    Subscribed {
        subscription: u64,
    },
    /// A layout file, from `layout.export`.
    LayoutFile {
        toml: String,
        /// Where it was written, if `path` was given.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// The command palette's entries, from `commands.list`.
    Commands {
        commands: Vec<crate::project::CommandEntry>,
    },
}

/// A window without its workspaces' contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WindowSummary {
    pub id: WindowId,
    pub focused: bool,
    pub active_workspace: WorkspaceId,
    pub workspaces: Vec<WorkspaceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Frame>,
}

/// A workspace and where it lives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceSummary {
    pub window: WindowId,
    /// The window's active workspace.
    pub active: bool,
    /// Active in the focused window.
    pub focused: bool,
    pub tabs: usize,
    #[serde(flatten)]
    pub workspace: WorkspaceInfo,
}

/// A tab and where it lives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TabSummary {
    pub window: WindowId,
    pub workspace: WorkspaceId,
    /// The workspace's active tab.
    pub active: bool,
    /// The focused tab.
    pub focused: bool,
    pub tiles: usize,
    pub focused_tile: TileId,
    #[serde(flatten)]
    pub tab: TabInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScrollState {
    pub offset_from_bottom: u64,
    pub max_offset_from_bottom: u64,
    pub viewport_rows: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SearchMatch {
    /// 0-based line in the searched history, oldest first.
    pub line: usize,
    pub text: String,
}

/// IDs of what a command created; only the relevant fields are set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Created {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<TabId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
}

/// A tile and where it lives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TileInfo {
    pub window: WindowId,
    pub workspace: WorkspaceId,
    pub tab: TabId,
    /// Whether this is the focused tile.
    pub focused: bool,
    #[serde(flatten)]
    pub tile: Tile,
}
