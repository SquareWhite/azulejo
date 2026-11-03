//! Typed views of herdr objects. Fields Azulejo doesn't use are ignored, so
//! newer herdr versions that add fields still parse.

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }
    };
}

id_type!(
    /// herdr workspace ID, e.g. `w1`.
    WorkspaceId
);
id_type!(
    /// herdr tab ID, e.g. `w1:t2`.
    TabId
);
id_type!(
    /// herdr pane ID, e.g. `w1:p2`. Changes when a pane moves to another workspace.
    PaneId
);
id_type!(
    /// herdr terminal ID, e.g. `term_0123456789abcd`. What `terminal attach` takes.
    TerminalId
);

/// Agent lifecycle state herdr reports per pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScrollInfo {
    pub offset_from_bottom: u64,
    pub max_offset_from_bottom: u64,
    pub viewport_rows: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneInfo {
    pub pane_id: PaneId,
    pub terminal_id: TerminalId,
    pub workspace_id: WorkspaceId,
    pub tab_id: TabId,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    #[serde(default)]
    pub terminal_title: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_status: AgentStatus,
    #[serde(default)]
    pub scroll: Option<ScrollInfo>,
    #[serde(default)]
    pub revision: u64,
}

impl PaneInfo {
    pub fn terminal_ref(&self) -> TerminalRef {
        TerminalRef {
            workspace_id: self.workspace_id.clone(),
            tab_id: self.tab_id.clone(),
            pane_id: self.pane_id.clone(),
            terminal_id: self.terminal_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabInfo {
    pub tab_id: TabId,
    pub workspace_id: WorkspaceId,
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub pane_count: u32,
    #[serde(default)]
    pub agent_status: AgentStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub workspace_id: WorkspaceId,
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub pane_count: u32,
    #[serde(default)]
    pub tab_count: u32,
    #[serde(default)]
    pub active_tab_id: Option<TabId>,
    #[serde(default)]
    pub agent_status: AgentStatus,
}

/// Everything herdr knows, from `session.snapshot`. Used to resync after a
/// reconnect or lost events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: String,
    pub protocol: u32,
    #[serde(default)]
    pub focused_workspace_id: Option<WorkspaceId>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceInfo>,
    #[serde(default)]
    pub tabs: Vec<TabInfo>,
    #[serde(default)]
    pub panes: Vec<PaneInfo>,
}

/// Which way a herdr split places its second child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirection {
    /// `second` sits to the right of `first`.
    Right,
    /// `second` sits below `first`.
    Down,
}

/// One node of a herdr tab's split tree, from `layout.export`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    Pane {
        #[serde(default)]
        pane_id: Option<PaneId>,
    },
    Split {
        direction: SplitDirection,
        /// Share of the space given to `first`.
        ratio: f64,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

impl LayoutNode {
    /// Pane IDs at the leaves, left to right.
    pub fn panes(&self) -> Vec<&PaneId> {
        match self {
            LayoutNode::Pane { pane_id } => pane_id.iter().collect(),
            LayoutNode::Split { first, second, .. } => {
                let mut out = first.panes();
                out.extend(second.panes());
                out
            }
        }
    }
}

/// A tab's split tree, from `layout.export`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabLayout {
    pub workspace_id: WorkspaceId,
    pub tab_id: TabId,
    #[serde(default)]
    pub zoomed: bool,
    #[serde(default)]
    pub focused_pane_id: Option<PaneId>,
    pub root: LayoutNode,
}

/// A read-only picture of a herdr server: the snapshot plus the split tree of
/// every tab with more than one pane. What Azulejo adopts from (ADR 0008).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Survey {
    pub workspaces: Vec<WorkspaceInfo>,
    pub tabs: Vec<TabInfo>,
    pub panes: Vec<PaneInfo>,
    pub layouts: Vec<TabLayout>,
    pub focused_workspace_id: Option<WorkspaceId>,
}

/// What `ping` reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub version: String,
    pub protocol: u32,
}

/// The IDs a terminal tile stores. Re-resolve `pane_id` from `terminal_id`
/// after a `pane.moved` event and on relaunch (pane IDs change on moves).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TerminalRef {
    pub workspace_id: WorkspaceId,
    pub tab_id: TabId,
    pub pane_id: PaneId,
    pub terminal_id: TerminalId,
}

/// Result of [`crate::Client::reresolve`].
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    /// Same process, same IDs.
    Same(PaneInfo),
    /// Same process (terminal ID), but the pane moved: store the new IDs.
    Moved(PaneInfo),
    /// The pane slot survived a herdr server restart but the process didn't:
    /// a fresh shell with a new terminal ID and none of the tile's env vars.
    Respawned(PaneInfo),
    /// Nothing left; the tile should offer Respawn or Close.
    Gone,
}

/// Which buffer `read` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadSource {
    /// The visible screen.
    Visible,
    /// Recent output including scrollback, soft-wrapped as on screen.
    Recent,
    /// Recent output with soft wraps joined, for log-like text.
    RecentUnwrapped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadResult {
    pub text: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub truncated: bool,
}

/// Values Azulejo injects into every terminal tile's environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileEnv {
    pub socket: String,
    pub tile_id: String,
    pub workspace_id: String,
}

impl TileEnv {
    pub fn to_pairs(&self) -> Vec<(String, String)> {
        vec![
            ("AZULEJO_SOCKET".into(), self.socket.clone()),
            ("AZULEJO_TILE_ID".into(), self.tile_id.clone()),
            ("AZULEJO_WORKSPACE_ID".into(), self.workspace_id.clone()),
        ]
    }
}

/// How to create a herdr workspace for an app workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceSpec {
    pub label: Option<String>,
    pub cwd: Option<String>,
    /// Environment for the workspace's first terminal (herdr always creates one).
    pub env: Vec<(String, String)>,
}

/// How to create the herdr tab behind a terminal tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSpec {
    pub workspace_id: WorkspaceId,
    pub label: Option<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
}

/// Result of [`crate::Client::ensure_workspace`].
#[derive(Debug, Clone, PartialEq)]
pub struct EnsuredWorkspace {
    pub workspace: WorkspaceInfo,
    /// Set when herdr created the workspace just now: the terminal herdr made
    /// with it. Use it for the first terminal tile, or kill it.
    pub created_terminal: Option<TerminalRef>,
}
