//! The command palette's entries (F5): every keymap action, plus the
//! user-facing protocol methods no key is bound to, each with its current
//! chords. `commands.list` serves this; shells show it.

use azulejo_protocol::{CommandEntry, METHODS};

use crate::action::{NameSpec, names};
use crate::{Action, Config, KeyContext, PLANNED_METHODS, Target};

/// How many entries an indexed action (`switch_tab`) gets: 1 to this.
pub const INDEXED_ENTRIES: u8 = 9;

/// User-facing methods that no keymap action sends: the palette lists them
/// on their own, usually with a prompt.
const METHOD_ENTRIES: &[(&str, &str, &str, Option<&str>, bool)] = &[
    // (method, title, category, prompt, needs_tile)
    (
        "browser.open",
        "Open URL in browser tile",
        "browser",
        Some("url"),
        true,
    ),
    (
        "preview.open",
        "Preview file in this tile",
        "preview",
        Some("path"),
        true,
    ),
    (
        "workspace.set_repo",
        "Set workspace repository folder…",
        "workspace",
        Some("path"),
        false,
    ),
    (
        "files.open",
        "Open path from file browser…",
        "files",
        Some("path"),
        true,
    ),
    (
        "files.reveal",
        "Reveal path in file browser…",
        "files",
        Some("path"),
        true,
    ),
    (
        "files.set_root",
        "Set file browser root…",
        "files",
        Some("root"),
        true,
    ),
    (
        "layout.apply",
        "Open layout file…",
        "layout",
        Some("path"),
        false,
    ),
    (
        "layout.export",
        "Save layout as…",
        "layout",
        Some("path"),
        false,
    ),
    (
        "layout.set",
        "Set layout (tile, monocle, three_column, manual)…",
        "layout",
        Some("kind"),
        false,
    ),
    (
        "terminal.respawn",
        "Respawn ended terminal",
        "terminal",
        None,
        true,
    ),
    (
        "workspace.move",
        "Move workspace to position…",
        "workspace",
        Some("index"),
        false,
    ),
    (
        "tab.move",
        "Move tab to position…",
        "tab",
        Some("index"),
        false,
    ),
];

/// Methods the palette never offers: plumbing for shells, the herdr adapter
/// and scripts, or covered by a shell-side action (search → Find).
pub const INTERNAL_METHODS: &[&str] = &[
    "core.ping",
    "session.get",
    "window.focus",
    "window.set_frame",
    "window.set_sidebar",
    "window.list",
    "workspace.list",
    "workspace.bind",
    "tab.list",
    "tile.list",
    "tile.get",
    // Shells send it when a focused tile's agent attention lights up.
    "tile.seen",
    "browser.state",
    "preview.get",
    // The files tile lists and watches folders as the user opens them.
    "files.list",
    "files.expand",
    "files.collapse",
    // The sidebar's Files view opens and closes its panel.
    "files.panel",
    "files.panel_close",
    "layout.get",
    "terminal.bind",
    "terminal.send_text",
    "terminal.send_keys",
    "terminal.read",
    "terminal.scroll",
    "terminal.search",
    "events.subscribe",
    "events.unsubscribe",
    "commands.list",
    // For `azj config keys` / `azj config defaults`; the palette shows chords
    // itself, and "Open config file" writes the template.
    "config.keys",
    "config.defaults",
    // The sidebar's machine groups read it; "+" on a group is workspace.create.
    "machines.list",
];

/// Every palette entry for `config`'s keymap, in a stable order: keymap
/// actions first (in config-name order), then method-only entries.
pub fn catalogue(config: &Config) -> Vec<CommandEntry> {
    let mut out = Vec::new();
    for (name, spec) in names() {
        match spec {
            NameSpec::Plain(action) => out.push(action_entry(config, &name, action, None)),
            NameSpec::Indexed(make) => {
                for i in 1..=INDEXED_ENTRIES {
                    out.push(action_entry(config, &name, make(i), Some(i)));
                }
            }
        }
    }
    for (method, title, category, prompt, needs_tile) in METHOD_ENTRIES {
        out.push(CommandEntry {
            id: (*method).into(),
            title: (*title).into(),
            category: (*category).into(),
            method: Some((*method).into()),
            action: None,
            index: None,
            chords: Vec::new(),
            needs_tile: *needs_tile,
            prompt: prompt.map(str::to_owned),
            planned: !METHODS.contains(method),
            context: None,
        });
    }
    out
}

