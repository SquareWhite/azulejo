//! Every change, as an event.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use azulejo_model::{Focus, Frame, Node, TabId, Tile, TileId, WindowId, WorkspaceId};

/// Something that changed in the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", content = "data")]
pub enum Event {
    #[serde(rename = "window.created")]
    WindowCreated { window: WindowId },
    #[serde(rename = "window.closed")]
    WindowClosed { window: WindowId },
    #[serde(rename = "window.updated")]
    WindowUpdated {
        window: WindowId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frame: Option<Frame>,
    },

    #[serde(rename = "workspace.created")]
    WorkspaceCreated {
        window: WindowId,
        workspace: WorkspaceInfo,
    },
    #[serde(rename = "workspace.closed")]
    WorkspaceClosed {
        window: WindowId,
        workspace: WorkspaceId,
    },
    /// Name, cwd or herdr binding changed.
    #[serde(rename = "workspace.updated")]
    WorkspaceUpdated {
        window: WindowId,
        workspace: WorkspaceInfo,
    },
    /// A window's workspaces changed order.
    #[serde(rename = "workspace.reordered")]
    WorkspacesReordered {
        window: WindowId,
        order: Vec<WorkspaceId>,
    },

    #[serde(rename = "tab.created")]
    TabCreated {
        workspace: WorkspaceId,
        tab: TabInfo,
    },
    #[serde(rename = "tab.closed")]
    TabClosed { workspace: WorkspaceId, tab: TabId },
    #[serde(rename = "tab.updated")]
    TabUpdated {
        workspace: WorkspaceId,
        tab: TabInfo,
    },
    /// A workspace's tabs changed order.
    #[serde(rename = "tab.reordered")]
    TabsReordered {
        workspace: WorkspaceId,
        order: Vec<TabId>,
    },

    #[serde(rename = "tile.created")]
    TileCreated { tab: TabId, tile: Tile },
    /// A tile's title or content changed (e.g. a terminal got its herdr pane).
    #[serde(rename = "tile.updated")]
    TileUpdated { tab: TabId, tile: Tile },
    /// A tile closed. `kill` asks the herdr adapter to kill the terminal's pane
    /// rather than detach from it.
    #[serde(rename = "tile.closed")]
    TileClosed { tab: TabId, tile: Tile, kill: bool },

    /// A tab's tree, focused tile or zoom changed.
    #[serde(rename = "layout.changed")]
    LayoutChanged(LayoutState),

    /// The focused window, workspace, tab or tile changed.
    #[serde(rename = "focus.changed")]
    FocusChanged(Focus),

    /// A preview tile's file was (re)rendered: on open, on restore and after
    /// every change on disk. Not saved; `preview.get` returns the latest.
    #[serde(rename = "preview.updated")]
    PreviewUpdated(PreviewContent),
}

/// What a preview tile shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PreviewKind {
    Markdown,
    Code,
    Text,
    /// Native kinds: the shell shows `path` with QuickLook (macOS) or GTK
    /// (Linux); there's no `html`.
    Image,
    Video,
    Audio,
    Pdf,
    /// The rest get an explanatory `html` page.
    Binary,
    Directory,
    Missing,
    Unreadable,
}

/// A rendered preview, from `preview.updated` or `preview.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PreviewContent {
    pub tile: TileId,
    pub path: String,
    pub kind: PreviewKind,
    /// A complete HTML document for the tile's webview; absent for native
    /// kinds. When the tile already shows `path`, pass it to
    /// `azulejoPreview.swap(html)` (shared/webview/preview.js) to keep the
    /// scroll position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    /// Syntax name for `code`, e.g. `Rust`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Only the start of a very large file was rendered.
    #[serde(default)]
    pub truncated: bool,
    /// Counts renders of this tile in this run, so a shell can drop stale
    /// updates.
    pub revision: u64,
}

/// A tab's layout, as `layout.changed` and `layout.get` report it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutState {
    pub window: WindowId,
    pub workspace: WorkspaceId,
    pub tab: TabId,
    pub tree: Node,
    pub focused: TileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoomed: Option<TileId>,
}

/// A workspace without its tabs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceInfo {
    pub id: WorkspaceId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub herdr_workspace: Option<String>,
}

/// A tab without its tiles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TabInfo {
    pub id: TabId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Event {
    /// This event's name, e.g. `layout.changed`.
    pub fn name(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.get("event").and_then(|e| e.as_str()).map(str::to_owned))
            .unwrap_or_default()
    }
}
