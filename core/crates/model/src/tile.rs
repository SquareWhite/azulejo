//! Tiles and what they show.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::TileId;

/// The kind of content a tile shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TileKind {
    Terminal,
    Browser,
    Preview,
    Files,
}

/// The herdr pane a terminal tile views. Both IDs are kept: herdr changes a
/// pane's ID when it moves to another workspace, while the terminal ID is what
/// `herdr terminal attach` takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HerdrRef {
    pub pane_id: String,
    pub terminal_id: String,
}

/// A tile's content and its kind-specific state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TileContent {
    Terminal {
        /// Working directory for a new pane.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        /// Command to run instead of the user's shell.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<Vec<String>>,
        /// Set once the herdr adapter has created or adopted a pane.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        herdr: Option<HerdrRef>,
    },
    Browser {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
    Preview {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    Files {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        root: Option<String>,
    },
}

impl TileContent {
    pub fn kind(&self) -> TileKind {
        match self {
            TileContent::Terminal { .. } => TileKind::Terminal,
            TileContent::Browser { .. } => TileKind::Browser,
            TileContent::Preview { .. } => TileKind::Preview,
            TileContent::Files { .. } => TileKind::Files,
        }
    }

    /// An empty terminal in the default directory.
    pub fn terminal() -> Self {
        TileContent::Terminal {
            cwd: None,
            command: None,
            herdr: None,
        }
    }
}

/// One tile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tile {
    pub id: TileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(flatten)]
    pub content: TileContent,
}

impl Tile {
    pub fn new(content: TileContent) -> Self {
        Self {
            id: TileId::new(),
            title: None,
            content,
        }
    }

    pub fn kind(&self) -> TileKind {
        self.content.kind()
    }
}