fn action_entry(config: &Config, name: &str, action: Action, index: Option<u8>) -> CommandEntry {
    let keymap = &config.keymap;
    let platform = keymap.platform();
    let mut chords: Vec<String> = Vec::new();
    let mut push = |c: String| {
        if !chords.contains(&c) {
            chords.push(c);
        }
    };
    for context in std::iter::once(None).chain(KeyContext::ALL.into_iter().map(Some)) {
        for chord in keymap.chords_for(action, context) {
            push(chord.display(platform));
        }
    }
    if keymap.prefix_enabled()
        && let Some(prefix) = keymap.prefix_key()
    {
        for chord in keymap.prefix_chords_for(action) {
            push(format!(
                "{} {}",
                prefix.display(platform),
                chord.display(platform)
            ));
        }
    }
    let method = match action.target() {
        Target::Shell => None,
        Target::Method(m) => Some(m.to_owned()),
    };
    let planned = method
        .as_deref()
        .is_some_and(|m| PLANNED_METHODS.contains(&m) && !METHODS.contains(&m));
    CommandEntry {
        id: match index {
            Some(i) => format!("{name}_{i}"),
            None => name.to_owned(),
        },
        title: title(action),
        category: category(action).into(),
        method,
        action: Some(name.to_owned()),
        index,
        chords,
        needs_tile: needs_tile(action),
        prompt: prompt(action).map(str::to_owned),
        planned,
        context: action.context().map(|c| c.name().to_owned()),
    }
}

fn dir(d: azulejo_protocol::Direction) -> &'static str {
    use azulejo_protocol::Direction::*;
    match d {
        Left => "left",
        Right => "right",
        Up => "up",
        Down => "down",
    }
}

/// What an action does, for listings: its palette title, with `N` for
/// indexed actions.
pub(crate) fn describe(name: &str, spec: &NameSpec) -> String {
    match spec {
        NameSpec::Plain(action) => title(*action),
        NameSpec::Indexed(_) => match name {
            "switch_workspace" => "Switch to workspace N".into(),
            "switch_tab" => "Switch to tab N".into(),
            other => other.to_owned(),
        },
    }
}

