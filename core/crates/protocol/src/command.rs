//! Every user action, as a command. Shells send the same commands the public
//! API takes, so anything the user can do is scriptable by construction.
//!
//! Most commands act on the focused tile, tab or workspace when no target is
//! given.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use azulejo_model::{
    Direction, Frame, HerdrRef, TabId, TileContent, TileId, WindowId, WorkspaceId,
};

use crate::error::{Error, ErrorCode};

/// A request to the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "method", content = "params")]
pub enum Command {
    /// Liveness check; replies `pong`.
    #[serde(rename = "core.ping")]
    Ping,

    // ----- session (F1, F24) -----
    /// The whole object model; replies `session`.
    #[serde(rename = "session.get")]
    SessionGet,

    // ----- windows (F18) -----
    /// Opens a window with one empty workspace.
    #[serde(rename = "window.create")]
    WindowCreate(WindowCreate),
    #[serde(rename = "window.close")]
    WindowClose(WindowTarget),
    #[serde(rename = "window.focus")]
    WindowFocus(WindowTarget),
    /// Records a window's frame after the user moves or resizes it.
    #[serde(rename = "window.set_frame")]
    WindowSetFrame(WindowSetFrame),
    /// Shows or hides a window's workspace sidebar (restored on relaunch).
    #[serde(rename = "window.set_sidebar")]
    WindowSetSidebar(WindowSetSidebar),
    /// Every window; replies `windows`.
    #[serde(rename = "window.list")]
    WindowList,

    // ----- workspaces (F1, F13, F18) -----
    /// Workspaces of one window, or of all windows; replies `workspaces`.
    #[serde(rename = "workspace.list")]
    WorkspaceList(WorkspaceList),
    #[serde(rename = "workspace.create")]
    WorkspaceCreate(WorkspaceCreate),
    #[serde(rename = "workspace.close")]
    WorkspaceClose(WorkspaceTarget),
    #[serde(rename = "workspace.focus")]
    WorkspaceFocus(WorkspaceTarget),
    #[serde(rename = "workspace.rename")]
    WorkspaceRename(WorkspaceRename),
    #[serde(rename = "workspace.move")]
    WorkspaceMove(WorkspaceMove),
    /// Records the herdr workspace backing a workspace. Sent by the herdr adapter.
    #[serde(rename = "workspace.bind")]
    WorkspaceBind(WorkspaceBind),

    // ----- tabs (F1, F18) -----
    /// Tabs of one workspace, or of all workspaces; replies `tabs`.
    #[serde(rename = "tab.list")]
    TabList(TabList),
    /// Opens a tab with one tile (a terminal unless `content` says otherwise).
    #[serde(rename = "tab.create")]
    TabCreate(TabCreate),
    #[serde(rename = "tab.close")]
    TabClose(TabTarget),
    #[serde(rename = "tab.focus")]
    TabFocus(TabTarget),
    #[serde(rename = "tab.rename")]
    TabRename(TabRename),
    #[serde(rename = "tab.move")]
    TabMove(TabMove),

    // ----- tiles (F12, F14, F18) -----
    /// Lists tiles; replies `tiles`.
    #[serde(rename = "tile.list")]
    TileList(TileList),
    /// One tile; replies `tile`.
    #[serde(rename = "tile.get")]
    TileGet(TileTarget),
    /// Closes a tile. A terminal's herdr pane is detached, not killed, unless
    /// `kill` is set (F14).
    #[serde(rename = "tile.close")]
    TileClose(TileClose),
    /// Focuses a tile; counts as looking at it, so its agent's attention
    /// clears (F11).
    #[serde(rename = "tile.focus")]
    TileFocus(TileTarget),
    /// Clears a tile's agent attention without moving focus: shells send it
    /// when a focused tile's attention lights up while the user is looking.
    #[serde(rename = "tile.seen")]
    TileSeen(TileTarget),
    /// Sets or clears a tile's title.
    #[serde(rename = "tile.rename")]
    TileRename(TileRename),

