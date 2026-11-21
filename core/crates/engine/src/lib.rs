//! Core state machine: a command in, a reply and events out.
//!
//! Events come from diffing the session before and after each command, so no
//! command handler can forget to report a change.

pub mod adopt;
pub mod agent;
mod diff;
mod driver;
mod files;
pub mod layout_file;
pub mod link;
mod meta;
pub mod mirror;
pub mod persist;
mod remote_files;
mod repos;
pub mod terminal;
mod transfer;

use azulejo_model::{
    HerdrRef, LayoutPrefs, ModelError, Session, SidebarState, TabId, Tile, TileContent, TileId,
    WorkspaceId,
};
use azulejo_protocol::{
    API_VERSION, Command, Created, Error, ErrorCode, Event, LayoutState, Reply, TabSummary,
    TileInfo, WindowSummary, WorkspaceSummary,
};
use serde_json::json;

pub use diff::diff;
pub use driver::{CommandsFn, ConfigRepliesFn, Core, CoreOptions, LogFn};
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
    /// A fresh session's first tab, made before the layout preferences
    /// arrive: it takes the configured default kind when they do.
    fresh_tab: Option<TabId>,
    /// When each pane's agent status last came in on herdr's stream, so a
    /// survey requested before that can't put an older status back.
    agent_reported: std::collections::HashMap<(link::Machine, String), u64>,
    /// When each pane's title or working directory last came in on herdr's
    /// stream or from the process poller, for the same reason.
    info_reported: std::collections::HashMap<(link::Machine, String), u64>,
    /// Panes herdr reported closed (or exited, or gone with their
    /// workspace), with when: a survey requested before that still lists
    /// them and mustn't adopt them again.
    closed_reported: Vec<(azulejo_model::HerdrRef, u64)>,
    /// What a collapsed sidebar shows as (`[sidebar] collapsed`).
    sidebar_collapsed: SidebarState,
    /// herdr's machines other than Local, in `machines.list` order: the
    /// order of the sidebar's machine groups, which each window's
    /// workspaces keep ([`Session::group_workspaces`]).
    machine_order: Vec<String>,
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
        let tab = session
            .create_tab(workspace, Tile::new(TileContent::terminal()), None)
            .expect("new workspace exists");
        meta::refresh_headers(&mut session);
        Self {
            session,
            placeholder: Some(workspace),
            agent_reported: Default::default(),
            info_reported: Default::default(),
            closed_reported: Vec::new(),
            fresh_tab: Some(tab),
            sidebar_collapsed: SidebarState::Rail,
            machine_order: Vec::new(),
        }
    }

    /// Starts from a saved session.
    /// A collapsed sidebar comes back as a rail until the config says
    /// otherwise ([`Engine::set_sidebar_collapsed`]); state saved before the
    /// rail only says "hidden".
    pub fn with_session(mut session: Session) -> Self {
        meta::refresh_headers(&mut session);
        session.set_sidebar_collapsed(SidebarState::Rail);
        session.group_workspaces(&[]);
        Self {
            session,
            placeholder: None,
            fresh_tab: None,
            agent_reported: Default::default(),
            info_reported: Default::default(),
            closed_reported: Vec::new(),
            sidebar_collapsed: SidebarState::Rail,
            machine_order: Vec::new(),
        }
    }

    /// Applies `[sidebar] collapsed`: what collapsing a sidebar gives (a
    /// rail, or hidden; `Full` counts as a rail), and every sidebar that's
    /// collapsed now takes that form. Returns the events for any change.
    pub fn set_sidebar_collapsed(&mut self, collapsed: SidebarState) -> Vec<Event> {
        let before = self.session.clone();
        self.sidebar_collapsed = match collapsed {
            SidebarState::Hidden => SidebarState::Hidden,
            SidebarState::Full | SidebarState::Rail => SidebarState::Rail,
        };
        self.session.set_sidebar_collapsed(self.sidebar_collapsed);
        diff(&before, &self.session, false)
    }

    /// Applies the `[layout]` preferences and returns the events
    /// for any change. A fresh session's untouched first tab switches to the
    /// default kind, so a first launch starts the way new tabs will.
    pub fn set_layout_prefs(&mut self, prefs: LayoutPrefs) -> Vec<Event> {
        let before = self.session.clone();
        self.session.prefs = prefs;
        if let Some(tab) = self.fresh_tab.take()
            && let Ok(t) = self.session.tab(tab)
            && t.tiles.len() == 1
            && t.layout.is_plain_manual()
        {
            let kind = self.session.prefs.default;
            let _ = self.session.set_layout(tab, kind);
        }
        diff(&before, &self.session, false)
    }

    /// herdr's machine list changed (`machines`: its IDs in `machines.list`
    /// order, Local's left out): workspaces regroup to match the sidebar's
    /// groups. Returns the events for any change.
    pub fn set_machine_order(&mut self, machines: Vec<String>) -> Vec<Event> {
        if machines == self.machine_order {
            return Vec::new();
        }
        let before = self.session.clone();
        self.machine_order = machines;
        if !self.session.group_workspaces(&self.machine_order) {
            return Vec::new();
        }
        diff(&before, &self.session, false)
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
        // Focus alone doesn't count as reading an agent's result: the shell
        // knows whether the tile is on screen and sends `tile.seen`.
        meta::refresh_headers(&mut self.session);
        // A new workspace, or one moved past its machine's group, lands at
        // that group's edge: numbers follow the sidebar.
        self.session.group_workspaces(&self.machine_order);
        let mut events = diff(&before, &self.session, kill);
        self.remember_detached(&events);
        if force_layout && let Reply::Layout(state) = &reply {
            events.push(Event::LayoutChanged(state.clone()));
        }
        Ok(Outcome { reply, events })
    }

    /// Remembers the herdr panes of closed terminal tiles, so adoption
    /// doesn't bring them back. Killed panes go on the list too:
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
        let collapsed = self.sidebar_collapsed;
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
            Command::WindowSetSidebar(p) => {
                s.set_window_sidebar(p.window, p.visible, collapsed)?;
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
                        sidebar: w.sidebar,
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
                        w.workspaces
                            .iter()
                            .enumerate()
                            .map(move |(i, ws)| WorkspaceSummary {
                                window: w.id,
                                number: i + 1,
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
                let machine = p.machine.filter(|m| m != azulejo_protocol::LOCAL_MACHINE);
                let workspace = s.create_workspace(window, name, p.cwd)?;
                s.set_workspace_machine(workspace, machine)?;
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
            Command::WorkspaceSetRepo(p) => {
                let id = p.workspace.or(focus.workspace).ok_or_else(nothing)?;
                let repo = p.path.map(|p| files::expand_home(&p).display().to_string());
                s.set_workspace_repo(id, repo)?;
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
            Command::TileSeen(p) => {
                agent::mark_seen(s, s.target(p.tile)?);
                Reply::Ok
            }
            Command::TileMarkUnread(p) => {
                let id = s.target(p.tile)?;
                if agent::mark_unread(s, id).is_none() {
                    return Err(Error::new(
                        ErrorCode::InvalidState,
                        "only a tile with an agent can be marked unread",
                    )
                    .with_data(json!({ "tile": id })));
                }
                Reply::Ok
            }
            Command::TileRename(p) => {
                let id = s.target(p.tile)?;
                s.tile_mut(id)?.title = p.title.filter(|t| !t.is_empty());
                Reply::Ok
            }
            Command::FilesOpen(p) => Reply::Created(files_open(s, p)?),
            // Listing and watching touch the disk; `Core` answers these.
            Command::FilesList(_)
            | Command::FilesExpand(_)
            | Command::FilesCollapse(_)
            | Command::FilesReveal(_)
            | Command::FilesToggleHidden(_)
            | Command::FilesPanel(_)
            | Command::FilesPanelClose(_) => {
                return Err(Error::new(
                    ErrorCode::Unavailable,
                    "files panels need a running core",
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
                let tab = p.tab.or(focus.tab).ok_or_else(nothing)?;
                match (p.part, p.flip) {
                    (true, true) => {
                        return Err(Error::new(
                            ErrorCode::InvalidParams,
                            "give at most one of `part` and `flip`",
                        ));
                    }
                    (true, false) => s.rotate_part(tab)?,
                    (false, true) => s.flip(tab)?,
                    (false, false) => s.rotate_turn(tab, p.reverse)?,
                };
                Reply::Ok
            }
            Command::LayoutSet(p) => {
                s.set_layout(p.tab.or(focus.tab).ok_or_else(nothing)?, p.kind)?;
                Reply::Ok
            }
            Command::LayoutToggle(p) => {
                s.toggle_layout(p.tab.or(focus.tab).ok_or_else(nothing)?, p.kind)?;
                Reply::Ok
            }
            Command::LayoutCycle(p) => {
                s.cycle_layout(p.tab.or(focus.tab).ok_or_else(nothing)?, p.step)?;
                Reply::Ok
            }
            Command::LayoutMasters(p) => {
                s.change_masters(p.tab.or(focus.tab).ok_or_else(nothing)?, p.delta)?;
                Reply::Ok
            }
            Command::LayoutMasterRatio(p) => {
                s.change_master_ratio(p.tab.or(focus.tab).ok_or_else(nothing)?, p.delta)?;
                Reply::Ok
            }
            Command::LayoutMoveOrder(p) => {
                s.move_in_order(s.target(p.tile)?, p.step)?;
                Reply::Ok
            }
            Command::LayoutSetRatio(p) => {
                let path = azulejo_model::parse_path(&p.path)?;
                s.set_split_ratio(p.tab.or(focus.tab).ok_or_else(nothing)?, &path, p.ratio)?;
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
                        // Another process: whatever agent ran before is gone.
                        if herdr
                            .as_ref()
                            .is_some_and(|h| h.terminal_id != p.herdr.terminal_id)
                        {
                            tile.agent = None;
                        }
                        *herdr = Some(p.herdr);
                        *ended = false;
                    }
                    other => return Err(wrong_kind(p.tile, other, "terminal")),
                }
                Reply::Ok
            }
            // Unbinds the ended pane; `Core` then creates a fresh one for the
            // same tile (link::create_effect).
            Command::TerminalRespawn(p) => {
                let id = s.target(p.tile)?;
                match &mut s.tile_mut(id)?.content {
                    TileContent::Terminal { herdr, ended, .. } => {
                        if !*ended {
                            return Err(Error::new(
                                ErrorCode::InvalidState,
                                "the terminal's herdr pane is still running",
                            )
                            .with_data(json!({ "tile": id })));
                        }
                        *herdr = None;
                        *ended = false;
                    }
                    other => return Err(wrong_kind(id, other, "terminal")),
                }
                s.tile_mut(id)?.agent = None;
                Reply::Ok
            }
            Command::LayoutApply(p) => layout_file::apply(s, focus, p)?,
            Command::LayoutExport(p) => Reply::LayoutFile {
                toml: layout_file::export(s, focus, &p)?,
                path: p.path,
            },
            // The palette's entries and the keymap come from the config,
            // which `Core` holds.
            Command::CommandsList => {
                return Err(Error::new(
                    ErrorCode::Unavailable,
                    "the command list needs a running core with a config",
                ));
            }
            Command::ConfigKeys | Command::ConfigDefaults => {
                return Err(Error::new(
                    ErrorCode::Unavailable,
                    "the keymap needs a running core with a config",
                ));
            }
            // Copying files takes time, and machines are links `Core` holds.
            Command::FilesTransfer(_) => {
                return Err(Error::new(
                    ErrorCode::Unavailable,
                    "file transfers need a running core",
                ));
            }
            // Machines are links `Core` holds.
            Command::MachinesList => {
                return Err(Error::new(
                    ErrorCode::Unavailable,
                    "machines need a running core",
                ));
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
            // Streams belong to a socket connection; the API server answers these.
            Command::EventsSubscribe(_) | Command::EventsUnsubscribe(_) => {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "event streams are served by the API socket",
                ));
            }
        })
    }

    /// The herdr pane behind a terminal tile (the focused tile when `tile` is
    /// absent), for terminal I/O.
    pub fn terminal_pane(&self, tile: Option<TileId>) -> Result<HerdrRef, Error> {
        let id = self.session.target(tile)?;
        match &self.session.tile(id)?.content {
            TileContent::Terminal { ended: true, .. } => Err(Error::new(
                ErrorCode::InvalidState,
                "the terminal's herdr pane has ended",
            )
            .with_data(json!({ "tile": id }))),
            TileContent::Terminal { herdr: None, .. } => Err(Error::new(
                ErrorCode::Unavailable,
                "the terminal isn't attached to a herdr pane yet",
            )
            .with_data(json!({ "tile": id }))),
            TileContent::Terminal { herdr: Some(h), .. } => Ok(h.clone()),
            other => Err(wrong_kind(id, other, "terminal")),
        }
    }
}

