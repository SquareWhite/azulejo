//! What a successful command returns to its caller. Changes are reported
//! separately, as events.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use azulejo_model::{Session, TabId, Tile, TileId, WindowId, WorkspaceId};

use crate::event::LayoutState;

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