    // ----- browser and preview tiles (F8, F9) -----
    /// Points a browser tile at `url`; the shell loads it on `tile.updated`.
    #[serde(rename = "browser.open")]
    BrowserOpen(BrowserOpen),
    /// The shell reports where a browser tile's page is now (after a link,
    /// a redirect, back or forward) and its `<title>`, so `tile.get` and
    /// restore see the live page. Unlike `browser.open` it asks for no load.
    #[serde(rename = "browser.state")]
    BrowserState(BrowserState),
    /// Shows `path` in a preview tile.
    #[serde(rename = "preview.open")]
    PreviewOpen(PreviewOpen),
    /// A preview tile's rendered content; replies `preview`. Shells call it
    /// when they create a preview view; later changes arrive as
    /// `preview.updated` events.
    #[serde(rename = "preview.get")]
    PreviewGet(TileTarget),

    // ----- file browser tiles (F10) -----
    /// Lists one folder of a files tile (its root when `dir` is absent);
    /// replies `files`. Doesn't watch it: see `files.expand`.
    #[serde(rename = "files.list")]
    FilesList(crate::files::FilesDir),
    /// Lists a folder and watches it from now on: changes arrive as
    /// `files.changed`. Shells send it when the user opens a folder.
    #[serde(rename = "files.expand")]
    FilesExpand(crate::files::FilesDir),
    /// Stops watching a folder and the folders open below it.
    #[serde(rename = "files.collapse")]
    FilesCollapse(crate::files::FilesDir),
    /// Points a files tile at another folder.
    #[serde(rename = "files.set_root")]
    FilesSetRoot(crate::files::FilesSetRoot),
    /// Opens the folders down to `path` and asks shells to select it
    /// (`files.revealed`).
    #[serde(rename = "files.reveal")]
    FilesReveal(crate::files::FilesReveal),
    /// Shows or hides hidden and gitignored entries; toggles when `show` is
    /// absent. Shells list their open folders again on `files.changed`.
    #[serde(rename = "files.toggle_hidden")]
    FilesToggleHidden(crate::files::FilesToggleHidden),
    /// Opens a path from a files tile in a preview tile (the tab's first one,
    /// or a new one to the right) or in a new terminal below it; replies
    /// `created` with the tile.
    #[serde(rename = "files.open")]
    FilesOpen(crate::files::FilesOpen),

    // ----- layout (F2) -----
    /// The focused tab's layout; emits `layout.changed` and replies `layout`.
    #[serde(rename = "layout.get")]
    LayoutGet,
    /// Splits a tile and opens a new one beside it, which takes focus.
    #[serde(rename = "layout.split")]
    LayoutSplit(LayoutSplit),
    /// Moves focus to the neighbouring tile.
    #[serde(rename = "layout.focus")]
    LayoutFocus(LayoutFocus),
    /// Swaps a tile with its neighbour in `direction`, or with tile `with`.
    #[serde(rename = "layout.swap")]
    LayoutSwap(LayoutSwap),
    /// Moves a tile one step: into its neighbour's slot, or to the tab's edge.
    #[serde(rename = "layout.move")]
    LayoutMove(LayoutMove),
    /// Moves a tile's edge in `direction` by `amount` (negative shrinks).
    #[serde(rename = "layout.resize")]
    LayoutResize(LayoutResize),
    /// Zooms a tile to fill its tab; leave `zoomed` out to toggle.
    #[serde(rename = "layout.zoom")]
    LayoutZoom(LayoutZoom),

    // ----- tall layout: master left, even stack right (F3) -----
    /// Opens a tile at the end of the tab's stack, whichever tile has focus; a
    /// lone tile becomes master | stack. Evens out the stack afterwards.
    #[serde(rename = "layout.split_auto")]
    LayoutSplitAuto(LayoutSplitAuto),
    /// Swaps a tile with its tab's master; focus stays on the tile.
    #[serde(rename = "layout.promote")]
    LayoutPromote(TileTarget),
    /// Toggles master on the left ↔ master on top, keeping the stack's order.
    #[serde(rename = "layout.rotate")]
    LayoutRotate(TabTarget),
    /// Gives every tile in the tab's stack the same size.
    #[serde(rename = "layout.equalize")]
    LayoutEqualize(TabTarget),

