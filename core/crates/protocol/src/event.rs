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
