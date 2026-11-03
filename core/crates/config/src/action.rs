//! Keymap actions and what each one turns into.
//!
//! An action either becomes a protocol command (the shell sends it to the core
//! like any API client would) or stays in the shell (palette, sidebar, prefix
//! mode). Actions that need session state to pick a target, such as "switch to
//! tab 3", name their method but leave building the command to the shell.

use azulejo_protocol::{
    Command, DEFAULT_RESIZE_STEP, Direction, LayoutFocus, LayoutMove, LayoutResize, LayoutSplit,
    LayoutSplitAuto, LayoutSwap, LayoutZoom, TabCreate, TabTarget, TileClose, TileTarget,
    WindowCreate, WorkspaceCreate, WorkspaceTarget,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    // Tiles
    FocusTile(Direction),
    SwapTile(Direction),
    MoveTile(Direction),
    ResizeTile(Direction),
    /// Sticky resize: hjkl resize until escape. Shell-local.
    ResizeMode,
    ZoomTile,
    /// Closes the tile and detaches its herdr pane (F14).
    CloseTile,
    /// Closes the tile and kills its herdr pane.
    KillTile,
    CycleTileNext,
    CycleTilePrevious,
    RenameTile,
    // New tiles and layout rules
    Split(Direction),
    /// Tall layout: master left, stack right, new tiles append to the stack.
    SplitAuto,
    PromoteTile,
    RotateLayout,
    EqualizeLayout,
    // Workspaces
    NewWorkspace,
    /// 1-based position in the window.
    SwitchWorkspace(u8),
    LastWorkspace,
    NextWorkspace,
    PreviousWorkspace,
    CloseWorkspace,
    RenameWorkspace,
    // Tabs
    NewTab,
    /// 1-based position in the workspace.
    SwitchTab(u8),
    LastTab,
    NextTab,
    PreviousTab,
    CloseTab,
    RenameTab,
    // Windows
    NewWindow,
    CloseWindow,
    // App
    CommandPalette,
    Goto,
    Find,
    OpenConfig,
    ReloadConfig,
    ToggleSidebar,
    EditScrollback,
    /// Arms prefix mode; the next chord is looked up in the prefix keymap.
    EnterPrefix,
}

/// Where an action ends up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Handled entirely in the shell.
    Shell,
    /// Becomes this protocol method. The shell may need session state (or a
    /// prompt, for renames) to fill in the params; [`Action::command`] builds
    /// the command when it doesn't.
    Method(&'static str),
}

/// How an action is spelled in config files.
pub(crate) enum NameSpec {
    Plain(Action),
    /// Takes an index from a digit binding (`cmd+1..8`).
    Indexed(fn(u8) -> Action),
}

const DIRS: [(&str, Direction); 4] = [
    ("left", Direction::Left),
    ("down", Direction::Down),
    ("up", Direction::Up),
    ("right", Direction::Right),
];

/// Every action name, in a fixed order (used for tie-breaks and listings).
pub(crate) fn names() -> Vec<(String, NameSpec)> {
    use Action::*;
    let mut out: Vec<(String, NameSpec)> = Vec::new();
    for (prefix, make) in [
        ("focus_tile", FocusTile as fn(Direction) -> Action),
        ("swap_tile", SwapTile),
        ("move_tile", MoveTile),
        ("resize_tile", ResizeTile),
        ("split", Split),
    ] {
        for (dir_name, dir) in DIRS {
            out.push((format!("{prefix}_{dir_name}"), NameSpec::Plain(make(dir))));
        }
    }
    let plain = [
        ("resize_mode", ResizeMode),
        ("zoom_tile", ZoomTile),
        ("close_tile", CloseTile),
        ("kill_tile", KillTile),
        ("cycle_tile_next", CycleTileNext),
        ("cycle_tile_previous", CycleTilePrevious),
        ("rename_tile", RenameTile),
        ("split_auto", SplitAuto),
        ("promote_tile", PromoteTile),
        ("rotate_layout", RotateLayout),
        ("equalize_layout", EqualizeLayout),
        ("new_workspace", NewWorkspace),
        ("last_workspace", LastWorkspace),
        ("next_workspace", NextWorkspace),
        ("previous_workspace", PreviousWorkspace),
        ("close_workspace", CloseWorkspace),
        ("rename_workspace", RenameWorkspace),
        ("new_tab", NewTab),
        ("last_tab", LastTab),
        ("next_tab", NextTab),
        ("previous_tab", PreviousTab),
        ("close_tab", CloseTab),
        ("rename_tab", RenameTab),
        ("new_window", NewWindow),
        ("close_window", CloseWindow),
        ("command_palette", CommandPalette),
        ("goto", Goto),
        ("find", Find),
        ("open_config", OpenConfig),
        ("reload_config", ReloadConfig),
        ("toggle_sidebar", ToggleSidebar),
        ("edit_scrollback", EditScrollback),
    ];
    out.extend(
        plain
            .into_iter()
            .map(|(name, action)| (name.to_string(), NameSpec::Plain(action))),
    );
    out.push((
        "switch_workspace".into(),
        NameSpec::Indexed(SwitchWorkspace),
    ));
    out.push(("switch_tab".into(), NameSpec::Indexed(SwitchTab)));
    out
}

/// Methods the protocol doesn't have yet but the keymap already binds. Each is
/// planned for M1 or later; [`Action::target`] names them so the shell can
/// grey the action out until the core supports it.
pub const PLANNED_METHODS: &[&str] = &["tile.rename", "terminal.edit_scrollback"];