    // ----- terminals (F12, F20) -----
    /// Records the herdr pane a terminal tile views. Sent by the herdr adapter.
    #[serde(rename = "terminal.bind")]
    TerminalBind(TerminalBind),
    /// Gives an ended terminal tile a fresh herdr pane, keeping the tile's id
    /// and place. The new pane gets its own herdr tab and the tile's cwd.
    #[serde(rename = "terminal.respawn")]
    TerminalRespawn(TileTarget),
    #[serde(rename = "terminal.send_text")]
    TerminalSendText(TerminalSendText),
    #[serde(rename = "terminal.send_keys")]
    TerminalSendKeys(TerminalSendKeys),
    /// Reads a terminal's screen or history; replies `text`. History lives in
    /// herdr, not in the tile (ADR 0005).
    #[serde(rename = "terminal.read")]
    TerminalRead(TerminalRead),
    /// Scrolls a terminal's view; replies `scroll`. 0 is the live bottom.
    #[serde(rename = "terminal.scroll")]
    TerminalScroll(TerminalScroll),
    /// Finds lines containing `query` in a terminal's history; replies
    /// `matches`. Runs over `recent_unwrapped` output in the core.
    #[serde(rename = "terminal.search")]
    TerminalSearch(TerminalSearch),

    // ----- event streams (F22) -----
    /// Streams events on this connection as `events.event` notifications;
    /// replies `subscribed`. Answered by the API server, not the engine.
    #[serde(rename = "events.subscribe")]
    EventsSubscribe(crate::stream::EventsSubscribe),
    /// Ends a subscription made on this connection.
    #[serde(rename = "events.unsubscribe")]
    EventsUnsubscribe(crate::stream::EventsUnsubscribe),

    // ----- project layout files, palette (F5, F6) -----
    /// Opens a project layout file's tabs and tiles; replies `created` with
    /// the workspace and first tab.
    #[serde(rename = "layout.apply")]
    LayoutApply(crate::project::LayoutApply),
    /// A tab or workspace as layout-file TOML; replies `layout_file`.
    #[serde(rename = "layout.export")]
    LayoutExport(crate::project::LayoutExport),
    /// Every action the palette offers, with its current key chords;
    /// replies `commands`.
    #[serde(rename = "commands.list")]
    CommandsList,

    // ----- machines (F17, ADR 0011) -----
    /// Local plus herdr's saved machines and their connection state;
    /// replies `machines`.
    #[serde(rename = "machines.list")]
    MachinesList,
}

/// Every method name, in declaration order.
pub const METHODS: &[&str] = &[
    "core.ping",
    "session.get",
    "window.create",
    "window.close",
    "window.focus",
    "window.set_frame",
    "window.set_sidebar",
    "window.list",
    "workspace.list",
    "workspace.create",
    "workspace.close",
    "workspace.focus",
    "workspace.rename",
    "workspace.move",
    "workspace.bind",
    "tab.list",
    "tab.create",
    "tab.close",
    "tab.focus",
    "tab.rename",
    "tab.move",
    "tile.list",
    "tile.get",
    "tile.close",
    "tile.focus",
    "tile.seen",
    "tile.rename",
    "browser.open",
    "browser.state",
    "preview.open",
    "preview.get",
    "files.list",
    "files.expand",
    "files.collapse",
    "files.set_root",
    "files.reveal",
    "files.toggle_hidden",
    "files.open",
    "layout.get",
    "layout.split",
    "layout.focus",
    "layout.swap",
    "layout.move",
    "layout.resize",
    "layout.zoom",
    "layout.split_auto",
    "layout.promote",
    "layout.rotate",
    "layout.equalize",
    "terminal.bind",
    "terminal.respawn",
    "terminal.send_text",
    "terminal.send_keys",
    "terminal.read",
    "terminal.scroll",
    "terminal.search",
    "events.subscribe",
    "events.unsubscribe",
    "layout.apply",
    "layout.export",
    "commands.list",
    "machines.list",
];

