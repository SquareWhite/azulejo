//! The whole object model: windows → workspaces → tabs → layout tree → tiles.
//!
//! Every operation either succeeds or returns an error with the session
//! untouched. `check_invariants` states the rules; the proptests hold every
//! operation to them.

use std::collections::HashSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ModelError;
use crate::dynamic::{self, InsertAt, LayoutKind, LayoutParams, Placement, TabLayout};
use crate::ids::{TabId, TileId, WindowId, WorkspaceId};
use crate::layout::{Direction, Node, Rect};
use crate::tile::{HerdrRef, Tile};

/// How many recently focused tiles a tab remembers for tie-breaking.
const RECENT_MAX: usize = 32;

/// Session-wide layout preferences from config. Not saved: the core sets
/// them at startup and on config reload.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutPrefs {
    /// What new tabs start as.
    pub default: LayoutKind,
    /// Where new tiles go in dynamic tabs.
    pub insert: InsertAt,
    /// What `layout.cycle` walks through.
    pub cycle: Vec<LayoutKind>,
    /// Parameters new dynamic tabs start with.
    pub params: LayoutParams,
}

impl Default for LayoutPrefs {
    fn default() -> Self {
        Self {
            default: LayoutKind::Manual,
            insert: InsertAt::End,
            cycle: LayoutKind::DYNAMIC.to_vec(),
            params: LayoutParams::default(),
        }
    }
}

/// A tab: one layout tree. A tab always holds at least one tile; closing its
/// last tile closes the tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tab {
    pub id: TabId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub root: Node,
    /// The tiles in `root`, in no particular order.
    pub tiles: Vec<Tile>,
    pub focused: TileId,
    /// A zoomed tile fills the tab until unzoomed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoomed: Option<TileId>,
    /// How the tiles are arranged (ADR 0010). Old state files have none and
    /// load as manual. For dynamic kinds `root` is derived from it.
    #[serde(default, skip_serializing_if = "TabLayout::is_plain_manual")]
    pub layout: TabLayout,
    /// Recently focused tiles, most recent first: directional focus breaks
    /// ties with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<TileId>,
}

impl Tab {
    fn new(tile: Tile, name: Option<String>, prefs: &LayoutPrefs) -> Self {
        let mut layout = TabLayout {
            kind: prefs.default,
            params: prefs.params,
            ..TabLayout::default()
        };
        layout.params.clamp();
        if layout.kind.is_dynamic() {
            layout.order = vec![tile.id];
        }
        Self {
            id: TabId::new(),
            name,
            root: Node::leaf(tile.id),
            focused: tile.id,
            recent: vec![tile.id],
            tiles: vec![tile],
            zoomed: None,
            layout,
        }
    }

    /// A manual tab built from parts, e.g. one adopted from herdr.
    pub fn manual(
        id: TabId,
        name: Option<String>,
        root: Node,
        tiles: Vec<Tile>,
        focused: TileId,
    ) -> Self {
        Self {
            id,
            name,
            root,
            tiles,
            focused,
            zoomed: None,
            layout: TabLayout::default(),
            recent: vec![focused],
        }
    }

    pub fn tile(&self, id: TileId) -> Option<&Tile> {
        self.tiles.iter().find(|t| t.id == id)
    }

    pub fn kind(&self) -> LayoutKind {
        self.layout.kind
    }

    pub fn is_dynamic(&self) -> bool {
        self.layout.kind.is_dynamic()
    }

    /// Where every tile goes, in the unit square. Manual tabs place each leaf
    /// of the tree; dynamic ones run their layout rule.
    pub fn placements(&self) -> Vec<Placement> {
        if self.is_dynamic() {
            self.layout.arrange(self.focused)
        } else {
            self.root
                .rects()
                .into_iter()
                .map(|(tile, rect)| Placement {
                    tile,
                    rect,
                    z: 0,
                    visible: true,
                })
                .collect()
        }
    }

    /// Recomputes the derived tree of a dynamic tab.
    pub fn sync(&mut self) {
        if self.is_dynamic() {
            self.root = self.layout.frozen_tree();
        }
    }

    /// Focuses `id` and remembers it as the most recent.
    pub fn set_focused(&mut self, id: TileId) {
        self.focused = id;
        self.recent.retain(|t| *t != id);
        self.recent.insert(0, id);
        self.recent.truncate(RECENT_MAX);
    }

    /// The tile `direction` of `from`: geometric over the placements, ties to
    /// the most recently focused. Monocle walks the order instead (right and
    /// down go forward), as Bismuth does.
    pub fn neighbour(&self, from: TileId, direction: Direction) -> Option<TileId> {
        match self.layout.kind {
            LayoutKind::Manual => self.root.neighbour_recent(from, direction, &self.recent),
            LayoutKind::Monocle => {
                let order = &self.layout.order;
                let n = order.len();
                if n < 2 {
                    return None;
                }
                let i = order.iter().position(|t| *t == from)?;
                let j = if direction.is_forward() {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                };
                Some(order[j])
            }
            _ => {
                let rects: Vec<(TileId, Rect)> = self
                    .placements()
                    .into_iter()
                    .map(|p| (p.tile, p.rect))
                    .collect();
                dynamic::neighbour_among(&rects, from, direction, &self.recent)
            }
        }
    }

    /// Puts a new tile into a dynamic tab's order. `anchor` is the tile an
    /// `after_focused` insert goes after.
    fn insert_dynamic(&mut self, tile: TileId, anchor: TileId, at: InsertAt) {
        let order = &mut self.layout.order;
        match at {
            InsertAt::End => order.push(tile),
            InsertAt::Master => order.insert(0, tile),
            InsertAt::AfterFocused => {
                let i = order
                    .iter()
                    .position(|t| *t == anchor)
                    .map_or(order.len(), |i| i + 1);
                order.insert(i, tile);
            }
        }
    }

    /// Swaps two tiles' places: in the order for dynamic tabs (weights travel
    /// with the tiles), in the tree for manual ones.
    fn swap(&mut self, a: TileId, b: TileId) -> Result<(), ModelError> {
        if self.is_dynamic() {
            let order = &mut self.layout.order;
            let i = order
                .iter()
                .position(|t| *t == a)
                .ok_or(ModelError::TileNotFound(a))?;
            let j = order
                .iter()
                .position(|t| *t == b)
                .ok_or(ModelError::TileNotFound(b))?;
            order.swap(i, j);
            self.sync();
            Ok(())
        } else {
            self.root.swap(a, b)
        }
    }
}

