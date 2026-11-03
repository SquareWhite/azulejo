//! Turns a before/after pair of sessions into events.
//!
//! Order: created (outside in), updated, reordered, layout, closed (inside
//! out), then focus. Listeners can rely on a parent existing before its
//! children are announced, and on children closing before their parent.

use std::collections::HashMap;

use azulejo_model::{Session, Tab, TabId, Tile, TileId, WindowId, WorkspaceId};
use azulejo_protocol::{Event, LayoutState, TabInfo, WorkspaceInfo};

struct Index<'a> {
    windows: Vec<(WindowId, &'a azulejo_model::Window)>,
    workspaces: Vec<(WorkspaceId, WindowId, WorkspaceInfo)>,
    tabs: Vec<(TabId, WindowId, WorkspaceId, &'a Tab)>,
    tiles: Vec<(TileId, TabId, &'a Tile)>,
}

impl<'a> Index<'a> {
    fn new(s: &'a Session) -> Self {
        let mut index = Index {
            windows: Vec::new(),
            workspaces: Vec::new(),
            tabs: Vec::new(),
            tiles: Vec::new(),
        };
        for window in &s.windows {
            index.windows.push((window.id, window));
            for ws in &window.workspaces {
                index
                    .workspaces
                    .push((ws.id, window.id, workspace_info(ws)));
                for tab in &ws.tabs {
                    index.tabs.push((tab.id, window.id, ws.id, tab));
                    for tile in &tab.tiles {
                        index.tiles.push((tile.id, tab.id, tile));
                    }
                }
            }
        }
        index
    }
}

pub(crate) fn workspace_info(ws: &azulejo_model::Workspace) -> WorkspaceInfo {
    WorkspaceInfo {
        id: ws.id,
        name: ws.name.clone(),
        cwd: ws.cwd.clone(),
        herdr_workspace: ws.herdr_workspace.clone(),
    }
}

pub(crate) fn tab_info(tab: &Tab) -> TabInfo {
    TabInfo {
        id: tab.id,
        name: tab.name.clone(),
    }
}

/// Events describing how `before` became `after`. `kill` marks closed tiles
/// whose herdr panes should be killed rather than detached.
pub fn diff(before: &Session, after: &Session, kill: bool) -> Vec<Event> {
    let b = Index::new(before);
    let a = Index::new(after);
    let b_windows: HashMap<_, _> = b.windows.iter().map(|(id, w)| (*id, *w)).collect();
    let b_workspaces: HashMap<_, _> = b
        .workspaces
        .iter()
        .map(|(id, w, i)| (*id, (*w, i)))
        .collect();
    let b_tabs: HashMap<_, _> = b.tabs.iter().map(|(id, _, _, t)| (*id, *t)).collect();
    let b_tiles: HashMap<_, _> = b.tiles.iter().map(|(id, _, t)| (*id, *t)).collect();
    let a_windows: HashMap<_, _> = a.windows.iter().map(|(id, w)| (*id, *w)).collect();
    let a_workspaces: HashMap<_, _> = a.workspaces.iter().map(|(id, _, _)| (*id, ())).collect();
    let a_tabs: HashMap<_, _> = a.tabs.iter().map(|(id, _, _, _)| (*id, ())).collect();
    let a_tiles: HashMap<_, _> = a.tiles.iter().map(|(id, _, _)| (*id, ())).collect();

    let mut events = Vec::new();

    // Created, outside in.
    for (id, _) in &a.windows {
        if !b_windows.contains_key(id) {
            events.push(Event::WindowCreated { window: *id });
        }
    }
    for (id, window, info) in &a.workspaces {
        if !b_workspaces.contains_key(id) {
            events.push(Event::WorkspaceCreated {
                window: *window,
                workspace: info.clone(),
            });
        }
    }
    for (id, _, workspace, tab) in &a.tabs {
        if !b_tabs.contains_key(id) {
            events.push(Event::TabCreated {
                workspace: *workspace,
                tab: tab_info(tab),
            });
        }
    }
    for (id, tab, tile) in &a.tiles {
        if !b_tiles.contains_key(id) {
            events.push(Event::TileCreated {
                tab: *tab,
                tile: (*tile).clone(),
            });
        }
    }

    // Updated.
    for (id, window) in &a.windows {
        if let Some(old) = b_windows.get(id)
            && old.frame != window.frame
        {
            events.push(Event::WindowUpdated {
                window: *id,
                frame: window.frame,
            });
        }
    }
    for (id, window, info) in &a.workspaces {
        if let Some((_, old)) = b_workspaces.get(id)
            && *old != info
        {
            events.push(Event::WorkspaceUpdated {
                window: *window,
                workspace: info.clone(),
            });
        }
    }
    for (id, _, workspace, tab) in &a.tabs {
        if let Some(old) = b_tabs.get(id)
            && old.name != tab.name
        {
            events.push(Event::TabUpdated {
                workspace: *workspace,
                tab: tab_info(tab),
            });
        }
    }
    for (id, tab, tile) in &a.tiles {
        if let Some(old) = b_tiles.get(id)
            && *old != *tile
        {
            events.push(Event::TileUpdated {
                tab: *tab,
                tile: (*tile).clone(),
            });
        }
    }

    // Reordered: same members, different order.
    for (id, window) in &a.windows {
        if let Some(old) = b_windows.get(id) {
            let new_order: Vec<_> = window.workspaces.iter().map(|w| w.id).collect();
            let old_order: Vec<_> = old.workspaces.iter().map(|w| w.id).collect();
            if new_order != old_order && same_members(&new_order, &old_order) {
                events.push(Event::WorkspacesReordered {
                    window: *id,
                    order: new_order,
                });
            }
            for ws in &window.workspaces {
                let Some(old_ws) = old.workspaces.iter().find(|w| w.id == ws.id) else {
                    continue;
                };
                let new_order: Vec<_> = ws.tabs.iter().map(|t| t.id).collect();
                let old_order: Vec<_> = old_ws.tabs.iter().map(|t| t.id).collect();
                if new_order != old_order && same_members(&new_order, &old_order) {
                    events.push(Event::TabsReordered {
                        workspace: ws.id,
                        order: new_order,
                    });
                }
            }
        }
    }

    // Layout: new tabs and tabs whose tree, focus or zoom changed.
    for (id, window, workspace, tab) in &a.tabs {
        let changed = match b_tabs.get(id) {
            None => true,
            Some(old) => {
                old.root != tab.root || old.focused != tab.focused || old.zoomed != tab.zoomed
            }
        };
        if changed {
            events.push(Event::LayoutChanged(LayoutState {
                window: *window,
                workspace: *workspace,
                tab: *id,
                tree: tab.root.clone(),
                focused: tab.focused,
                zoomed: tab.zoomed,
            }));
        }
    }

    // Closed, inside out.
    for (id, tab, tile) in &b.tiles {
        if !a_tiles.contains_key(id) {
            events.push(Event::TileClosed {
                tab: *tab,
                tile: (*tile).clone(),
                kill,
            });
        }
    }
    for (id, _, workspace, _) in &b.tabs {
        if !a_tabs.contains_key(id) {
            events.push(Event::TabClosed {
                workspace: *workspace,
                tab: *id,
            });
        }
    }
    for (id, window, _) in &b.workspaces {
        if !a_workspaces.contains_key(id) {
            events.push(Event::WorkspaceClosed {
                window: *window,
                workspace: *id,
            });
        }
    }
    for (id, _) in &b.windows {
        if !a_windows.contains_key(id) {
            events.push(Event::WindowClosed { window: *id });
        }
    }

    let focus = after.focus();
    if focus != before.focus() {
        events.push(Event::FocusChanged(focus));
    }
    events
}

fn same_members<T: Ord + Copy>(x: &[T], y: &[T]) -> bool {
    let mut x = x.to_vec();
    let mut y = y.to_vec();
    x.sort();
    y.sort();
    x == y
}
