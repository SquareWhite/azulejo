//! Core state machine: a command in, a reply and events out.
//!
//! Events come from diffing the session before and after each command, so no
//! command handler can forget to report a change.

pub mod adopt;
mod diff;
mod driver;
pub mod link;
pub mod persist;
mod preview;
pub mod terminal;

use azulejo_model::{HerdrRef, ModelError, Session, Tile, TileContent, TileId, WorkspaceId};
use azulejo_protocol::{
    API_VERSION, Command, Created, Error, ErrorCode, Event, LayoutState, Reply, TabSummary,
    TileInfo, WindowSummary, WorkspaceSummary,
};

pub use diff::diff;
pub use driver::{Core, CoreOptions, LogFn};
pub use link::{Effect, HerdrPort, Internal};

/// The result of a successful command.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub reply: Reply,
    pub events: Vec<Event>,
}

#[derive(Debug)]
pub struct Engine {
    session: Session,
    /// The empty workspace a fresh session starts with, until the first herdr
    /// survey: dropped then if adoption brought in the user's workspaces.
    placeholder: Option<WorkspaceId>,
}

/// How many closed-here herdr panes `Session::detached_herdr` remembers.
const MAX_DETACHED: usize = 1024;

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// A fresh session: one window, one workspace, one tab with a terminal.
    pub fn new() -> Self {
        let mut session = Session::new();
        let (_, workspace) = session.create_window("main", None);
        session
            .create_tab(workspace, Tile::new(TileContent::terminal()), None)
            .expect("new workspace exists");
        Self {
            session,
            placeholder: Some(workspace),
        }
    }

    /// Starts from a saved session.
    pub fn with_session(session: Session) -> Self {
        Self {
            session,
            placeholder: None,
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Runs a command. On error the session is unchanged and no events fire.
    pub fn execute(&mut self, command: Command) -> Result<Outcome, Error> {
        let before = self.session.clone();
        let kill = matches!(&command, Command::TileClose(p) if p.kill);
        let force_layout = matches!(command, Command::LayoutGet);
        let reply = match self.apply(command) {
            Ok(reply) => reply,
            Err(e) => {
                self.session = before;
                return Err(e);
            }
        };
        let mut events = diff(&before, &self.session, kill);
        self.remember_detached(&events);
        if force_layout && let Reply::Layout(state) = &reply {
            events.push(Event::LayoutChanged(state.clone()));
        }
        Ok(Outcome { reply, events })
    }

    /// Remembers the herdr panes of closed terminal tiles, so adoption
    /// doesn't bring them back (ADR 0008). Killed panes go on the list too:
    /// the kill reaches herdr after a survey may already have seen them.
    fn remember_detached(&mut self, events: &[Event]) {
        for event in events {
            if let Event::TileClosed { tile, .. } = event
                && let TileContent::Terminal { herdr: Some(h), .. } = &tile.content
                && !self.session.detached_herdr.contains(h)
            {
                self.session.detached_herdr.push(h.clone());
            }
        }
        let len = self.session.detached_herdr.len();
        if len > MAX_DETACHED {
            self.session.detached_herdr.drain(..len - MAX_DETACHED);
        }
    }

    /// Runs a command and returns only its events; for shells that render from
    /// events and don't need the reply yet.
    pub fn handle(&mut self, command: Command) -> Vec<Event> {
        self.execute(command).map(|o| o.events).unwrap_or_default()
    }

    fn apply(&mut self, command: Command) -> Result<Reply, Error> {
        let s = &mut self.session;
        let focus = s.focus();
        let nothing = || Error::from(ModelError::NothingFocused);
        Ok(match command {
            Command::Ping => Reply::Pong {
                api_version: API_VERSION,
            },
            Command::SessionGet => Reply::Session { session: s.clone() },

            Command::WindowCreate(p) => {
                let (window, workspace) =
                    s.create_window(p.workspace_name.unwrap_or_else(|| "main".into()), p.cwd);
                Reply::Created(Created {
                    window: Some(window),
                    workspace: Some(workspace),
                    ..Created::default()
                })
            }
            Command::WindowClose(p) => {
                s.close_window(p.window)?;
                Reply::Ok
            }
            Command::WindowFocus(p) => {
                s.focus_window(p.window)?;
                Reply::Ok
            }
            Command::WindowSetFrame(p) => {
                s.set_window_frame(p.window, p.frame)?;
                Reply::Ok
            }
            Command::WindowList => Reply::Windows {
                windows: s
                    .windows
                    .iter()
                    .map(|w| WindowSummary {
                        id: w.id,
                        focused: focus.window == Some(w.id),
                        active_workspace: w.active_workspace,
                        workspaces: w.workspaces.iter().map(|ws| ws.id).collect(),
                        frame: w.frame,
                    })
                    .collect(),
            },
            Command::WorkspaceList(p) => {
                if let Some(window) = p.window {
                    s.window(window)?;
                }
                let workspaces = s
                    .windows
                    .iter()
                    .filter(|w| p.window.is_none_or(|id| id == w.id))
                    .flat_map(|w| {
                        w.workspaces.iter().map(move |ws| WorkspaceSummary {
                            window: w.id,
                            active: w.active_workspace == ws.id,
                            focused: focus.workspace == Some(ws.id),
                            tabs: ws.tabs.len(),
                            workspace: diff::workspace_info(ws),
                        })
                    })
                    .collect();
                Reply::Workspaces { workspaces }
            }

            Command::WorkspaceCreate(p) => {
                let window = p.window.or(focus.window).ok_or_else(nothing)?;
                let count = s.window(window)?.workspaces.len();
                let name = p.name.unwrap_or_else(|| format!("workspace {}", count + 1));
                let workspace = s.create_workspace(window, name, p.cwd)?;
                Reply::Created(Created {
                    workspace: Some(workspace),
                    ..Created::default()
                })
            }
            Command::WorkspaceClose(p) => {
                s.close_workspace(p.workspace.or(focus.workspace).ok_or_else(nothing)?)?;
                Reply::Ok
            }
            Command::WorkspaceFocus(p) => {
                s.focus_workspace(p.workspace.or(focus.workspace).ok_or_else(nothing)?)?;
                Reply::Ok
            }
            Command::WorkspaceRename(p) => {
                s.rename_workspace(p.workspace.or(focus.workspace).ok_or_else(nothing)?, p.name)?;
                Reply::Ok
            }
            Command::WorkspaceMove(p) => {
                s.move_workspace(
                    p.workspace.or(focus.workspace).ok_or_else(nothing)?,
                    p.index,
                )?;
                Reply::Ok
            }
            Command::WorkspaceBind(p) => {
                s.bind_workspace(p.workspace, p.herdr_workspace)?;
                Reply::Ok
            }

            Command::TabList(p) => {
                if let Some(workspace) = p.workspace {
                    s.workspace(workspace)?;
                }
                let tabs = s
                    .windows
                    .iter()
                    .flat_map(|w| w.workspaces.iter().map(move |ws| (w.id, ws)))
                    .filter(|(_, ws)| p.workspace.is_none_or(|id| id == ws.id))
                    .flat_map(|(window, ws)| {
                        ws.tabs.iter().map(move |tab| TabSummary {
                            window,
                            workspace: ws.id,
                            active: ws.active_tab == Some(tab.id),
                            focused: focus.tab == Some(tab.id),
                            tiles: tab.tiles.len(),
                            focused_tile: tab.focused,
                            tab: diff::tab_info(tab),
                        })
                    })
                    .collect();
                Reply::Tabs { tabs }
            }
            Command::TabCreate(p) => {
                let workspace = p.workspace.or(focus.workspace).ok_or_else(nothing)?;
                let tile = Tile::new(p.content.unwrap_or_else(TileContent::terminal));
                let tile_id = tile.id;
                let tab = s.create_tab(workspace, tile, p.name)?;
                Reply::Created(Created {
                    tab: Some(tab),
                    tile: Some(tile_id),
                    ..Created::default()
                })
            }
            Command::TabClose(p) => {
                s.close_tab(p.tab.or(focus.tab).ok_or_else(nothing)?)?;
                Reply::Ok
            }
            Command::TabFocus(p) => {
                s.focus_tab(p.tab.or(focus.tab).ok_or_else(nothing)?)?;
                Reply::Ok
            }
            Command::TabRename(p) => {
                s.rename_tab(p.tab.or(focus.tab).ok_or_else(nothing)?, p.name)?;
                Reply::Ok
            }
            Command::TabMove(p) => {
                s.move_tab(p.tab.or(focus.tab).ok_or_else(nothing)?, p.index)?;
                Reply::Ok
            }

            Command::TileList(p) => {
                if let Some(workspace) = p.workspace {
                    s.workspace(workspace)?;
                }
                let tiles = s
                    .tiles()
                    .map(|(path, tile)| tile_info(s, path, tile, focus.tile))
                    .filter(|info| p.workspace.is_none_or(|w| info.workspace == w))
                    .collect();
                Reply::Tiles { tiles }
            }
            Command::TileGet(p) => {
                let id = s.target(p.tile)?;
                let path = s.locate(id)?;
                Reply::Tile(tile_info(s, path, s.tile(id)?, focus.tile))
            }
            Command::TileClose(p) => {
                s.close_tile(s.target(p.tile)?)?;
                Reply::Ok
            }
            Command::TileFocus(p) => {
                s.focus_tile(s.target(p.tile)?)?;
                Reply::Ok
            }
            Command::TileRename(p) => {
                let id = s.target(p.tile)?;
                s.tile_mut(id)?.title = p.title.filter(|t| !t.is_empty());
                Reply::Ok
            }
            Command::BrowserOpen(p) => {
                let id = s.target(p.tile)?;
                match &mut s.tile_mut(id)?.content {
                    TileContent::Browser { url, .. } => *url = Some(p.url),
                    other => return Err(wrong_kind(id, other, "browser")),
                }
                Reply::Ok
            }
            Command::BrowserState(p) => {
                let id = s.target(p.tile)?;
                match &mut s.tile_mut(id)?.content {
                    TileContent::Browser { url, page_title } => {
                        if let Some(new) = p.url {
                            *url = Some(new);
                        }
                        *page_title = p.page_title.filter(|t| !t.is_empty());
                    }
                    other => return Err(wrong_kind(id, other, "browser")),
                }
                Reply::Ok
            }
            Command::PreviewOpen(p) => {
                let id = s.target(p.tile)?;
                match &mut s.tile_mut(id)?.content {
                    TileContent::Preview { path } => *path = Some(p.path),
                    other => return Err(wrong_kind(id, other, "preview")),
                }
                Reply::Ok
            }
            // Rendering reads files; `Core` answers this off the engine.
            Command::PreviewGet(_) => {
                return Err(Error::new(
                    ErrorCode::Unavailable,
                    "previews need a running core",
                ));
            }

            Command::LayoutGet => {
                let tile = focus.tile.ok_or_else(nothing)?;
                Reply::Layout(layout_state(s, tile)?)
            }
            Command::LayoutSplit(p) => {
                let target = s.target(p.tile)?;
                let tile = Tile::new(p.content.unwrap_or_else(TileContent::terminal));
                let id = tile.id;
                s.split(target, p.direction, tile)?;
                Reply::Created(Created {
                    tile: Some(id),
                    ..Created::default()
                })
            }
            Command::LayoutFocus(p) => {
                let from = s.target(None)?;
                s.focus_direction(from, p.direction)?;
                Reply::Ok
            }
            Command::LayoutSwap(p) => {
                let tile = s.target(p.tile)?;
                match (p.direction, p.with) {
                    (Some(direction), None) => {
                        s.swap_direction(tile, direction)?;
                    }
                    (None, Some(other)) => s.swap_tiles(tile, other)?,
                    _ => {
                        return Err(Error::new(
                            ErrorCode::InvalidParams,
                            "give exactly one of `direction` and `with`",
                        ));
                    }
                }
                Reply::Ok
            }
            Command::LayoutMove(p) => {
                s.move_tile(s.target(p.tile)?, p.direction)?;
                Reply::Ok
            }
            Command::LayoutResize(p) => {
                s.resize(s.target(p.tile)?, p.direction, p.amount)?;
                Reply::Ok
            }
            Command::LayoutZoom(p) => {
                s.zoom(s.target(p.tile)?, p.zoomed)?;
                Reply::Ok
            }
            Command::LayoutSplitAuto(p) => {
                let tab = p.tab.or(focus.tab).ok_or_else(nothing)?;
                let tile = Tile::new(p.content.unwrap_or_else(TileContent::terminal));
                let id = tile.id;
                s.split_auto(tab, tile)?;
                Reply::Created(Created {
                    tile: Some(id),
                    ..Created::default()
                })
            }
            Command::LayoutPromote(p) => {
                s.promote(s.target(p.tile)?)?;
                Reply::Ok
            }
            Command::LayoutRotate(p) => {
                s.rotate(p.tab.or(focus.tab).ok_or_else(nothing)?)?;
                Reply::Ok
            }
            Command::LayoutEqualize(p) => {
                s.equalize(p.tab.or(focus.tab).ok_or_else(nothing)?)?;
                Reply::Ok
            }

            Command::TerminalBind(p) => {
                let tile = s.tile_mut(p.tile)?;
                match &mut tile.content {
                    TileContent::Terminal { herdr, ended, .. } => {
                        *herdr = Some(p.herdr);
                        *ended = false;
                    }
                    other => return Err(wrong_kind(p.tile, other, "terminal")),
                }
                Reply::Ok
            }
            // The engine can't do I/O; `Core` runs these against herdr.
            Command::TerminalSendText(_)
            | Command::TerminalSendKeys(_)
            | Command::TerminalRead(_)
            | Command::TerminalScroll(_)
            | Command::TerminalSearch(_) => {
                return Err(Error::new(
                    ErrorCode::Unavailable,
                    "terminal I/O needs a core connected to herdr",
                ));
            }
        })
    }

    /// The file a preview tile shows (the focused tile when `tile` is absent).
    pub fn preview_path(&self, tile: Option<TileId>) -> Result<(TileId, Option<String>), Error> {
        let id = self.session.target(tile)?;
        match &self.session.tile(id)?.content {
            TileContent::Preview { path } => Ok((id, path.clone())),
            other => Err(wrong_kind(id, other, "preview")),
        }
    }

    /// The herdr pane behind a terminal tile (the focused tile when `tile` is
    /// absent), for terminal I/O.
    pub fn terminal_pane(&self, tile: Option<TileId>) -> Result<HerdrRef, Error> {
        let id = self.session.target(tile)?;
        match &self.session.tile(id)?.content {
            TileContent::Terminal { ended: true, .. } => Err(Error::new(
                ErrorCode::InvalidState,
                format!("{id}'s herdr pane has ended"),
            )),
            TileContent::Terminal { herdr: None, .. } => Err(Error::new(
                ErrorCode::Unavailable,
                format!("{id} isn't attached to a herdr pane yet"),
            )),
            TileContent::Terminal { herdr: Some(h), .. } => Ok(h.clone()),
            other => Err(wrong_kind(id, other, "terminal")),
        }
    }
}

fn wrong_kind(id: TileId, content: &TileContent, want: &str) -> Error {
    let kind = serde_json::to_value(content.kind())
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    Error::new(
        ErrorCode::InvalidState,
        format!("{id} is a {kind} tile, not a {want} tile"),
    )
}

fn tile_info(
    s: &Session,
    path: azulejo_model::TilePath,
    tile: &Tile,
    focused: Option<TileId>,
) -> TileInfo {
    let (window, workspace, tab) = s.ids_at(path);
    TileInfo {
        window,
        workspace,
        tab,
        focused: focused == Some(tile.id),
        tile: tile.clone(),
    }
}

/// The layout of the tab holding `tile`.
fn layout_state(s: &Session, tile: TileId) -> Result<LayoutState, Error> {
    let path = s.locate(tile)?;
    let (window, workspace, tab_id) = s.ids_at(path);
    let tab = s.tab(tab_id)?;
    Ok(LayoutState {
        window,
        workspace,
        tab: tab_id,
        tree: tab.root.clone(),
        focused: tab.focused,
        zoomed: tab.zoomed,
    })
}

#[cfg(test)]
mod tests;