/// `files.open`: opens an absolute path beside `tile` (the focused tile
/// when absent; the driver sends a files panel's paths here, made absolute,
/// without one). A file opens in the editor, a new terminal to the right
/// in the file's folder ([`editor_command`]); a folder, or any path with the
/// `terminal` target, opens a new terminal below, in the folder (a file's
/// folder for a file). The new tile takes focus. In a remote workspace the
/// path is on its machine, where the terminal runs: the driver has already
/// worked out what a remote panel's path is.
fn files_open(s: &mut Session, p: azulejo_protocol::FilesOpen) -> Result<Created, Error> {
    use azulejo_model::Direction;
    use azulejo_protocol::FilesOpenTarget;
    use std::path::{Path, PathBuf};
    let id = s.target(p.tile)?;
    let path = files::expand_home(&p.path);
    if !path.is_absolute() {
        return Err(Error::new(
            ErrorCode::InvalidParams,
            format!(
                "{} isn't absolute; files.open takes absolute paths, or a files panel's",
                p.path
            ),
        ));
    }
    let (_, workspace, _) = s.ids_at(s.locate(id)?);
    let remote = s.workspace(workspace)?.machine.is_some();
    let folder_of = |p: &Path| {
        p.parent()
            .map_or_else(|| p.to_path_buf(), Path::to_path_buf)
    };
    let folder = remote || path.is_dir();
    let terminal = |cwd: PathBuf, command: Option<Vec<String>>| {
        let mut content = TileContent::terminal();
        if let TileContent::Terminal {
            cwd: c, command: k, ..
        } = &mut content
        {
            *c = Some(cwd.display().to_string());
            *k = command;
        }
        Tile::new(content)
    };
    let (tile, direction) = match p.target {
        FilesOpenTarget::Editor if !(path.is_dir() && !remote) => (
            terminal(folder_of(&path), Some(editor_command(&path))),
            Direction::Right,
        ),
        _ => {
            let cwd = if folder { path } else { folder_of(&path) };
            (terminal(cwd, None), Direction::Down)
        }
    };
    let new = tile.id;
    s.split(id, direction, tile)?;
    Ok(Created {
        tile: Some(new),
        ..Created::default()
    })
}

/// The command a terminal runs to edit `path`: `vim <path>`, sent as an
/// argv (quoted for the pane's shell).
pub(crate) fn editor_command(path: &std::path::Path) -> Vec<String> {
    vec!["vim".to_owned(), path.display().to_string()]
}

fn wrong_kind(id: TileId, content: &TileContent, want: &str) -> Error {
    let kind = serde_json::to_value(content.kind())
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    Error::new(
        ErrorCode::InvalidState,
        format!("that's a {kind} tile, not a {want} tile"),
    )
    .with_data(json!({ "tile": id, "kind": kind }))
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
    Ok(LayoutState::of(window, workspace, tab))
}

#[cfg(test)]
mod tests;