/// A workspace; maps one-to-one to a herdr workspace. It may have no tabs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The herdr workspace backing this one, once the adapter has created it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub herdr_workspace: Option<String>,
    /// The herdr machine its terminals run on: `None` for Local, else
    /// herdr's saved-machine ID (ADR 0011).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// The folder whose git branch the sidebar shows, when the user set one
    /// (`workspace.set_repo`). Without it the core follows the focused
    /// terminal's working directory (F26).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The repository the workspace is in and its branch, as the core last
    /// read it; absent outside a repository (F26).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitInfo>,
    pub tabs: Vec<Tab>,
    /// `None` exactly when there are no tabs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tab: Option<TabId>,
}

/// A workspace's repository and branch (F26).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitInfo {
    /// The work tree's top folder.
    pub repo: String,
    /// The checked-out branch, or the short commit when `detached`. Absent in
    /// a repository with no commits yet only if git can't name the branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// HEAD points at a commit, not a branch.
    #[serde(default, skip_serializing_if = "crate::tile::is_false")]
    pub detached: bool,
    /// Tracked files have changes (untracked files don't count).
    #[serde(default, skip_serializing_if = "crate::tile::is_false")]
    pub dirty: bool,
}

/// An app window. It always holds at least one workspace; closing its last
/// workspace closes the window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Window {
    pub id: WindowId,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: WorkspaceId,
    /// Last known screen frame, in points; restored on relaunch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Frame>,
    /// The workspace sidebar is collapsed, to a rail or out of sight
    /// (`sidebar` says which); restored on relaunch. The name predates the
    /// rail.
    #[serde(default, skip_serializing_if = "crate::tile::is_false")]
    pub sidebar_hidden: bool,
    /// How the sidebar shows: `full`, or collapsed as the `[sidebar]
    /// collapsed` setting says (`rail` by default, or `hidden`).
    #[serde(default, skip_serializing_if = "SidebarState::is_full")]
    pub sidebar: SidebarState,
}

/// How a window's sidebar shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SidebarState {
    /// Machine groups, workspace rows and the footer.
    #[default]
    Full,
    /// Collapsed to a narrow rail: workspace numbers and agent states.
    Rail,
    /// Collapsed out of sight.
    Hidden,
}

impl SidebarState {
    pub fn is_full(&self) -> bool {
        *self == SidebarState::Full
    }
}

/// A window's frame on screen, in points, origin top left of the main display.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Everything the app shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Session {
    pub windows: Vec<Window>,
    /// `None` exactly when there are no windows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused_window: Option<WindowId>,
    /// herdr panes whose tiles were closed here. Adoption skips them, so a
    /// closed (detached) tile doesn't come back on the next relaunch. Pruned
    /// once herdr no longer has the pane.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detached_herdr: Vec<HerdrRef>,
    /// Layout preferences from config; not saved.
    #[serde(skip)]
    #[schemars(skip)]
    pub prefs: LayoutPrefs,
}

/// Where the user's focus is. Inner fields are `None` when that level is empty.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Focus {
    pub window: Option<WindowId>,
    pub workspace: Option<WorkspaceId>,
    pub tab: Option<TabId>,
    pub tile: Option<TileId>,
}

/// Indices of a tile's window, workspace and tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TilePath {
    pub window: usize,
    pub workspace: usize,
    pub tab: usize,
}

/// What `close_tile` removed.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedTile {
    pub tile: Tile,
    pub window: WindowId,
    pub workspace: WorkspaceId,
    pub tab: TabId,
    /// Set when the tile was the tab's last one, so the tab closed too.
    pub tab_closed: bool,
}

