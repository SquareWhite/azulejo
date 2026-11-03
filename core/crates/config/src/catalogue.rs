//! The command palette's entries (F5): every keymap action, plus the
//! user-facing protocol methods no key is bound to, each with its current
//! chords. `commands.list` serves this; shells show it.

use azulejo_protocol::{CommandEntry, METHODS, TileKind};

use crate::action::{NameSpec, names};
use crate::{Action, Config, PLANNED_METHODS, Target};

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
        "files.toggle_hidden",
        "Show or hide hidden files",
        "files",
        None,
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
    for tile in [
        None,
        Some(TileKind::Terminal),
        Some(TileKind::Browser),
        Some(TileKind::Preview),
        Some(TileKind::Files),
    ] {
        for chord in keymap.chords_for(action, tile) {
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

fn title(action: Action) -> String {
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
        LastWorkspace => "Last workspace".into(),
        NextWorkspace => "Next workspace".into(),
        PreviousWorkspace => "Previous workspace".into(),
        CloseWorkspace => "Close workspace".into(),
        RenameWorkspace => "Rename workspace…".into(),
        NewTab => "New tab".into(),
        SwitchTab(i) => format!("Switch to tab {i}"),
        LastTab => "Last tab".into(),
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
        EditScrollback => "Edit scrollback".into(),
        EnterPrefix => "Prefix".into(),
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
        NewWorkspace | SwitchWorkspace(_) | LastWorkspace | NextWorkspace | PreviousWorkspace
        | CloseWorkspace | RenameWorkspace => "workspace",
        NewTab | SwitchTab(_) | LastTab | NextTab | PreviousTab | CloseTab | RenameTab => "tab",
        NewWindow | CloseWindow => "window",
        Find | EditScrollback => "terminal",
        QuickOpen | CommandPalette | Goto | OpenConfig | ReloadConfig | ToggleSidebar
        | EnterPrefix => "app",
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
        for e in &list {
            assert!(!e.title.is_empty() && !e.category.is_empty(), "{e:?}");
        }
    }
}
