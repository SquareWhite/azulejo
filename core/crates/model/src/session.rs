//! The whole object model: windows → workspaces → tabs → layout tree → tiles.
//!
//! Every operation either succeeds or returns an error with the session
//! untouched. `check_invariants` states the rules; the proptests hold every
//! operation to them.

use std::collections::HashSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ModelError;
use crate::ids::{TabId, TileId, WindowId, WorkspaceId};
use crate::layout::{Direction, Node};
use crate::tile::Tile;

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
}

impl Tab {
    fn new(tile: Tile, name: Option<String>) -> Self {
        Self {
            id: TabId::new(),
            name,
            root: Node::leaf(tile.id),
            focused: tile.id,
            tiles: vec![tile],
            zoomed: None,
        }
    }

    pub fn tile(&self, id: TileId) -> Option<&Tile> {
        self.tiles.iter().find(|t| t.id == id)
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
    pub tabs: Vec<Tab>,
    /// `None` exactly when there are no tabs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tab: Option<TabId>,
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
            tabs: Vec::new(),
            active_tab: None,
        };
        let window = Window {
            id: WindowId::new(),
            active_workspace: ws.id,
            workspaces: vec![ws],
            frame: None,
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

    /// Records where the shell put a window.
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
        let tab = Tab::new(tile, name);
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
        let tab = self.tab_at_mut(p);
        tab.root.split(target, direction, tile.id)?;
        tab.focused = tile.id;
        tab.zoomed = None;
        tab.tiles.push(tile);
        self.activate(p);
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
        let next = tab.root.remove(id).expect("tile is in a multi-tile tree");
        if tab.focused == id {
            tab.focused = next;
        }
        if tab.zoomed == Some(id) {
            tab.zoomed = None;
        }
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
        tab.focused = id;
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
        let next =
            self.tab_at(p)
                .root
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
            .root
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
        tab.root.swap(a, b)?;
        tab.zoomed = None;
        Ok(())
    }

    /// Moves a tile one step in `direction`; see [`Node::move_tile`].
    pub fn move_tile(&mut self, tile: TileId, direction: Direction) -> Result<bool, ModelError> {
        let p = self.locate(tile)?;
        let tab = self.tab_at_mut(p);
        let moved = tab.root.move_tile(tile, direction)?;
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
        self.tab_at_mut(p).root.resize(tile, direction, amount)
    }

    /// Zooms or unzooms a tile (`None` toggles) and focuses it. Returns whether
    /// it ends up zoomed.
    pub fn zoom(&mut self, tile: TileId, zoomed: Option<bool>) -> Result<bool, ModelError> {
        let p = self.locate(tile)?;
        let tab = self.tab_at_mut(p);
        let on = zoomed.unwrap_or(tab.zoomed != Some(tile));
        tab.zoomed = on.then_some(tile);
        tab.focused = tile;
        self.activate(p);
        Ok(on)
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
    Ok(())
}