pub(crate) fn title(action: Action) -> String {
    use Action::*;
    match action {
        FocusTile(d) => format!("Focus tile {}", dir(d)),
        SwapTile(d) => format!("Swap tile {}", dir(d)),
        MoveTile(d) => format!("Move tile {}", dir(d)),
        ResizeTile(d) => format!("Resize tile {}", dir(d)),
        ResizeMode => "Resize mode".into(),
        ZoomTile => "Zoom tile".into(),
        CloseTile => "Close tile (detach)".into(),
        KillTile => "Kill tile and its process".into(),
        CycleTileNext => "Next tile".into(),
        CycleTilePrevious => "Previous tile".into(),
        RenameTile => "Rename tile…".into(),
        Split(d) => format!("Split {}", dir(d)),
        SplitAuto => "New tile in stack (tall)".into(),
        PromoteTile => "Promote tile to master".into(),
        RotateLayout => "Rotate layout".into(),
        EqualizeLayout => "Equalize stack".into(),
        CycleLayoutNext => "Next layout".into(),
        CycleLayoutPrevious => "Previous layout".into(),
        ToggleLayout(kind) => match kind {
            azulejo_protocol::LayoutKind::Tile => "Toggle Tile layout".into(),
            azulejo_protocol::LayoutKind::Monocle => "Toggle Monocle layout".into(),
            azulejo_protocol::LayoutKind::ThreeColumn => "Toggle Three Column layout".into(),
            azulejo_protocol::LayoutKind::Manual => "Toggle manual layout (freeze)".into(),
        },
        IncreaseMasters => "More masters".into(),
        DecreaseMasters => "Fewer masters".into(),
        GrowMasterArea => "Grow master area".into(),
        ShrinkMasterArea => "Shrink master area".into(),
        MovePositionNext => "Move tile to next position".into(),
        MovePositionPrevious => "Move tile to previous position".into(),
        RotatePart => "Rotate master area".into(),
        RotateClockwise => "Rotate layout clockwise".into(),
        RotateAnticlockwise => "Rotate layout anticlockwise".into(),
        NewWorkspace => "New workspace".into(),
        SwitchWorkspace(i) => format!("Switch to workspace {i}"),
        SwitchWorkspaceLast => "Switch to last workspace".into(),
        LastWorkspace => "Previous active workspace".into(),
        NextWorkspace => "Next workspace".into(),
        PreviousWorkspace => "Previous workspace".into(),
        CloseWorkspace => "Close workspace (detach)".into(),
        RenameWorkspace => "Rename workspace…".into(),
        NewTab => "New tab".into(),
        SwitchTab(i) => format!("Switch to tab {i}"),
        SwitchTabLast => "Switch to last tab".into(),
        LastTab => "Previous active tab".into(),
        NextTab => "Next tab".into(),
        PreviousTab => "Previous tab".into(),
        CloseTab => "Close tab".into(),
        RenameTab => "Rename tab…".into(),
        NewWindow => "New window".into(),
        CloseWindow => "Close window".into(),
        QuickOpen => "Quick open".into(),
        CommandPalette => "Commands…".into(),
        Goto => "Go to…".into(),
        Find => "Find in terminal…".into(),
        OpenConfig => "Open config file".into(),
        ReloadConfig => "Reload config".into(),
        ToggleSidebar => "Toggle sidebar".into(),
        SetTheme => "Set theme…".into(),
        EditScrollback => "Edit scrollback".into(),
        EnterPrefix => "Prefix".into(),
        Quit => "Quit Azulejo".into(),
        Hide => "Hide Azulejo".into(),
        HideOthers => "Hide other apps".into(),
        Minimize => "Minimise window".into(),
        SendHerdrPrefix => "Type herdr's prefix (ctrl+b) into the pane".into(),
        TakeOver => "Take over or give back a terminal".into(),
        PaletteClose => "Quick open: close".into(),
        PaletteRun => "Quick open: run the selected row".into(),
        PaletteNext => "Quick open: next row".into(),
        PalettePrevious => "Quick open: previous row".into(),
        PalettePageDown => "Quick open: page down".into(),
        PalettePageUp => "Quick open: page up".into(),
        PaletteComplete => "Quick open: complete the query".into(),
        FindNext => "Find: next match".into(),
        FindPrevious => "Find: previous match".into(),
        FindClose => "Find: close the bar".into(),
        ResizeModeExit => "Resize mode: done".into(),
        BrowserFocusAddress => "Browser: edit the address".into(),
        BrowserReload => "Browser: reload".into(),
        BrowserReloadFromOrigin => "Browser: reload from origin".into(),
        BrowserBack => "Browser: back".into(),
        BrowserForward => "Browser: forward".into(),
        BrowserStop => "Browser: stop loading".into(),
        BrowserZoomIn => "Browser: zoom in".into(),
        BrowserZoomOut => "Browser: zoom out".into(),
        BrowserZoomReset => "Browser: actual size".into(),
        BrowserInspector => "Browser: Web Inspector".into(),
        PreviewToggleSource => "Preview: toggle Preview | Source".into(),
        FilesDown => "Files: next row".into(),
        FilesUp => "Files: previous row".into(),
        FilesOpen => "Files: open".into(),
        FilesExpand => "Files: expand folder or open file".into(),
        FilesCollapse => "Files: collapse folder or go to parent".into(),
        FilesOpenExternal => "Files: open in default app".into(),
        FilesParent => "Files: go to parent folder".into(),
        FilesQuickLook => "Files: Quick Look".into(),
        FilesFilter => "Files: filter".into(),
        FilesToggleHidden => "Files: show or hide hidden files".into(),
        FilesLeave => "Files: back to the tiles (sidebar)".into(),
    }
}

