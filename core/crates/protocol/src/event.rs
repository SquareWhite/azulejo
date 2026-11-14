//! Every change, as an event.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use azulejo_model::{
    Focus, Frame, LayoutKind, LayoutParams, Node, Placement, Tab, TabId, Tile, TileId, WindowId,
    WorkspaceId,
};

/// Something that changed in the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", content = "data")]
pub enum Event {
    #[serde(rename = "window.created")]
    WindowCreated { window: WindowId },
    #[serde(rename = "window.closed")]
    WindowClosed { window: WindowId },
    /// Frame or sidebar changed.
    #[serde(rename = "window.updated")]
    WindowUpdated {
        window: WindowId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frame: Option<Frame>,
        #[serde(default)]
        sidebar_hidden: bool,
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
    /// A terminal tile's agent started or stopped needing a look: it became
    /// blocked or finished, or someone focused the tile (F11). Also visible as
    /// `tile.agent.attention` on `tile.updated`.
    #[serde(rename = "tile.attention_changed")]
    TileAttentionChanged {
        tab: TabId,
        tile: TileId,
        attention: bool,
    },

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

    /// Folders a files tile has open changed on disk, or hidden entries were
    /// toggled: list them again. Not saved.
    #[serde(rename = "files.changed")]
    FilesChanged(crate::files::FilesChanged),
    /// `files.reveal` opened the folders down to an entry: shells expand them
    /// and select it. Not saved.
    #[serde(rename = "files.revealed")]
    FilesRevealed(crate::files::FilesRevealed),
    /// Git status for a files tile's work tree, after it opens and after
    /// changes. Not saved.
    #[serde(rename = "files.git")]
    FilesGit(crate::files::FilesGit),

    /// The core's link to herdr came up or dropped. After `connected: true`,
    /// shells reattach terminal tiles that gave up waiting. Not saved.
    #[serde(rename = "herdr.status")]
    HerdrStatus {
        connected: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// The agent in a terminal tile changed state, as herdr reports it (F16).
    /// Not saved.
    #[serde(rename = "agent.state_changed")]
    AgentStateChanged(AgentStateChange),

    /// The config file changed on disk and was read again (F25b). When it has
    /// errors the previous config stays and `applied` is false. Not saved.
    #[serde(rename = "config.reloaded")]
    ConfigReloaded(crate::project::ConfigReloaded),
}

/// What the agent in a terminal tile is doing, per herdr.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Idle,
    Working,
    /// Waiting for the user, e.g. a permission prompt.
    Blocked,
    Done,
    /// herdr doesn't know, or no agent runs in the pane.
    #[default]
    #[serde(other)]
    Unknown,
}

/// Payload of `agent.state_changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentStateChange {
    pub tile: TileId,
    pub state: AgentState,
    /// The state before this change, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<AgentState>,
    /// The agent herdr detected, e.g. `claude` or `codex`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
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
///
/// Shells draw from `rects`: every tile's rectangle in the unit square, its
/// stacking order and whether it's shown (Monocle hides all but the focused
/// tile without resizing them). `tree` is the tab's split tree: the source of
/// truth for manual tabs, derived for dynamic ones (ADR 0010).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutState {
    pub window: WindowId,
    pub workspace: WorkspaceId,
    pub tab: TabId,
    pub tree: Node,
    pub focused: TileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoomed: Option<TileId>,
    #[serde(default)]
    pub kind: LayoutKind,
    /// Every kind's parameters, as the tab remembers them.
    #[serde(default)]
    pub params: LayoutParams,
    /// Tile order of a dynamic tab, masters first; empty for manual tabs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<TileId>,
    #[serde(default)]
    pub rects: Vec<Placement>,
}

impl LayoutState {
    /// The layout of `tab`, which lives in `window` › `workspace`.
    pub fn of(window: WindowId, workspace: WorkspaceId, tab: &Tab) -> Self {
        Self {
            window,
            workspace,
            tab: tab.id,
            tree: tab.root.clone(),
            focused: tab.focused,
            zoomed: tab.zoomed,
            kind: tab.layout.kind,
            params: tab.layout.params,
            order: tab.layout.order.clone(),
            rects: tab.placements(),
        }
    }
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
    /// Terminal tiles whose agent is blocked, for the sidebar (F16).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub blocked: u32,
    /// Terminal tiles whose agent needs a look (F11).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attention: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A tab without its tiles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TabInfo {
    pub id: TabId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The tab's layout kind, for a subtle hint in the tab bar.
    #[serde(default)]
    pub kind: LayoutKind,
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