impl Action {
    /// The config-file name (`switch_tab` for `SwitchTab(3)`; see [`Action::index`]).
    pub fn name(self) -> String {
        names()
            .into_iter()
            .find(|(_, spec)| match spec {
                NameSpec::Plain(a) => *a == self,
                NameSpec::Indexed(make) => self.index().is_some_and(|i| make(i) == self),
            })
            .map(|(name, _)| name)
            .expect("every action has a name")
    }

    /// The action a config-file name stands for (`"split_right"`); indexed
    /// actions (`"switch_tab"`) need `index`. The inverse of [`Action::name`]
    /// and [`Action::index`], for menus and the palette.
    pub fn from_name(name: &str, index: Option<u8>) -> Option<Action> {
        names()
            .into_iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, spec)| match (spec, index) {
                (NameSpec::Plain(action), None) => Some(action),
                (NameSpec::Indexed(make), Some(i)) => Some(make(i)),
                _ => None,
            })
    }

    /// The index of an indexed action (`SwitchTab(3)` → 3).
    pub fn index(self) -> Option<u8> {
        match self {
            Action::SwitchWorkspace(i) | Action::SwitchTab(i) => Some(i),
            _ => None,
        }
    }

    pub fn target(self) -> Target {
        use Action::*;
        let method = match self {
            ResizeMode | CommandPalette | Goto | Find | OpenConfig | ReloadConfig
            | ToggleSidebar | EnterPrefix => return Target::Shell,
            FocusTile(_) => "layout.focus",
            SwapTile(_) => "layout.swap",
            MoveTile(_) => "layout.move",
            ResizeTile(_) => "layout.resize",
            ZoomTile => "layout.zoom",
            CloseTile | KillTile => "tile.close",
            CycleTileNext | CycleTilePrevious => "tile.focus",
            RenameTile => "tile.rename",
            Split(_) => "layout.split",
            SplitAuto => "layout.split_auto",
            PromoteTile => "layout.promote",
            RotateLayout => "layout.rotate",
            EqualizeLayout => "layout.equalize",
            NewWorkspace => "workspace.create",
            SwitchWorkspace(_) | LastWorkspace | NextWorkspace | PreviousWorkspace => {
                "workspace.focus"
            }
            CloseWorkspace => "workspace.close",
            RenameWorkspace => "workspace.rename",
            NewTab => "tab.create",
            SwitchTab(_) | LastTab | NextTab | PreviousTab => "tab.focus",
            CloseTab => "tab.close",
            RenameTab => "tab.rename",
            NewWindow => "window.create",
            CloseWindow => "window.close",
            EditScrollback => "terminal.edit_scrollback",
        };
        Target::Method(method)
    }

    /// The command for actions that act on whatever is focused and need no
    /// session state. `None` for shell-local actions and for ones the shell has
    /// to resolve (switch to tab N, renames, cycling).
    pub fn command(self) -> Option<Command> {
        use Action::*;
        Some(match self {
            FocusTile(direction) => Command::LayoutFocus(LayoutFocus { direction }),
            SwapTile(direction) => Command::LayoutSwap(LayoutSwap {
                direction: Some(direction),
                ..LayoutSwap::default()
            }),
            MoveTile(direction) => Command::LayoutMove(LayoutMove {
                tile: None,
                direction,
            }),
            ResizeTile(direction) => Command::LayoutResize(LayoutResize {
                tile: None,
                direction,
                amount: DEFAULT_RESIZE_STEP,
            }),
            ZoomTile => Command::LayoutZoom(LayoutZoom::default()),
            CloseTile => Command::TileClose(TileClose {
                tile: None,
                kill: false,
            }),
            KillTile => Command::TileClose(TileClose {
                tile: None,
                kill: true,
            }),
            Split(direction) => Command::LayoutSplit(LayoutSplit {
                tile: None,
                direction,
                content: None,
            }),
            SplitAuto => Command::LayoutSplitAuto(LayoutSplitAuto::default()),
            PromoteTile => Command::LayoutPromote(TileTarget::default()),
            RotateLayout => Command::LayoutRotate(TabTarget::default()),
            EqualizeLayout => Command::LayoutEqualize(TabTarget::default()),
            NewWorkspace => Command::WorkspaceCreate(WorkspaceCreate::default()),
            CloseWorkspace => Command::WorkspaceClose(WorkspaceTarget::default()),
            NewTab => Command::TabCreate(TabCreate::default()),
            CloseTab => Command::TabClose(TabTarget::default()),
            NewWindow => Command::WindowCreate(WindowCreate::default()),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use azulejo_protocol::METHODS;

    #[test]
    fn names_are_unique_and_round_trip() {
        let all = names();
        let mut seen = std::collections::HashSet::new();
        for (name, spec) in &all {
            assert!(seen.insert(name.clone()), "duplicate name {name}");
            let action = match spec {
                NameSpec::Plain(a) => *a,
                NameSpec::Indexed(make) => make(4),
            };
            assert_eq!(&action.name(), name);
        }
    }

    #[test]
    fn commands_match_targets() {
        for (_, spec) in names() {
            let action = match spec {
                NameSpec::Plain(a) => a,
                NameSpec::Indexed(make) => make(1),
            };
            if let Some(command) = action.command() {
                assert_eq!(action.target(), Target::Method(command.method()));
            }
        }
    }

    #[test]
    fn planned_methods_are_not_typos_of_real_ones() {
        // A planned method that already exists is fine (the protocol caught up);
        // one that differs from a real method only by case is a typo.
        for planned in PLANNED_METHODS {
            assert!(
                !METHODS
                    .iter()
                    .any(|m| m.eq_ignore_ascii_case(planned) && m != planned)
            );
        }
    }
}