fn category(action: Action) -> &'static str {
    use Action::*;
    match action {
        FocusTile(_) | SwapTile(_) | MoveTile(_) | ResizeTile(_) | ResizeMode | ZoomTile
        | CloseTile | KillTile | CycleTileNext | CycleTilePrevious | RenameTile => "tile",
        Split(_) | SplitAuto | PromoteTile | RotateLayout | EqualizeLayout | CycleLayoutNext
        | CycleLayoutPrevious | ToggleLayout(_) | IncreaseMasters | DecreaseMasters
        | GrowMasterArea | ShrinkMasterArea | MovePositionNext | MovePositionPrevious
        | RotatePart | RotateClockwise | RotateAnticlockwise => "layout",
        NewWorkspace | SwitchWorkspace(_) | SwitchWorkspaceLast | LastWorkspace | NextWorkspace
        | PreviousWorkspace | CloseWorkspace | RenameWorkspace => "workspace",
        NewTab | SwitchTab(_) | SwitchTabLast | LastTab | NextTab | PreviousTab | CloseTab
        | RenameTab => "tab",
        NewWindow | CloseWindow | Minimize => "window",
        Find | EditScrollback | SendHerdrPrefix | TakeOver => "terminal",
        QuickOpen | CommandPalette | Goto | OpenConfig | ReloadConfig | ToggleSidebar
        | SetTheme | EnterPrefix | Quit | Hide | HideOthers => "app",
        PaletteClose | PaletteRun | PaletteNext | PalettePrevious | PalettePageDown
        | PalettePageUp | PaletteComplete => "palette",
        FindNext | FindPrevious | FindClose => "find",
        ResizeModeExit => "tile",
        BrowserFocusAddress
        | BrowserReload
        | BrowserReloadFromOrigin
        | BrowserBack
        | BrowserForward
        | BrowserStop
        | BrowserZoomIn
        | BrowserZoomOut
        | BrowserZoomReset
        | BrowserInspector => "browser",
        PreviewToggleSource => "preview",
        FilesDown | FilesUp | FilesOpen | FilesExpand | FilesCollapse | FilesOpenExternal
        | FilesParent | FilesQuickLook | FilesFilter | FilesToggleHidden | FilesLeave => "files",
    }
}

fn needs_tile(action: Action) -> bool {
    use Action::*;
    matches!(
        action,
        FocusTile(_)
            | SwapTile(_)
            | MoveTile(_)
            | ResizeTile(_)
            | ResizeMode
            | ZoomTile
            | CloseTile
            | KillTile
            | RenameTile
            | Split(_)
            | PromoteTile
            | MovePositionNext
            | MovePositionPrevious
            | Find
            | EditScrollback
            | SendHerdrPrefix
            | TakeOver
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
            | PreviewToggleSource
            | FilesDown
            | FilesUp
            | FilesOpen
            | FilesExpand
            | FilesCollapse
            | FilesOpenExternal
            | FilesParent
            | FilesQuickLook
            | FilesFilter
            | FilesToggleHidden
    )
}

