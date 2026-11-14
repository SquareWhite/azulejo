//! Tiles and what they show.

use std::collections::BTreeMap;

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
/// `herdr terminal attach` takes. The tab ID matters because closing a herdr
/// tab reports only the tab, and herdr keeps pane and tab IDs (but not the
/// process) across a server restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HerdrRef {
    pub pane_id: String,
    pub terminal_id: String,
    pub tab_id: String,
}

/// A tile's content and its kind-specific state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TileContent {
    Terminal {
        /// Working directory for a new pane.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        /// Command typed into the pane's shell once it starts. One element is
        /// sent as a shell line (`npm run dev`); more are an argv, quoted for
        /// the shell. herdr has no way to replace the shell, so the shell stays
        /// when the command exits.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<Vec<String>>,
        /// Extra environment for a new pane, on top of `AZULEJO_*`.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
        /// Set once the herdr adapter has created or adopted a pane.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        herdr: Option<HerdrRef>,
        /// The herdr pane is gone (closed elsewhere, or lost with its server).
        /// The shell offers Respawn or Close; binding a new pane clears it.
        #[serde(default, skip_serializing_if = "is_false")]
        ended: bool,
    },
    Browser {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        /// The loaded page's `<title>`, as the shell last reported it
        /// (`browser.state`). Separate from the tile's own `title`, which is
        /// the user's name for the tile.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page_title: Option<String>,
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
            env: BTreeMap::new(),
            herdr: None,
            ended: false,
        }
    }
}

pub(crate) fn is_false(b: &bool) -> bool {
    !*b
}

/// What herdr says the agent in a terminal tile is doing (F16).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    /// Waiting for the user, e.g. a permission prompt.
    Blocked,
    Done,
    #[default]
    #[serde(other)]
    Unknown,
}

impl AgentStatus {
    /// Statuses that ask for a look: the agent waits, or it finished.
    pub fn wants_attention(self) -> bool {
        matches!(self, AgentStatus::Blocked | AgentStatus::Done)
    }
}

/// The agent running in a terminal tile, from herdr's agent status (F11,
/// F16). Absent when herdr knows of no agent there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentInfo {
    /// The agent herdr detected or was told about, e.g. `claude`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub status: AgentStatus,
    /// When `status` last changed, in milliseconds since the Unix epoch.
    pub since_ms: u64,
    /// The agent became blocked or finished and nobody has looked since:
    /// focusing the tile, or `tile.seen`, clears it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub attention: bool,
}

/// One tile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tile {
    pub id: TileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(flatten)]
    pub content: TileContent,
    /// Terminal tiles only: the agent herdr reports in the pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentInfo>,
}

impl Tile {
    pub fn new(content: TileContent) -> Self {
        Self {
            id: TileId::new(),
            title: None,
            content,
            agent: None,
        }
    }

    /// Whether the tile's agent needs a look (F11).
    pub fn needs_attention(&self) -> bool {
        self.agent.as_ref().is_some_and(|a| a.attention)
    }

    pub fn kind(&self) -> TileKind {
        self.content.kind()
    }
}