/// What `close_workspace` removed.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedWorkspace {
    pub workspace: Workspace,
    pub window: WindowId,
    /// Set when it was the window's last workspace, so the window closed too.
    pub window_closed: bool,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    // ----- lookups -----

    pub fn focus(&self) -> Focus {
        let mut focus = Focus::default();
        let Some(window) = self.focused_window.and_then(|id| self.window(id).ok()) else {
            return focus;
        };
        focus.window = Some(window.id);
        let Some(ws) = window
            .workspaces
            .iter()
            .find(|w| w.id == window.active_workspace)
        else {
            return focus;
        };
        focus.workspace = Some(ws.id);
        let Some(tab) = ws
            .active_tab
            .and_then(|id| ws.tabs.iter().find(|t| t.id == id))
        else {
            return focus;
        };
        focus.tab = Some(tab.id);
        focus.tile = Some(tab.focused);
        focus
    }

    pub fn focused_tile(&self) -> Option<TileId> {
        self.focus().tile
    }

    pub fn window(&self, id: WindowId) -> Result<&Window, ModelError> {
        self.windows
            .iter()
            .find(|w| w.id == id)
            .ok_or(ModelError::WindowNotFound(id))
    }

    fn window_index(&self, id: WindowId) -> Result<usize, ModelError> {
        self.windows
            .iter()
            .position(|w| w.id == id)
            .ok_or(ModelError::WindowNotFound(id))
    }

    fn workspace_index(&self, id: WorkspaceId) -> Result<(usize, usize), ModelError> {
        for (wi, window) in self.windows.iter().enumerate() {
            if let Some(si) = window.workspaces.iter().position(|w| w.id == id) {
                return Ok((wi, si));
            }
        }
        Err(ModelError::WorkspaceNotFound(id))
    }

    pub fn workspace(&self, id: WorkspaceId) -> Result<&Workspace, ModelError> {
        let (wi, si) = self.workspace_index(id)?;
        Ok(&self.windows[wi].workspaces[si])
    }

    fn workspace_mut(&mut self, id: WorkspaceId) -> Result<&mut Workspace, ModelError> {
        let (wi, si) = self.workspace_index(id)?;
        Ok(&mut self.windows[wi].workspaces[si])
    }

    fn tab_index(&self, id: TabId) -> Result<TilePath, ModelError> {
        for (wi, window) in self.windows.iter().enumerate() {
            for (si, ws) in window.workspaces.iter().enumerate() {
                if let Some(ti) = ws.tabs.iter().position(|t| t.id == id) {
                    return Ok(TilePath {
                        window: wi,
                        workspace: si,
                        tab: ti,
                    });
                }
            }
        }
        Err(ModelError::TabNotFound(id))
    }

    pub fn tab(&self, id: TabId) -> Result<&Tab, ModelError> {
        let p = self.tab_index(id)?;
        Ok(self.tab_at(p))
    }

    fn tab_at(&self, p: TilePath) -> &Tab {
        &self.windows[p.window].workspaces[p.workspace].tabs[p.tab]
    }

    fn tab_at_mut(&mut self, p: TilePath) -> &mut Tab {
        &mut self.windows[p.window].workspaces[p.workspace].tabs[p.tab]
    }

    /// Finds the window, workspace and tab holding `tile`.
    pub fn locate(&self, tile: TileId) -> Result<TilePath, ModelError> {
        for (wi, window) in self.windows.iter().enumerate() {
            for (si, ws) in window.workspaces.iter().enumerate() {
                for (ti, tab) in ws.tabs.iter().enumerate() {
                    if tab.tile(tile).is_some() {
                        return Ok(TilePath {
                            window: wi,
                            workspace: si,
                            tab: ti,
                        });
                    }
                }
            }
        }
        Err(ModelError::TileNotFound(tile))
    }

    /// IDs of the window, workspace and tab at `path`.
    pub fn ids_at(&self, p: TilePath) -> (WindowId, WorkspaceId, TabId) {
        let window = &self.windows[p.window];
        let ws = &window.workspaces[p.workspace];
        (window.id, ws.id, ws.tabs[p.tab].id)
    }

    pub fn tile(&self, id: TileId) -> Result<&Tile, ModelError> {
        let p = self.locate(id)?;
        Ok(self.tab_at(p).tile(id).expect("located"))
    }

    pub fn tile_mut(&mut self, id: TileId) -> Result<&mut Tile, ModelError> {
        let p = self.locate(id)?;
        Ok(self
            .tab_at_mut(p)
            .tiles
            .iter_mut()
            .find(|t| t.id == id)
            .expect("located"))
    }

    /// Every tile with where it lives.
    pub fn tiles(&self) -> impl Iterator<Item = (TilePath, &Tile)> {
        self.windows.iter().enumerate().flat_map(|(wi, window)| {
            window
                .workspaces
                .iter()
                .enumerate()
                .flat_map(move |(si, ws)| {
                    ws.tabs.iter().enumerate().flat_map(move |(ti, tab)| {
                        tab.tiles.iter().map(move |t| {
                            (
                                TilePath {
                                    window: wi,
                                    workspace: si,
                                    tab: ti,
                                },
                                t,
                            )
                        })
                    })
                })
        })
    }

    /// The tile to act on: `tile` if given, else the focused one.
    pub fn target(&self, tile: Option<TileId>) -> Result<TileId, ModelError> {
        match tile {
            Some(id) => self.locate(id).map(|_| id),
            None => self.focused_tile().ok_or(ModelError::NothingFocused),
        }
    }

    /// Makes `p` the active window, workspace and tab.
    fn activate(&mut self, p: TilePath) {
        let window = &mut self.windows[p.window];
        let ws = &mut window.workspaces[p.workspace];
        ws.active_tab = Some(ws.tabs[p.tab].id);
        window.active_workspace = ws.id;
        self.focused_window = Some(window.id);
    }

    // ----- windows -----

    /// Opens a window with one empty workspace and focuses it.
    pub fn create_window(
        &mut self,
        workspace_name: impl Into<String>,
        cwd: Option<String>,
    ) -> (WindowId, WorkspaceId) {
        let ws = Workspace {
            id: WorkspaceId::new(),
            name: workspace_name.into(),
            cwd,
            herdr_workspace: None,
            machine: None,
            repo: None,
            git: None,
            tabs: Vec::new(),
            active_tab: None,
        };
        let window = Window {
            id: WindowId::new(),
            active_workspace: ws.id,
            workspaces: vec![ws],
            frame: None,
            sidebar_hidden: false,
            sidebar: SidebarState::Full,
        };
        let ids = (window.id, window.active_workspace);
        self.windows.push(window);
        self.focused_window = Some(ids.0);
        ids
    }

    pub fn close_window(&mut self, id: WindowId) -> Result<Window, ModelError> {
        let index = self.window_index(id)?;
        let window = self.windows.remove(index);
        if self.focused_window == Some(id) {
            self.focused_window = self.windows.last().map(|w| w.id);
        }
        Ok(window)
    }

    pub fn focus_window(&mut self, id: WindowId) -> Result<(), ModelError> {
        self.window_index(id)?;
        self.focused_window = Some(id);
        Ok(())
    }

    /// Opens a window's sidebar, or collapses it to `collapsed` (a rail,
    /// or hidden; `Full` counts as a rail).
    pub fn set_window_sidebar(
        &mut self,
        id: WindowId,
        visible: bool,
        collapsed: SidebarState,
    ) -> Result<(), ModelError> {
        let index = self.window_index(id)?;
        let window = &mut self.windows[index];
        window.sidebar_hidden = !visible;
        window.sidebar = Self::sidebar_form(visible, collapsed);
        Ok(())
    }

    /// Every collapsed sidebar takes the `collapsed` form (the setting
    /// changed, or a saved session came back); open ones stay open.
    pub fn set_sidebar_collapsed(&mut self, collapsed: SidebarState) {
        for window in &mut self.windows {
            window.sidebar = Self::sidebar_form(!window.sidebar_hidden, collapsed);
        }
    }

    fn sidebar_form(visible: bool, collapsed: SidebarState) -> SidebarState {
        match (visible, collapsed) {
            (true, _) => SidebarState::Full,
            (false, SidebarState::Hidden) => SidebarState::Hidden,
            (false, _) => SidebarState::Rail,
        }
    }

    pub fn set_window_frame(&mut self, id: WindowId, frame: Frame) -> Result<(), ModelError> {
        let valid = [frame.x, frame.y, frame.width, frame.height]
            .iter()
            .all(|v| v.is_finite())
            && frame.width > 0.0
            && frame.height > 0.0;
        if !valid {
            return Err(ModelError::InvalidValue(format!(
                "invalid window frame {frame:?}"
            )));
        }
        let index = self.window_index(id)?;
        self.windows[index].frame = Some(frame);
        Ok(())
    }

    // ----- workspaces -----

    /// Adds an empty workspace to `window` and makes it active.
    pub fn create_workspace(
        &mut self,
        window: WindowId,
        name: impl Into<String>,
        cwd: Option<String>,
    ) -> Result<WorkspaceId, ModelError> {
        let index = self.window_index(window)?;
        let ws = Workspace {
            id: WorkspaceId::new(),
            name: name.into(),
            cwd,
            herdr_workspace: None,
            machine: None,
            repo: None,
            git: None,
            tabs: Vec::new(),
            active_tab: None,
        };
        let id = ws.id;
        let window = &mut self.windows[index];
        window.workspaces.push(ws);
        window.active_workspace = id;
        self.focused_window = Some(window.id);
        Ok(id)
    }

    /// Closes a workspace and its tabs; closes the window too if it was the last.
    pub fn close_workspace(&mut self, id: WorkspaceId) -> Result<ClosedWorkspace, ModelError> {
        let (wi, si) = self.workspace_index(id)?;
        let window_id = self.windows[wi].id;
        if self.windows[wi].workspaces.len() == 1 {
            let mut window = self.close_window(window_id)?;
            return Ok(ClosedWorkspace {
                workspace: window.workspaces.remove(0),
                window: window_id,
                window_closed: true,
            });
        }
        let window = &mut self.windows[wi];
        let workspace = window.workspaces.remove(si);
        if window.active_workspace == id {
            let next = si.min(window.workspaces.len() - 1);
            window.active_workspace = window.workspaces[next].id;
        }
        Ok(ClosedWorkspace {
            workspace,
            window: window_id,
            window_closed: false,
        })
    }

    pub fn focus_workspace(&mut self, id: WorkspaceId) -> Result<(), ModelError> {
        let (wi, _) = self.workspace_index(id)?;
        let window = &mut self.windows[wi];
        window.active_workspace = id;
        self.focused_window = Some(window.id);
        Ok(())
    }

    /// Moves a workspace to `index` within its window (clamped to the end).
    pub fn move_workspace(&mut self, id: WorkspaceId, index: usize) -> Result<(), ModelError> {
        let (wi, si) = self.workspace_index(id)?;
        let list = &mut self.windows[wi].workspaces;
        let ws = list.remove(si);
        let index = index.min(list.len());
        list.insert(index, ws);
        Ok(())
    }

    pub fn rename_workspace(
        &mut self,
        id: WorkspaceId,
        name: impl Into<String>,
    ) -> Result<(), ModelError> {
        self.workspace_mut(id)?.name = name.into();
        Ok(())
    }

    /// Puts `id`'s terminals on a herdr machine (`None` = Local).
    pub fn set_workspace_machine(
        &mut self,
        id: WorkspaceId,
        machine: Option<String>,
    ) -> Result<(), ModelError> {
        self.workspace_mut(id)?.machine = machine;
        Ok(())
    }

    /// Sets or clears the folder whose branch `id` shows (F26).
    pub fn set_workspace_repo(
        &mut self,
        id: WorkspaceId,
        repo: Option<String>,
    ) -> Result<(), ModelError> {
        self.workspace_mut(id)?.repo = repo.filter(|r| !r.is_empty());
        Ok(())
    }

    /// Records what the core last read about `id`'s repository (F26).
    pub fn set_workspace_git(
        &mut self,
        id: WorkspaceId,
        git: Option<GitInfo>,
    ) -> Result<(), ModelError> {
        self.workspace_mut(id)?.git = git;
        Ok(())
    }

    /// Records the herdr workspace backing `id`.
    pub fn bind_workspace(&mut self, id: WorkspaceId, herdr: String) -> Result<(), ModelError> {
        self.workspace_mut(id)?.herdr_workspace = Some(herdr);
        Ok(())
    }

    // ----- tabs -----

    /// Opens a tab holding `tile` and focuses it.
    pub fn create_tab(
        &mut self,
        workspace: WorkspaceId,
        tile: Tile,
        name: Option<String>,
    ) -> Result<TabId, ModelError> {
        let (wi, si) = self.workspace_index(workspace)?;
        if self.locate(tile.id).is_ok() {
            return Err(ModelError::InvalidValue(format!(
                "tile {} already exists",
                tile.id
            )));
        }
        let tab = Tab::new(tile, name, &self.prefs);
        let id = tab.id;
        let ws = &mut self.windows[wi].workspaces[si];
        ws.tabs.push(tab);
        let p = TilePath {
            window: wi,
            workspace: si,
            tab: ws.tabs.len() - 1,
        };
        self.activate(p);
        Ok(id)
    }

    /// Adds a ready-made tab (e.g. one adopted from herdr) to `workspace`
    /// without moving focus, unless the workspace had no tabs. The tab must
    /// be well formed and its IDs new.
    pub fn insert_tab(&mut self, workspace: WorkspaceId, tab: Tab) -> Result<TabId, ModelError> {
        let (wi, si) = self.workspace_index(workspace)?;
        let mut seen: HashSet<TileId> = self.tiles().map(|(_, t)| t.id).collect();
        let fresh_tab = self.tab_index(tab.id).is_err();
        if !fresh_tab {
            return Err(ModelError::InvalidValue(format!(
                "tab {} already exists",
                tab.id
            )));
        }
        check_tab(&tab, &mut seen).map_err(ModelError::InvalidValue)?;
        let id = tab.id;
        let ws = &mut self.windows[wi].workspaces[si];
        ws.tabs.push(tab);
        if ws.active_tab.is_none() {
            ws.active_tab = Some(id);
        }
        Ok(id)
    }

    pub fn close_tab(&mut self, id: TabId) -> Result<Tab, ModelError> {
        let p = self.tab_index(id)?;
        let ws = &mut self.windows[p.window].workspaces[p.workspace];
        let tab = ws.tabs.remove(p.tab);
        if ws.active_tab == Some(id) {
            ws.active_tab = if ws.tabs.is_empty() {
                None
            } else {
                Some(ws.tabs[p.tab.min(ws.tabs.len() - 1)].id)
            };
        }
        Ok(tab)
    }

    pub fn focus_tab(&mut self, id: TabId) -> Result<(), ModelError> {
        let p = self.tab_index(id)?;
        self.activate(p);
        Ok(())
    }

    /// Moves a tab to `index` within its workspace (clamped to the end).
    pub fn move_tab(&mut self, id: TabId, index: usize) -> Result<(), ModelError> {
        let p = self.tab_index(id)?;
        let list = &mut self.windows[p.window].workspaces[p.workspace].tabs;
        let tab = list.remove(p.tab);
        let index = index.min(list.len());
        list.insert(index, tab);
        Ok(())
    }

    pub fn rename_tab(&mut self, id: TabId, name: Option<String>) -> Result<(), ModelError> {
        let p = self.tab_index(id)?;
        self.tab_at_mut(p).name = name;
        Ok(())
    }

    // ----- tiles and layout -----

    /// Splits `target`, placing `tile` on the `direction` side, and focuses it.
    pub fn split(
        &mut self,
        target: TileId,
        direction: Direction,
        tile: Tile,
    ) -> Result<(), ModelError> {
        let p = self.locate(target)?;
        if self.locate(tile.id).is_ok() {
            return Err(ModelError::InvalidValue(format!(
                "tile {} already exists",
                tile.id
            )));
        }
        let insert = self.prefs.insert;
        let tab = self.tab_at_mut(p);
        let id = tile.id;
        if tab.is_dynamic() {
            tab.insert_dynamic(id, target, insert);
        } else {
            tab.root.split(target, direction, id)?;
        }
        tab.tiles.push(tile);
        tab.sync();
        tab.set_focused(id);
        tab.zoomed = None;
        self.activate(p);
        Ok(())
    }

    /// Adds `tile` beside `target` on the `direction` side without moving
    /// focus or zoom: for tiles that appear on their own, like a pane split
    /// off in herdr (F15). `ratio` is the new split's first child's share.
    pub fn insert_tile_beside(
        &mut self,
        target: TileId,
        direction: Direction,
        tile: Tile,
        ratio: f64,
    ) -> Result<(), ModelError> {
        let p = self.locate(target)?;
        if self.locate(tile.id).is_ok() {
            return Err(ModelError::InvalidValue(format!(
                "tile {} already exists",
                tile.id
            )));
        }
        let insert = self.prefs.insert;
        let tab = self.tab_at_mut(p);
        if tab.is_dynamic() {
            tab.insert_dynamic(tile.id, target, insert);
        } else {
            tab.root
                .split_with_ratio(target, direction, tile.id, ratio)?;
        }
        tab.tiles.push(tile);
        tab.sync();
        Ok(())
    }

    /// Adds `tile` to the end of the tab's stack (as [`Session::split_auto`])
    /// without moving focus or zoom.
    pub fn insert_tile_auto(&mut self, tab: TabId, tile: Tile) -> Result<(), ModelError> {
        let p = self.tab_index(tab)?;
        if self.locate(tile.id).is_ok() {
            return Err(ModelError::InvalidValue(format!(
                "tile {} already exists",
                tile.id
            )));
        }
        let insert = self.prefs.insert;
        let tab = self.tab_at_mut(p);
        if tab.is_dynamic() {
            let anchor = tab.focused;
            tab.insert_dynamic(tile.id, anchor, insert);
        } else {
            tab.root.split_auto(tile.id)?;
        }
        tab.tiles.push(tile);
        tab.sync();
        Ok(())
    }

    /// Removes a tile. Focus moves to its sibling; an emptied tab closes.
    pub fn close_tile(&mut self, id: TileId) -> Result<ClosedTile, ModelError> {
        let p = self.locate(id)?;
        let (window, workspace, tab_id) = self.ids_at(p);
        if self.tab_at(p).tiles.len() == 1 {
            let mut tab = self.close_tab(tab_id)?;
            return Ok(ClosedTile {
                tile: tab.tiles.remove(0),
                window,
                workspace,
                tab: tab_id,
                tab_closed: true,
            });
        }
        let tab = self.tab_at_mut(p);
        let index = tab.tiles.iter().position(|t| t.id == id).expect("located");
        let next = if tab.is_dynamic() {
            // The tile now in that position takes focus, else the one before.
            let order = &mut tab.layout.order;
            let i = order.iter().position(|t| *t == id).expect("ordered");
            order.remove(i);
            tab.layout.weights.remove(&id);
            let order = &tab.layout.order;
            order
                .get(i)
                .or_else(|| i.checked_sub(1).and_then(|j| order.get(j)))
                .copied()
                .expect("tiles remain")
        } else {
            tab.root.remove(id).expect("tile is in a multi-tile tree")
        };
        tab.recent.retain(|t| *t != id);
        if tab.focused == id {
            tab.set_focused(next);
        }
        if tab.zoomed == Some(id) {
            tab.zoomed = None;
        }
        tab.sync();
        Ok(ClosedTile {
            tile: tab.tiles.remove(index),
            window,
            workspace,
            tab: tab_id,
            tab_closed: false,
        })
    }

    /// Focuses a tile, activating its tab, workspace and window.
    pub fn focus_tile(&mut self, id: TileId) -> Result<(), ModelError> {
        let p = self.locate(id)?;
        let tab = self.tab_at_mut(p);
        if tab.zoomed.is_some_and(|z| z != id) {
            tab.zoomed = None;
        }
        tab.set_focused(id);
        self.activate(p);
        Ok(())
    }

    /// Moves focus to the neighbour in `direction` and returns it.
    pub fn focus_direction(
        &mut self,
        from: TileId,
        direction: Direction,
    ) -> Result<TileId, ModelError> {
        let p = self.locate(from)?;
        let next = self
            .tab_at(p)
            .neighbour(from, direction)
            .ok_or(ModelError::NoNeighbour {
                tile: from,
                direction,
            })?;
        self.focus_tile(next)?;
        Ok(next)
    }

    /// Swaps `tile` with its neighbour in `direction`; returns the neighbour.
    pub fn swap_direction(
        &mut self,
        tile: TileId,
        direction: Direction,
    ) -> Result<TileId, ModelError> {
        let p = self.locate(tile)?;
        let other = self
            .tab_at(p)
            .neighbour(tile, direction)
            .ok_or(ModelError::NoNeighbour { tile, direction })?;
        self.swap_tiles(tile, other)?;
        Ok(other)
    }

    /// Swaps two tiles in the same tab.
    pub fn swap_tiles(&mut self, a: TileId, b: TileId) -> Result<(), ModelError> {
        let pa = self.locate(a)?;
        let pb = self.locate(b)?;
        if pa != pb {
            return Err(ModelError::DifferentTabs { a, b });
        }
        let tab = self.tab_at_mut(pa);
        tab.swap(a, b)?;
        tab.zoomed = None;
        Ok(())
    }

    /// Moves a tile one step in `direction`; see [`Node::move_tile`].
    pub fn move_tile(&mut self, tile: TileId, direction: Direction) -> Result<bool, ModelError> {
        let p = self.locate(tile)?;
        let tab = self.tab_at_mut(p);
        let moved = if tab.is_dynamic() {
            // As in Bismuth: a move is a swap with the neighbour that way.
            match tab.neighbour(tile, direction) {
                Some(other) => {
                    tab.swap(tile, other)?;
                    true
                }
                None => false,
            }
        } else {
            tab.root.move_tile(tile, direction)?
        };
        if moved {
            tab.zoomed = None;
        }
        Ok(moved)
    }

    /// Moves a tile's edge; see [`Node::resize`].
    pub fn resize(
        &mut self,
        tile: TileId,
        direction: Direction,
        amount: f64,
    ) -> Result<(), ModelError> {
        let p = self.locate(tile)?;
        let tab = self.tab_at_mut(p);
        match tab.layout.kind {
            LayoutKind::Manual => tab.root.resize(tile, direction, amount),
            // Every tile fills the tab: nothing to resize.
            LayoutKind::Monocle => Ok(()),
            _ => {
                if !amount.is_finite() {
                    return Err(ModelError::InvalidValue(format!(
                        "resize amount {amount} is not finite"
                    )));
                }
                // The edge that way, else the opposite one moved the same way.
                let moved = dynamic::adjust(&mut tab.layout, tile, direction, amount)
                    || dynamic::adjust(&mut tab.layout, tile, direction.opposite(), -amount);
                if !moved {
                    return Err(ModelError::NoEdge { tile, direction });
                }
                tab.sync();
                Ok(())
            }
        }
    }

    /// Zooms or unzooms a tile (`None` toggles) and focuses it. Returns whether
    /// it ends up zoomed.
    pub fn zoom(&mut self, tile: TileId, zoomed: Option<bool>) -> Result<bool, ModelError> {
        let p = self.locate(tile)?;
        let tab = self.tab_at_mut(p);
        let on = zoomed.unwrap_or(tab.zoomed != Some(tile));
        tab.zoomed = on.then_some(tile);
        tab.set_focused(tile);
        self.activate(p);
        Ok(on)
    }

    // ----- tall (master-stack) layout; see `Node::tall` -----

    /// Adds `tile` to the end of the tab's stack and focuses it.
    pub fn split_auto(&mut self, tab: TabId, tile: Tile) -> Result<(), ModelError> {
        let p = self.tab_index(tab)?;
        if self.locate(tile.id).is_ok() {
            return Err(ModelError::InvalidValue(format!(
                "tile {} already exists",
                tile.id
            )));
        }
        let insert = self.prefs.insert;
        let tab = self.tab_at_mut(p);
        let id = tile.id;
        if tab.is_dynamic() {
            let anchor = tab.focused;
            tab.insert_dynamic(id, anchor, insert);
        } else {
            tab.root.split_auto(id)?;
        }
        tab.tiles.push(tile);
        tab.sync();
        tab.set_focused(id);
        tab.zoomed = None;
        self.activate(p);
        Ok(())
    }

    /// Swaps `tile` with its tab's master. Focus stays on `tile`. Returns false
    /// when it already is the master.
    pub fn promote(&mut self, tile: TileId) -> Result<bool, ModelError> {
        let p = self.locate(tile)?;
        let tab = self.tab_at_mut(p);
        let changed = if tab.is_dynamic() {
            // To the front of the order (Bismuth's push to master).
            let order = &mut tab.layout.order;
            let i = order.iter().position(|t| *t == tile).expect("ordered");
            if i > 0 {
                let t = order.remove(i);
                order.insert(0, t);
            }
            tab.sync();
            i > 0
        } else {
            tab.root.promote(tile)?
        };
        if changed {
            tab.zoomed = None;
        }
        Ok(changed)
    }

    /// Flips the tab on its side: master left ↔ master on top (and right ↔
    /// bottom in a Tile tab), `tall.py`'s rotate. Focus and zoom are kept.
    /// Other dynamic kinds have nothing to flip.
    pub fn flip(&mut self, tab: TabId) -> Result<bool, ModelError> {
        let p = self.tab_index(tab)?;
        let tab = self.tab_at_mut(p);
        Ok(match tab.layout.kind {
            LayoutKind::Manual => tab.root.rotate(),
            LayoutKind::Tile => {
                let o = &mut tab.layout.params.tile.orientation;
                *o = o.flipped();
                tab.sync();
                true
            }
            _ => false,
        })
    }

    /// A quarter turn clockwise, or back with `reverse`. A Tile tab turns as
    /// Bismuth's rotate: master left → top → right → bottom. A manual tab
    /// turns its whole split tree as a picture, whatever its shape (a grid,
    /// columns, a tall layout), and four turns give it back exactly. Monocle
    /// and Three Column have nothing to turn: a silent no-op (`Ok(false)`),
    /// as is a lone tile. Focus and zoom are kept.
    pub fn rotate_turn(&mut self, tab: TabId, reverse: bool) -> Result<bool, ModelError> {
        let p = self.tab_index(tab)?;
        let t = self.tab_at_mut(p);
        Ok(match t.layout.kind {
            LayoutKind::Manual if reverse => t.root.rotate_anticlockwise(),
            LayoutKind::Manual => t.root.rotate_clockwise(),
            LayoutKind::Tile => {
                let o = &mut t.layout.params.tile.orientation;
                *o = o.turned(reverse);
                t.sync();
                true
            }
            _ => false,
        })
    }

    /// Toggles a Tile tab's masters between stacked and side by side
    /// ("rotate part").
    pub fn rotate_part(&mut self, tab: TabId) -> Result<bool, ModelError> {
        let p = self.tab_index(tab)?;
        let t = self.tab_at_mut(p);
        if t.layout.kind != LayoutKind::Tile {
            return Ok(false);
        }
        let side = &mut t.layout.params.tile.rotate_part;
        *side = !*side;
        t.sync();
        Ok(true)
    }

    /// Evens out the tab's stack; in a dynamic tab, resets every weight.
    pub fn equalize(&mut self, tab: TabId) -> Result<bool, ModelError> {
        let p = self.tab_index(tab)?;
        let tab = self.tab_at_mut(p);
        if tab.is_dynamic() {
            let changed = !tab.layout.weights.is_empty();
            tab.layout.weights.clear();
            tab.sync();
            Ok(changed)
        } else {
            Ok(tab.root.equalize())
        }
    }

    // ----- dynamic layouts (ADR 0010) -----

    /// Switches a tab's layout kind. Manual → dynamic takes the tree's tall
    /// reading as the order (and, for a canonical tall tree going to Tile,
    /// its ratio and stack sizes, so nothing jumps) and remembers the tree;
    /// dynamic → manual restores that tree if it still holds the same tiles,
    /// else keeps the frozen arrangement. Returns false when already there.
    pub fn set_layout(&mut self, tab: TabId, kind: LayoutKind) -> Result<bool, ModelError> {
        let p = self.tab_index(tab)?;
        let t = self.tab_at_mut(p);
        let old = t.layout.kind;
        if old == kind {
            return Ok(false);
        }
        match (old.is_dynamic(), kind.is_dynamic()) {
            (false, true) => {
                let (order, fit) = dynamic::from_tree(&t.root);
                t.layout.weights.clear();
                if kind == LayoutKind::Tile
                    && let Some((params, weights)) = fit
                {
                    t.layout.params.tile = params;
                    t.layout.weights = weights;
                }
                t.layout.order = order;
                t.layout.manual = Some(t.root.clone());
            }
            (true, false) => {
                // `root` already holds the frozen arrangement.
                let tiles: HashSet<TileId> = t.tiles.iter().map(|x| x.id).collect();
                if let Some(saved) = t.layout.manual.take()
                    && saved.leaf_count() == tiles.len()
                    && saved.tiles().into_iter().collect::<HashSet<_>>() == tiles
                {
                    t.root = saved;
                }
                t.layout.order.clear();
                t.layout.weights.clear();
            }
            _ => {}
        }
        t.layout.previous = Some(old);
        t.layout.kind = kind;
        t.sync();
        Ok(true)
    }

    /// Takes an arrangement made elsewhere (the herdr TUI, a phone; ADR
    /// 0012): `root` must hold exactly the tab's tiles. A manual tab takes
    /// the tree. A Tile or Three Column tab absorbs it when its rule can
    /// produce it (a reorder, a ratio, stack sizes), and otherwise freezes to
    /// manual with it, remembering its kind for toggling back. Monocle takes
    /// only the order. Focus and zoom stay. Returns whether anything changed.
    pub fn apply_tree(&mut self, tab: TabId, root: Node) -> Result<bool, ModelError> {
        let p = self.tab_index(tab)?;
        let t = self.tab_at_mut(p);
        let mut want: Vec<TileId> = t.tiles.iter().map(|x| x.id).collect();
        let mut have = root.tiles();
        want.sort_unstable();
        have.sort_unstable();
        if want != have {
            return Err(ModelError::InvalidValue(
                "the tree doesn't hold exactly the tab's tiles".into(),
            ));
        }
        let mut root = root;
        clamp_ratios(&mut root);
        if !t.is_dynamic() {
            if t.root == root {
                return Ok(false);
            }
            t.root = root;
            return Ok(true);
        }
        let before = (t.layout.clone(), t.root.clone());
        match dynamic::fit(&t.layout, &root.rects()) {
            Some(layout) => {
                t.layout = layout;
                t.sync();
            }
            None if t.layout.kind == LayoutKind::Monocle => {}
            None => {
                t.layout.previous = Some(t.layout.kind);
                t.layout.kind = LayoutKind::Manual;
                t.layout.order.clear();
                t.layout.weights.clear();
                t.layout.manual = None;
                t.root = root;
            }
        }
        Ok((t.layout.clone(), t.root.clone()) != before)
    }

    /// Zooms `zoomed` (focusing it) or unzooms a tab, without activating it:
    /// for zoom changed elsewhere (ADR 0012). Returns whether it changed.
    pub fn set_tab_zoom(&mut self, tab: TabId, zoomed: Option<TileId>) -> Result<bool, ModelError> {
        let p = self.tab_index(tab)?;
        let t = self.tab_at_mut(p);
        if let Some(z) = zoomed
            && t.tile(z).is_none()
        {
            return Err(ModelError::TileNotFound(z));
        }
        let before = (t.zoomed, t.focused);
        t.zoomed = zoomed;
        if let Some(z) = zoomed {
            t.set_focused(z);
        }
        Ok((t.zoomed, t.focused) != before)
    }

    /// Focuses `tile` within its tab without activating the tab: for a
    /// Monocle tab whose top tile changed elsewhere (ADR 0012).
    pub fn set_tab_focus(&mut self, tab: TabId, tile: TileId) -> Result<bool, ModelError> {
        let p = self.tab_index(tab)?;
        let t = self.tab_at_mut(p);
        if t.tile(tile).is_none() {
            return Err(ModelError::TileNotFound(tile));
        }
        if t.focused == tile {
            return Ok(false);
        }
        if t.zoomed.is_some_and(|z| z != tile) {
            t.zoomed = None;
        }
        t.set_focused(tile);
        Ok(true)
    }

    /// Switches to `kind`, or back to the previous kind if `kind` is already
    /// active (Bismuth's toggle).
    pub fn toggle_layout(&mut self, tab: TabId, kind: LayoutKind) -> Result<bool, ModelError> {
        let layout = &self.tab(tab)?.layout;
        let target = if layout.kind == kind {
            layout
                .previous
                .filter(|p| *p != kind)
                .unwrap_or(if kind == LayoutKind::Tile {
                    LayoutKind::Manual
                } else {
                    LayoutKind::Tile
                })
        } else {
            kind
        };
        self.set_layout(tab, target)
    }

    /// Steps through the configured cycle of kinds. From a kind not in the
    /// cycle, forward goes to the first and back to the last.
    pub fn cycle_layout(&mut self, tab: TabId, step: i32) -> Result<bool, ModelError> {
        let current = self.tab(tab)?.layout.kind;
        let cycle = self.prefs.cycle.clone();
        if cycle.is_empty() || step == 0 {
            return Ok(false);
        }
        let n = cycle.len() as i64;
        let next = match cycle.iter().position(|k| *k == current) {
            Some(i) => (i as i64 + i64::from(step)).rem_euclid(n),
            None if step > 0 => 0,
            None => n - 1,
        };
        self.set_layout(tab, cycle[next as usize])
    }

    /// Adds `delta` masters to a Tile or Three Column tab (clamped). Other
    /// kinds have no masters.
    pub fn change_masters(&mut self, tab: TabId, delta: i32) -> Result<bool, ModelError> {
        let p = self.tab_index(tab)?;
        let t = self.tab_at_mut(p);
        let (masters, min) = match t.layout.kind {
            LayoutKind::Tile => (&mut t.layout.params.tile.masters, 0),
            LayoutKind::ThreeColumn => (&mut t.layout.params.three_column.masters, 1),
            _ => return Ok(false),
        };
        let max = i32::from(dynamic::TILE_MASTERS_MAX);
        let new = (i32::from(*masters).saturating_add(delta)).clamp(min, max) as u8;
        let changed = new != *masters;
        *masters = new;
        t.sync();
        Ok(changed)
    }

    /// Grows (positive `delta`) or shrinks a Tile or Three Column tab's
    /// master area, clamped to the kind's range.
    pub fn change_master_ratio(&mut self, tab: TabId, delta: f64) -> Result<bool, ModelError> {
        if !delta.is_finite() {
            return Err(ModelError::InvalidValue(format!(
                "ratio change {delta} is not finite"
            )));
        }
        let p = self.tab_index(tab)?;
        let t = self.tab_at_mut(p);
        let before = t.layout.params;
        match t.layout.kind {
            LayoutKind::Tile => t.layout.params.tile.ratio += delta,
            LayoutKind::ThreeColumn => t.layout.params.three_column.ratio += delta,
            _ => return Ok(false),
        }
        t.layout.params.clamp();
        t.sync();
        Ok(t.layout.params != before)
    }

    /// Moves a tile `step` places along its dynamic tab's order, one place at
    /// a time. As Bismuth's move to next/previous position: a step swaps it
    /// with its neighbour, and past either end it wraps round to the other
    /// end, the rest shifting up one (the last tile becomes master; the
    /// master goes to the end). Manual tabs have no order.
    pub fn move_in_order(&mut self, tile: TileId, step: i32) -> Result<bool, ModelError> {
        let p = self.locate(tile)?;
        let t = self.tab_at_mut(p);
        if !t.is_dynamic() || t.layout.order.len() < 2 || step == 0 {
            return Ok(false);
        }
        let n = t.layout.order.len();
        for _ in 0..step.unsigned_abs().min(n as u32) {
            let order = &mut t.layout.order;
            let i = order.iter().position(|x| *x == tile).expect("ordered");
            match (step > 0, i) {
                (true, i) if i + 1 == n => {
                    order.remove(i);
                    order.insert(0, tile);
                }
                (true, i) => order.swap(i, i + 1),
                (false, 0) => {
                    order.remove(0);
                    order.push(tile);
                }
                (false, i) => order.swap(i, i - 1),
            }
        }
        t.zoomed = None;
        t.sync();
        Ok(true)
    }

    // ----- invariants -----

    /// Checks every structural rule; returns the first one broken.
    pub fn check_invariants(&self) -> Result<(), String> {
        let mut seen = HashSet::new();
        let mut ids = HashSet::new();
        match (self.focused_window, self.windows.is_empty()) {
            (None, false) => return Err("windows exist but none is focused".into()),
            (Some(id), _) if self.window(id).is_err() => {
                return Err(format!("focused window {id} doesn't exist"));
            }
            _ => {}
        }
        for window in &self.windows {
            if !ids.insert(window.id.to_string()) {
                return Err(format!("duplicate window {}", window.id));
            }
            if window.workspaces.is_empty() {
                return Err(format!("window {} has no workspaces", window.id));
            }
            if !window
                .workspaces
                .iter()
                .any(|w| w.id == window.active_workspace)
            {
                return Err(format!("window {} has a stale active workspace", window.id));
            }
            for ws in &window.workspaces {
                if !ids.insert(ws.id.to_string()) {
                    return Err(format!("duplicate workspace {}", ws.id));
                }
                match ws.active_tab {
                    None if !ws.tabs.is_empty() => {
                        return Err(format!("workspace {} has tabs but none active", ws.id));
                    }
                    Some(id) if !ws.tabs.iter().any(|t| t.id == id) => {
                        return Err(format!("workspace {} has a stale active tab", ws.id));
                    }
                    _ => {}
                }
                for tab in &ws.tabs {
                    if !ids.insert(tab.id.to_string()) {
                        return Err(format!("duplicate tab {}", tab.id));
                    }
                    check_tab(tab, &mut seen)?;
                }
            }
        }
        Ok(())
    }
}