fn prompt(action: Action) -> Option<&'static str> {
    use Action::*;
    match action {
        RenameTile | RenameTab | RenameWorkspace => Some("name"),
        Find => Some("query"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Platform;
    use std::collections::HashSet;

    fn entries(platform: Platform) -> Vec<CommandEntry> {
        catalogue(&Config::defaults(platform))
    }

    #[test]
    fn every_keymap_action_is_listed() {
        let list = entries(Platform::Macos);
        for (name, spec) in names() {
            match spec {
                NameSpec::Plain(_) => assert!(
                    list.iter()
                        .any(|e| e.action.as_deref() == Some(name.as_str())),
                    "{name} missing"
                ),
                NameSpec::Indexed(_) => {
                    for i in 1..=INDEXED_ENTRIES {
                        let id = format!("{name}_{i}");
                        assert!(list.iter().any(|e| e.id == id), "{id} missing");
                    }
                }
            }
        }
        let ids: HashSet<_> = list.iter().map(|e| e.id.clone()).collect();
        assert_eq!(ids.len(), list.len(), "ids are unique");
    }

    #[test]
    fn every_method_is_offered_or_classified_internal() {
        let list = entries(Platform::Macos);
        let offered: HashSet<&str> = list.iter().filter_map(|e| e.method.as_deref()).collect();
        for method in METHODS {
            let internal = INTERNAL_METHODS.contains(method);
            assert!(
                offered.contains(method) != internal,
                "`{method}` must be either in the palette or in INTERNAL_METHODS \
                 (config/src/catalogue.rs), not {}",
                if internal { "both" } else { "neither" }
            );
        }
        for (method, ..) in METHOD_ENTRIES {
            assert!(METHODS.contains(method), "{method} isn't a protocol method");
        }
    }

    #[test]
    fn chords_follow_the_keymap_and_platform() {
        let mac = entries(Platform::Macos);
        let linux = entries(Platform::Linux);
        let find =
            |list: &[CommandEntry], id: &str| list.iter().find(|e| e.id == id).cloned().unwrap();
        // Something is bound on each platform, spelled for that platform.
        let mac_bound: Vec<_> = mac.iter().flat_map(|e| e.chords.clone()).collect();
        assert!(
            mac_bound.iter().any(|c| c.contains("cmd+")),
            "{mac_bound:?}"
        );
        assert!(mac_bound.iter().all(|c| !c.contains("super+")));
        let linux_bound: Vec<_> = linux.iter().flat_map(|e| e.chords.clone()).collect();
        assert!(
            linux_bound.iter().all(|c| !c.contains("cmd+")),
            "{linux_bound:?}"
        );
        // The terminal-only cmd+a sidebar binding still shows up.
        let sidebar = find(&mac, "toggle_sidebar");
        assert!(sidebar.chords.contains(&"cmd+a".to_owned()), "{sidebar:?}");
        assert_eq!(sidebar.method, None, "shell-only");

        // A user override changes the listed chord.
        let config = Config::from_toml(
            "[keys]\nsplit_auto = \"ctrl+alt+n\"\n",
            None,
            Platform::Macos,
        );
        let split = catalogue(&config)
            .into_iter()
            .find(|e| e.id == "split_auto")
            .unwrap();
        assert_eq!(split.chords, ["ctrl+alt+n"]);
        assert_eq!(split.method.as_deref(), Some("layout.split_auto"));
    }

    #[test]
    fn entries_carry_what_the_palette_needs() {
        let list = entries(Platform::Macos);
        let get = |id: &str| list.iter().find(|e| e.id == id).unwrap();
        assert!(get("close_tile").needs_tile);
        assert!(!get("new_tab").needs_tile);
        assert_eq!(get("switch_tab_3").index, Some(3));
        assert_eq!(get("switch_tab_3").title, "Switch to tab 3");
        assert_eq!(get("rename_tab").prompt.as_deref(), Some("name"));
        assert_eq!(get("layout.apply").prompt.as_deref(), Some("path"));
        assert!(get("edit_scrollback").planned);
        assert!(!get("rename_tile").planned, "tile.rename exists now");
        assert_eq!(get("quick_open").chords, ["cmd+k"]);
        assert_eq!(get("quick_open").category, "app");
        assert!(get("command_palette").chords.is_empty());
        assert!(get("goto").chords.is_empty());
        // Themes go through the catalogue, with no default chord.
        assert!(get("set_theme").chords.is_empty());
        assert_eq!(get("set_theme").title, "Set theme…");
        assert_eq!(get("set_theme").method, None, "shell-only");
        for e in &list {
            assert!(!e.title.is_empty() && !e.category.is_empty(), "{e:?}");
        }
    }

    /// ⌘9 and ⌃9 go to the last workspace and tab in order, as in a browser;
    /// the previously active ones keep alt+0 (and the palette). ⌘W closes
    /// the workspace and ⌘⇧W the tab, as ⌘T and ⌘⇧T open them.
    #[test]
    fn last_and_close_chords_follow_the_browser() {
        for (platform, cmd) in [(Platform::Macos, "cmd"), (Platform::Linux, "super")] {
            let list = entries(platform);
            let get = |id: &str| list.iter().find(|e| e.id == id).unwrap().clone();
            let ws_last = get("switch_workspace_last");
            assert_eq!(ws_last.chords, [format!("{cmd}+9")], "{platform:?}");
            assert_eq!(ws_last.title, "Switch to last workspace");
            assert_eq!(ws_last.method.as_deref(), Some("workspace.focus"));
            let previous = get("last_workspace");
            assert_eq!(previous.chords, ["alt+0"], "{platform:?}");
            assert_eq!(previous.title, "Previous active workspace");
            let tab_last = get("switch_tab_last");
            assert_eq!(tab_last.chords, ["ctrl+9"], "{platform:?}");
            assert_eq!(tab_last.title, "Switch to last tab");
            let previous_tab = get("last_tab");
            assert!(previous_tab.chords.is_empty(), "{previous_tab:?}");
            assert_eq!(previous_tab.title, "Previous active tab");
            assert_eq!(get("close_workspace").chords, [format!("{cmd}+w")]);
            assert_eq!(
                get("close_workspace").method.as_deref(),
                Some("workspace.close")
            );
            assert_eq!(get("close_tab").chords, [format!("{cmd}+shift+w")]);
            // Indexed switches stop at 8: 9 is "last".
            assert!(get("switch_workspace_9").chords.is_empty());
            assert!(get("switch_tab_9").chords.is_empty());
        }
    }
}
