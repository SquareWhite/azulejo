//! Keymap actions and what each one turns into.
//!
//! An action either becomes a protocol command (the shell sends it to the core
//! like any API client would) or stays in the shell (palette, sidebar, prefix
//! mode). Actions that need session state to pick a target, such as "switch to
//! tab 3", name their method but leave building the command to the shell.

use crate::keymap::KeyContext;
use azulejo_protocol::{
    Command, DEFAULT_MASTER_RATIO_STEP, DEFAULT_RESIZE_STEP, Direction, LayoutCycle, LayoutFocus,
    LayoutKind, LayoutMasterRatio, LayoutMasters, LayoutMove, LayoutMoveOrder, LayoutResize,
    LayoutRotate, LayoutSetKind, LayoutSplit, LayoutSplitAuto, LayoutSwap, LayoutZoom, TabCreate,
    TabTarget, TileClose, TileTarget, WindowCreate, WorkspaceCreate, WorkspaceTarget,
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
    /// A quarter turn clockwise: Bismuth's rotate in a Tile tab, the whole
    /// split tree in a manual tab. Monocle and Three Column: nothing.
    RotateLayout,
    EqualizeLayout,
    // Dynamic layouts (ADR 0010)
    CycleLayoutNext,
    CycleLayoutPrevious,
    /// Switches to the kind, or back to the previous one if already on it.
    ToggleLayout(LayoutKind),
    IncreaseMasters,
    DecreaseMasters,
    GrowMasterArea,
    ShrinkMasterArea,
    MovePositionNext,
    MovePositionPrevious,
    /// Turns only the master area (masters side by side or stacked).
    RotatePart,
    RotateAnticlockwise,
    /// Master left ↔ top: `tall.py`'s rotate, `alt+o` before it turned
    /// four ways.
    FlipLayout,
    // Workspaces
    NewWorkspace,
    /// 1-based position in the window.
    SwitchWorkspace(u8),
    /// The last workspace in the window's order (browser-style ⌘9).
    SwitchWorkspaceLast,
    /// The workspace that was active before the current one.
    LastWorkspace,
    NextWorkspace,
    PreviousWorkspace,
    CloseWorkspace,
    RenameWorkspace,
    // Tabs
    NewTab,
    /// 1-based position in the workspace.
    SwitchTab(u8),
    /// The last tab in the workspace's order (browser-style).
    SwitchTabLast,
    /// The tab that was active before the current one.
    LastTab,
    NextTab,
    PreviousTab,
    CloseTab,
    RenameTab,
    // Windows
    NewWindow,
    CloseWindow,
    // App
    /// The ⌘K menu: commands and jump targets in one list.
    QuickOpen,
    /// The same menu limited to commands (opens with `>`).
    CommandPalette,
    /// The same menu limited to workspaces, tabs and tiles (opens with `@`).
    Goto,
    Find,
    OpenConfig,
    ReloadConfig,
    ToggleSidebar,
    /// Lists the themes in the ⌘K menu, previewing each as it's selected.
    SetTheme,
    EditScrollback,
    /// Arms prefix mode; the next chord is looked up in the prefix keymap.
    EnterPrefix,
    // App menu (macOS conventions; menus show these chords)
    Quit,
    Hide,
    HideOthers,
    Minimize,
    // Terminal tiles
    /// Types the chord into the pane twice, so herdr's attach client passes
    /// one on and never starts its detach sequence (`ctrl+b q`).
    SendHerdrPrefix,
    /// Take over a terminal another client holds (read-only view), or give
    /// it back.
    TakeOver,
    // Quick open (the ⌘K menu)
    PaletteClose,
    PaletteRun,
    PaletteNext,
    PalettePrevious,
    PalettePageDown,
    PalettePageUp,
    /// Completes the field with the selected row's title.
    PaletteComplete,
    // Find bars (and the files filter)
    FindNext,
    FindPrevious,
    FindClose,
    // Resize mode
    ResizeModeExit,
    // Browser tiles
    BrowserFocusAddress,
    BrowserReload,
    BrowserReloadFromOrigin,
    BrowserBack,
    BrowserForward,
    BrowserStop,
    BrowserZoomIn,
    BrowserZoomOut,
    BrowserZoomReset,
    BrowserInspector,
    // Preview tiles
    PreviewToggleSource,
    // Files tiles and the sidebar's Files view
    FilesDown,
    FilesUp,
    /// Opens a file (preview) or a folder, like a double-click.
    FilesOpen,
    /// Expands a folder, or opens a file.
    FilesExpand,
    /// Collapses a folder, or goes to the parent.
    FilesCollapse,
    /// Opens in the default app (expands a folder).
    FilesOpenExternal,
    FilesParent,
    FilesQuickLook,
    FilesFilter,
    FilesToggleHidden,
    /// The sidebar's Files view hands the keyboard back to the tiles.
    FilesLeave,
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
        ("cycle_layout_next", CycleLayoutNext),
        ("cycle_layout_previous", CycleLayoutPrevious),
        ("toggle_layout_tile", ToggleLayout(LayoutKind::Tile)),
        ("toggle_layout_monocle", ToggleLayout(LayoutKind::Monocle)),
        (
            "toggle_layout_three_column",
            ToggleLayout(LayoutKind::ThreeColumn),
        ),
        ("toggle_layout_manual", ToggleLayout(LayoutKind::Manual)),
        ("increase_masters", IncreaseMasters),
        ("decrease_masters", DecreaseMasters),
        ("grow_master_area", GrowMasterArea),
        ("shrink_master_area", ShrinkMasterArea),
        ("move_position_next", MovePositionNext),
        ("move_position_previous", MovePositionPrevious),
        ("rotate_part", RotatePart),
        ("rotate_anticlockwise", RotateAnticlockwise),
        ("flip_layout", FlipLayout),
        ("new_workspace", NewWorkspace),
        ("switch_workspace_last", SwitchWorkspaceLast),
        ("last_workspace", LastWorkspace),
        ("next_workspace", NextWorkspace),
        ("previous_workspace", PreviousWorkspace),
        ("close_workspace", CloseWorkspace),
        ("rename_workspace", RenameWorkspace),
        ("new_tab", NewTab),
        ("switch_tab_last", SwitchTabLast),
        ("last_tab", LastTab),
        ("next_tab", NextTab),
        ("previous_tab", PreviousTab),
        ("close_tab", CloseTab),
        ("rename_tab", RenameTab),
        ("new_window", NewWindow),
        ("close_window", CloseWindow),
        ("quick_open", QuickOpen),
        ("command_palette", CommandPalette),
        ("goto", Goto),
        ("find", Find),
        ("open_config", OpenConfig),
        ("reload_config", ReloadConfig),
        ("toggle_sidebar", ToggleSidebar),
        ("set_theme", SetTheme),
        ("edit_scrollback", EditScrollback),
        ("quit", Quit),
        ("hide", Hide),
        ("hide_others", HideOthers),
        ("minimize", Minimize),
        ("send_herdr_prefix", SendHerdrPrefix),
        ("take_over", TakeOver),
        ("palette_close", PaletteClose),
        ("palette_run", PaletteRun),
        ("palette_next", PaletteNext),
        ("palette_previous", PalettePrevious),
        ("palette_page_down", PalettePageDown),
        ("palette_page_up", PalettePageUp),
        ("palette_complete", PaletteComplete),
        ("find_next", FindNext),
        ("find_previous", FindPrevious),
        ("find_close", FindClose),
        ("resize_mode_exit", ResizeModeExit),
        ("browser_focus_address", BrowserFocusAddress),
        ("browser_reload", BrowserReload),
        ("browser_reload_from_origin", BrowserReloadFromOrigin),
        ("browser_back", BrowserBack),
        ("browser_forward", BrowserForward),
        ("browser_stop", BrowserStop),
        ("browser_zoom_in", BrowserZoomIn),
        ("browser_zoom_out", BrowserZoomOut),
        ("browser_zoom_reset", BrowserZoomReset),
        ("browser_inspector", BrowserInspector),
        ("preview_toggle_source", PreviewToggleSource),
        ("files_down", FilesDown),
        ("files_up", FilesUp),
        ("files_open", FilesOpen),
        ("files_expand", FilesExpand),
        ("files_collapse", FilesCollapse),
        ("files_open_external", FilesOpenExternal),
        ("files_parent", FilesParent),
        ("files_quick_look", FilesQuickLook),
        ("files_filter", FilesFilter),
        ("files_toggle_hidden", FilesToggleHidden),
        ("files_leave", FilesLeave),
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

    /// Where the action works. `None`: anywhere (global sections, or any
    /// context section to override it there). `Some`: only in that context,
    /// so a global section binds it there.
    pub fn context(self) -> Option<KeyContext> {
        use Action::*;
        Some(match self {
            SendHerdrPrefix | TakeOver => KeyContext::Terminal,
            PaletteClose | PaletteRun | PaletteNext | PalettePrevious | PalettePageDown
            | PalettePageUp | PaletteComplete => KeyContext::Palette,
            FindNext | FindPrevious | FindClose => KeyContext::Find,
            ResizeModeExit => KeyContext::Resize,
            BrowserFocusAddress
            | BrowserReload
            | BrowserReloadFromOrigin
            | BrowserBack
            | BrowserForward
            | BrowserStop
            | BrowserZoomIn
            | BrowserZoomOut
            | BrowserZoomReset
            | BrowserInspector => KeyContext::Browser,
            PreviewToggleSource => KeyContext::Preview,
            FilesDown | FilesUp | FilesOpen | FilesExpand | FilesCollapse | FilesOpenExternal
            | FilesParent | FilesQuickLook | FilesFilter | FilesToggleHidden | FilesLeave => {
                KeyContext::Files
            }
            _ => return None,
        })
    }

    pub fn target(self) -> Target {
        use Action::*;
        let method = match self {
            ResizeMode
            | QuickOpen
            | CommandPalette
            | Goto
            | Find
            | OpenConfig
            | ReloadConfig
            | ToggleSidebar
            | SetTheme
            | EnterPrefix
            | Quit
            | Hide
            | HideOthers
            | Minimize
            | SendHerdrPrefix
            | TakeOver
            | PaletteClose
            | PaletteRun
            | PaletteNext
            | PalettePrevious
            | PalettePageDown
            | PalettePageUp
            | PaletteComplete
            | FindNext
            | FindPrevious
            | FindClose
            | ResizeModeExit
            | BrowserFocusAddress
            | BrowserReload
            | BrowserReloadFromOrigin
            | BrowserBack
            | BrowserForward
            | BrowserStop
            | BrowserZoomIn
            | BrowserZoomOut
            | BrowserZoomReset
            | BrowserInspector
            | FilesDown
            | FilesUp
            | FilesOpen
            | FilesExpand
            | FilesCollapse
            | FilesOpenExternal
            | FilesParent
            | FilesQuickLook
            | FilesFilter
            | FilesLeave => return Target::Shell,
            // The shell sends these for the tile (or sidebar panel) that has
            // the keyboard, so they carry no command of their own.
            PreviewToggleSource => "preview.set_mode",
            FilesToggleHidden => "files.toggle_hidden",
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
            RotateLayout | RotatePart | RotateAnticlockwise | FlipLayout => "layout.rotate",
            EqualizeLayout => "layout.equalize",
            CycleLayoutNext | CycleLayoutPrevious => "layout.cycle",
            ToggleLayout(_) => "layout.toggle",
            IncreaseMasters | DecreaseMasters => "layout.masters",
            GrowMasterArea | ShrinkMasterArea => "layout.master_ratio",
            MovePositionNext | MovePositionPrevious => "layout.move_order",
            NewWorkspace => "workspace.create",
            SwitchWorkspace(_) | SwitchWorkspaceLast | LastWorkspace | NextWorkspace
            | PreviousWorkspace => "workspace.focus",
            CloseWorkspace => "workspace.close",
            RenameWorkspace => "workspace.rename",
            NewTab => "tab.create",
            SwitchTab(_) | SwitchTabLast | LastTab | NextTab | PreviousTab => "tab.focus",
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
            RotateLayout => Command::LayoutRotate(LayoutRotate::default()),
            RotatePart => Command::LayoutRotate(LayoutRotate {
                part: true,
                ..LayoutRotate::default()
            }),
            RotateAnticlockwise => Command::LayoutRotate(LayoutRotate {
                reverse: true,
                ..LayoutRotate::default()
            }),
            FlipLayout => Command::LayoutRotate(LayoutRotate {
                flip: true,
                ..LayoutRotate::default()
            }),
            EqualizeLayout => Command::LayoutEqualize(TabTarget::default()),
            CycleLayoutNext => Command::LayoutCycle(LayoutCycle { tab: None, step: 1 }),
            CycleLayoutPrevious => Command::LayoutCycle(LayoutCycle {
                tab: None,
                step: -1,
            }),
            ToggleLayout(kind) => Command::LayoutToggle(LayoutSetKind { tab: None, kind }),
            IncreaseMasters => Command::LayoutMasters(LayoutMasters {
                tab: None,
                delta: 1,
            }),
            DecreaseMasters => Command::LayoutMasters(LayoutMasters {
                tab: None,
                delta: -1,
            }),
            GrowMasterArea => Command::LayoutMasterRatio(LayoutMasterRatio {
                tab: None,
                delta: DEFAULT_MASTER_RATIO_STEP,
            }),
            ShrinkMasterArea => Command::LayoutMasterRatio(LayoutMasterRatio {
                tab: None,
                delta: -DEFAULT_MASTER_RATIO_STEP,
            }),
            MovePositionNext => Command::LayoutMoveOrder(LayoutMoveOrder {
                tile: None,
                step: 1,
            }),
            MovePositionPrevious => Command::LayoutMoveOrder(LayoutMoveOrder {
                tile: None,
                step: -1,
            }),
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