/// Brings every split ratio into the allowed range (non-finite ones to ½).
fn clamp_ratios(node: &mut Node) {
    if let Node::Split { ratio, a, b, .. } = node {
        *ratio = if ratio.is_finite() {
            ratio.clamp(crate::layout::MIN_RATIO, crate::layout::MAX_RATIO)
        } else {
            0.5
        };
        clamp_ratios(a);
        clamp_ratios(b);
    }
}

fn check_tab(tab: &Tab, seen: &mut HashSet<TileId>) -> Result<(), String> {
    let leaves = tab.root.tiles();
    if leaves.is_empty() {
        return Err(format!("tab {} has an empty tree", tab.id));
    }
    let mut leaf_set = HashSet::new();
    for leaf in &leaves {
        if !leaf_set.insert(*leaf) {
            return Err(format!("tile {leaf} appears twice in tab {}", tab.id));
        }
        if !seen.insert(*leaf) {
            return Err(format!("tile {leaf} appears in more than one tab"));
        }
    }
    let tile_set: HashSet<TileId> = tab.tiles.iter().map(|t| t.id).collect();
    if tile_set.len() != tab.tiles.len() {
        return Err(format!("tab {} lists a tile twice", tab.id));
    }
    if tile_set != leaf_set {
        return Err(format!("tab {} tiles don't match its tree leaves", tab.id));
    }
    if !leaf_set.contains(&tab.focused) {
        return Err(format!("tab {} focuses a missing tile", tab.id));
    }
    if let Some(z) = tab.zoomed
        && !leaf_set.contains(&z)
    {
        return Err(format!("tab {} zooms a missing tile", tab.id));
    }
    for ratio in tab.root.ratios() {
        if !(ratio > 0.0 && ratio < 1.0) {
            return Err(format!("tab {} has ratio {ratio} outside (0, 1)", tab.id));
        }
    }
    tab.layout
        .check(&leaf_set)
        .map_err(|e| format!("tab {}: {e}", tab.id))?;
    if tab.is_dynamic() {
        let want = tab.layout.frozen_tree().rects();
        let got = tab.root.rects();
        let same = want.len() == got.len()
            && want.iter().all(|(t, w)| {
                got.iter().any(|(u, g)| {
                    u == t
                        && (w.x - g.x).abs() < 1e-6
                        && (w.y - g.y).abs() < 1e-6
                        && (w.width - g.width).abs() < 1e-6
                        && (w.height - g.height).abs() < 1e-6
                })
            });
        if !same {
            return Err(format!("tab {}'s tree doesn't match its layout", tab.id));
        }
    }
    let mut recent = HashSet::new();
    for t in &tab.recent {
        if !leaf_set.contains(t) || !recent.insert(*t) {
            return Err(format!("tab {} remembers a stray recent tile {t}", tab.id));
        }
    }
    Ok(())
}