impl Command {
    /// Parses a command, telling an unknown method apart from bad params.
    pub fn from_json(text: &str) -> Result<Command, Error> {
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| Error::new(ErrorCode::ParseError, e.to_string()))?;
        let method = value
            .get("method")
            .and_then(|m| m.as_str())
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "missing `method`"))?;
        if !METHODS.contains(&method) {
            return Err(Error::new(
                ErrorCode::MethodNotFound,
                format!("unknown method `{method}`"),
            ));
        }
        let method = method.to_owned();
        // `params` may be omitted when every field is optional.
        let value = match value {
            serde_json::Value::Object(mut map) if !map.contains_key("params") => {
                if let Ok(command) = serde_json::from_value(serde_json::Value::Object(map.clone()))
                {
                    return Ok(command);
                }
                map.insert("params".into(), serde_json::json!({}));
                serde_json::Value::Object(map)
            }
            other => other,
        };
        serde_json::from_value(value).map_err(|e| {
            Error::new(
                ErrorCode::InvalidParams,
                format!("bad params for `{method}`: {e}"),
            )
        })
    }

    /// This command's method name.
    pub fn method(&self) -> &'static str {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(map)) => {
                let name = map.get("method").and_then(|m| m.as_str()).unwrap_or("");
                METHODS.iter().find(|m| **m == name).copied().unwrap_or("")
            }
            _ => "",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WindowCreate {
    /// Name of the window's first workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WindowTarget {
    pub window: WindowId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WindowSetFrame {
    pub window: WindowId,
    pub frame: Frame,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WindowSetSidebar {
    pub window: WindowId,
    pub visible: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceCreate {
    /// Defaults to the focused window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The herdr machine its terminals run on: `"local"` (the default), or a
    /// machine's ID or label from `machines.list` (ADR 0011).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceTarget {
    /// Defaults to the focused workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceRename {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceMove {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
    /// New position within its window; past the end means last.
    pub index: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceBind {
    pub workspace: WorkspaceId,
    pub herdr_workspace: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TabCreate {
    /// Defaults to the focused workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The tab's first tile; defaults to a terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<TileContent>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TabTarget {
    /// Defaults to the focused tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<TabId>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TabRename {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<TabId>,
    /// `null` or absent clears the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TabMove {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<TabId>,
    /// New position within its workspace; past the end means last.
    pub index: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TileList {
    /// Only tiles in this workspace; all tiles when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TileTarget {
    /// Defaults to the focused tile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TileClose {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// Kill the terminal's herdr pane instead of detaching from it.
    #[serde(default)]
    pub kill: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutSplit {
    /// The tile to split; defaults to the focused one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// Which side of `tile` the new tile goes.
    pub direction: Direction,
    /// The new tile; defaults to a terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<TileContent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutFocus {
    pub direction: Direction,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutSwap {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// Swap with the neighbour this way. Give exactly one of `direction`, `with`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    /// Swap with this tile, which must be in the same tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<TileId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutMove {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    pub direction: Direction,
}

/// Default step for `layout.resize`.
pub const DEFAULT_RESIZE_STEP: f64 = 0.05;

fn default_resize_step() -> f64 {
    DEFAULT_RESIZE_STEP
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutResize {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    pub direction: Direction,
    /// Share of the enclosing split to move the edge by. Defaults to 0.05.
    #[serde(default = "default_resize_step")]
    pub amount: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutZoom {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// `true` zooms, `false` unzooms, absent toggles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoomed: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutSplitAuto {
    /// Defaults to the focused tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<TabId>,
    /// The new tile; defaults to a terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<TileContent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalBind {
    pub tile: TileId,
    pub herdr: HerdrRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalSendText {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// Sent literally, as if pasted.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalSendKeys {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// Key names in herdr's notation, e.g. `enter`, `ctrl+c`, `up`.
    pub keys: Vec<String>,
}

/// What `terminal.read` returns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReadSource {
    /// The visible screen.
    #[default]
    Visible,
    /// Recent output, soft-wrapped as on screen.
    Recent,
    /// Recent output with soft wraps joined, for log-like text.
    RecentUnwrapped,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalRead {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    #[serde(default)]
    pub source: ReadSource,
    /// Upper bound on lines returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceList {
    /// Only this window's workspaces; all when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowId>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TileRename {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// `null` or absent clears the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserOpen {
    /// A browser tile; defaults to the focused tile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserState {
    /// A browser tile; defaults to the focused tile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// The page's current URL; absent leaves it unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The page's `<title>`; absent or empty clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PreviewOpen {
    /// A preview tile; defaults to the focused tile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalScroll {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// Lines above the live bottom; 0 returns to it.
    pub offset_from_bottom: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalSearch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
    /// Literal text to find.
    pub query: String,
    #[serde(default)]
    pub case_sensitive: bool,
    /// How many lines of history to search; herdr's default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TabList {
    /// Only this workspace's tabs; all when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
}
