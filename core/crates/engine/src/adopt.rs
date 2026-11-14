//! Adopting what already runs in herdr (ADR 0008).
//!
//! Azulejo connects to the user's own herdr session, so on connect (and after
//! herdr says events were lost) it mirrors every herdr tab it doesn't know yet:
//! one Azulejo workspace per herdr workspace, one tab per herdr tab with the
//! same split tree, one terminal tile per pane. It only reads herdr to do so.
//!
//! A herdr tab is adopted whole, and only when none of its panes is known: a
//! pane counts as known when a tile is bound to it or a tile showing it was
//! closed here (`Session::detached_herdr`). Panes split off later inside a
//! tab shown here (the herdr TUI, a phone, an agent) join that tab beside
//! their herdr sibling (F15, ADR 0009).

use std::collections::HashMap;

use azulejo_herdr as herdr;
use azulejo_model::{
    Axis, HerdrRef, MAX_RATIO, MIN_RATIO, Node, Session, Tab, TabId, Tile, TileContent, TileId,
    WorkspaceId,
};

/// Mirrors herdr tabs nobody here knows about into `session`. `placeholder` is
/// the empty workspace a fresh session starts with: it's dropped when adoption
/// brought in anything, so a first launch shows the user's herdr workspaces
/// instead of an extra empty one. Returns whether anything changed.
pub fn adopt(
    session: &mut Session,
    survey: &herdr::Survey,
    placeholder: Option<WorkspaceId>,
) -> bool {
    let detached_before = session.detached_herdr.len();
    session
        .detached_herdr
        .retain(|r| survey.panes.iter().any(|p| same_pane(p, r)));
    let mut changed = session.detached_herdr.len() != detached_before;

    let known: Vec<HerdrRef> = session
        .tiles()
        .filter_map(|(_, t)| match &t.content {
            TileContent::Terminal { herdr: Some(h), .. } => Some(h.clone()),
            _ => None,
        })
        .chain(session.detached_herdr.iter().cloned())
        .collect();
    let is_known = |p: &herdr::PaneInfo| known.iter().any(|r| same_pane(p, r));

    let mut workspaces: Vec<&herdr::WorkspaceInfo> = survey.workspaces.iter().collect();
    workspaces.sort_by_key(|w| w.number);
    let mut adopted_any = false;
    // herdr workspace → Azulejo workspace, for focusing afterwards.
    let mut mapped: HashMap<&str, WorkspaceId> = HashMap::new();

    for hw in workspaces {
        let mut tabs: Vec<&herdr::TabInfo> = survey
            .tabs
            .iter()
            .filter(|t| t.workspace_id == hw.workspace_id)
            .collect();
        tabs.sort_by_key(|t| t.number);
        let adoptable: Vec<(&herdr::TabInfo, Vec<&herdr::PaneInfo>)> = tabs
            .into_iter()
            .map(|t| {
                let panes: Vec<_> = survey
                    .panes
                    .iter()
                    .filter(|p| p.tab_id == t.tab_id)
                    .collect();
                (t, panes)
            })
            .filter(|(_, panes)| !panes.is_empty() && !panes.iter().any(|p| is_known(p)))
            .collect();

        let existing = find_workspace(session, hw.workspace_id.as_str());
        if let Some(id) = existing {
            mapped.insert(hw.workspace_id.as_str(), id);
        }
        if adoptable.is_empty() {
            continue;
        }
        let (workspace, created) = match existing {
            Some(id) => (id, false),
            None => (new_workspace(session, hw), true),
        };
        mapped.insert(hw.workspace_id.as_str(), workspace);

        let mut active = None;
        for (tab, panes) in adoptable {
            let layout = survey.layouts.iter().find(|l| l.tab_id == tab.tab_id);
            let built = build_tab(tab, &panes, layout);
            let id = built.id;
            if session.insert_tab(workspace, built).is_ok() {
                adopted_any = true;
                if hw.active_tab_id.as_ref() == Some(&tab.tab_id) {
                    active = Some(id);
                }
            }
        }
        if created && let Some(tab) = active {
            set_active_tab(session, workspace, tab);
        }
    }
    changed |= adopted_any;
    changed |= adopt_split_panes(session, survey, &is_known);

    if adopted_any
        && let Some(ph) = placeholder
        && is_pristine(session, ph)
    {
        let focus = survey
            .focused_workspace_id
            .as_ref()
            .and_then(|w| mapped.get(w.as_str()).copied())
            .filter(|w| *w != ph)
            .or_else(|| {
                session
                    .windows
                    .iter()
                    .flat_map(|w| &w.workspaces)
                    .find(|w| w.id != ph)
                    .map(|w| w.id)
            });
        if let Some(focus) = focus {
            let _ = session.focus_workspace(focus);
            let _ = session.close_workspace(ph);
        }
    }
    changed
}

/// The same herdr pane: by terminal ID (survives moves), or by pane and tab ID
/// (survives a herdr restart, which gives the slot a fresh terminal).
pub fn same_pane(p: &herdr::PaneInfo, r: &HerdrRef) -> bool {
    p.terminal_id.as_str() == r.terminal_id
        || (p.pane_id.as_str() == r.pane_id && p.tab_id.as_str() == r.tab_id)
}

fn find_workspace(session: &Session, herdr: &str) -> Option<WorkspaceId> {
    session
        .windows
        .iter()
        .flat_map(|w| &w.workspaces)
        .find(|w| w.herdr_workspace.as_deref() == Some(herdr))
        .map(|w| w.id)
}

/// A new Azulejo workspace bound to `hw`, added without moving focus.
fn new_workspace(session: &mut Session, hw: &herdr::WorkspaceInfo) -> WorkspaceId {
    let name = hw
        .label
        .clone()
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| format!("herdr {}", hw.number));
    let target = session
        .focused_window
        .or_else(|| session.windows.first().map(|w| w.id));
    let id = match target {
        Some(window) => {
            let before = session
                .windows
                .iter()
                .find(|w| w.id == window)
                .map(|w| w.active_workspace);
            let focused = session.focused_window;
            let id = session
                .create_workspace(window, name, None)
                .expect("target window exists");
            if let (Some(before), Some(w)) =
                (before, session.windows.iter_mut().find(|w| w.id == window))
            {
                w.active_workspace = before;
            }
            session.focused_window = focused;
            id
        }
        None => session.create_window(name, None).1,
    };
    session
        .bind_workspace(id, hw.workspace_id.0.clone())
        .expect("workspace just created");
    id
}

fn set_active_tab(session: &mut Session, workspace: WorkspaceId, tab: TabId) {
    if let Some(ws) = session
        .windows
        .iter_mut()
        .flat_map(|w| w.workspaces.iter_mut())
        .find(|w| w.id == workspace)
    {
        ws.active_tab = Some(tab);
    }
}

/// The untouched workspace a fresh session starts with: one tab holding one
/// terminal that never got a pane.
fn is_pristine(session: &Session, id: WorkspaceId) -> bool {
    let Ok(ws) = session.workspace(id) else {
        return false;
    };
    ws.herdr_workspace.is_none()
        && ws.tabs.len() == 1
        && ws.tabs[0].tiles.len() == 1
        && matches!(
            &ws.tabs[0].tiles[0],
            Tile {
                title: None,
                content: TileContent::Terminal {
                    cwd: None,
                    command: None,
                    herdr: None,
                    ended: false,
                    ..
                },
                ..
            }
        )
}

/// An Azulejo tab mirroring one herdr tab. Uses herdr's split tree when it
/// matches the panes; otherwise lays the panes out side by side.
pub fn build_tab(
    tab: &herdr::TabInfo,
    panes: &[&herdr::PaneInfo],
    layout: Option<&herdr::TabLayout>,
) -> Tab {
    let tiles: Vec<Tile> = panes.iter().map(|p| pane_tile(p)).collect();
    let by_pane: HashMap<&str, TileId> = panes
        .iter()
        .zip(&tiles)
        .map(|(p, t)| (p.pane_id.as_str(), t.id))
        .collect();

    let root = layout
        .filter(|l| {
            let mut leaves: Vec<&str> = l.root.panes().iter().map(|p| p.as_str()).collect();
            leaves.sort_unstable();
            let mut want: Vec<&str> = by_pane.keys().copied().collect();
            want.sort_unstable();
            leaves == want
        })
        .and_then(|l| convert(&l.root, &by_pane))
        .unwrap_or_else(|| side_by_side(&tiles.iter().map(|t| t.id).collect::<Vec<_>>()));

    let focused_pane = layout
        .and_then(|l| l.focused_pane_id.as_ref())
        .map(|p| p.as_str())
        .or_else(|| panes.iter().find(|p| p.focused).map(|p| p.pane_id.as_str()));
    let focused = focused_pane
        .and_then(|p| by_pane.get(p).copied())
        .unwrap_or(tiles[0].id);
    let zoomed = layout.is_some_and(|l| l.zoomed).then_some(focused);

    Tab {
        id: TabId::new(),
        name: tab.label.clone().filter(|l| !l.trim().is_empty()),
        root,
        tiles,
        focused,
        zoomed,
    }
}

/// A terminal tile bound to an existing herdr pane. Its agent state comes
/// from the survey afterwards (`agent::refresh`).
fn pane_tile(p: &herdr::PaneInfo) -> Tile {
    let mut content = TileContent::terminal();
    if let TileContent::Terminal { cwd, herdr, .. } = &mut content {
        *cwd = p.cwd.clone();
        *herdr = Some(HerdrRef {
            pane_id: p.pane_id.0.clone(),
            terminal_id: p.terminal_id.0.clone(),
            tab_id: p.tab_id.0.clone(),
        });
    }
    Tile {
        title: p.label.clone().filter(|l| !l.trim().is_empty()),
        ..Tile::new(content)
    }
}

/// Panes split off elsewhere inside herdr tabs already shown here (F15).
/// Each lands next to the tile of its sibling in herdr's split tree, on the
/// same side and with the same ratio; without a usable sibling it goes to the
/// end of the tab's stack (`layout.split_auto`). Focus doesn't move.
///
/// Returns whether anything was added.
fn adopt_split_panes(
    session: &mut Session,
    survey: &herdr::Survey,
    is_known: &dyn Fn(&herdr::PaneInfo) -> bool,
) -> bool {
    let mut added = false;
    for tab in &survey.tabs {
        let panes: Vec<&herdr::PaneInfo> = survey
            .panes
            .iter()
            .filter(|p| p.tab_id == tab.tab_id)
            .collect();
        let mut pending: Vec<&herdr::PaneInfo> =
            panes.iter().copied().filter(|p| !is_known(p)).collect();
        // Only tabs with a pane bound here; wholly new tabs are `adopt`'s.
        if pending.is_empty() || pending.len() == panes.len() {
            continue;
        }
        let layout = survey.layouts.iter().find(|l| l.tab_id == tab.tab_id);
        // Place what has a bound sibling; each placed pane can anchor the next.
        loop {
            let before = pending.len();
            pending.retain(|p| {
                let Some((anchor, direction, ratio)) =
                    layout.and_then(|l| beside(session, &l.root, p.pane_id.as_str()))
                else {
                    return true;
                };
                let placed = session
                    .insert_tile_beside(anchor, direction, pane_tile(p), ratio)
                    .is_ok();
                added |= placed;
                !placed
            });
            if pending.len() == before {
                break;
            }
        }
        // The rest: the stack of the tab showing this herdr tab's panes.
        let Some(target) = panes
            .iter()
            .find_map(|p| tile_for_pane(session, p.pane_id.as_str()))
            .and_then(|tile| session.locate(tile).ok())
            .map(|path| session.ids_at(path).2)
        else {
            continue;
        };
        for p in pending {
            added |= session.insert_tile_auto(target, pane_tile(p)).is_ok();
        }
    }
    added
}

/// The bound, live tile showing `pane`.
fn tile_for_pane(session: &Session, pane: &str) -> Option<TileId> {
    session.tiles().find_map(|(_, t)| match &t.content {
        TileContent::Terminal {
            herdr: Some(h),
            ended: false,
            ..
        } if h.pane_id == pane => Some(t.id),
        _ => None,
    })
}

/// Where herdr put `pane`: the tile of a bound pane in its sibling subtree,
/// the side `pane` is on, and the split's ratio.
fn beside(
    session: &Session,
    root: &herdr::LayoutNode,
    pane: &str,
) -> Option<(TileId, azulejo_model::Direction, f64)> {
    use azulejo_model::Direction;
    let herdr::LayoutNode::Split {
        direction,
        ratio,
        first,
        second,
    } = root
    else {
        return None;
    };
    let holds = |n: &herdr::LayoutNode| n.panes().iter().any(|p| p.as_str() == pane);
    let is_leaf = |n: &herdr::LayoutNode| matches!(n, herdr::LayoutNode::Pane { pane_id: Some(p) } if p.as_str() == pane);
    let (sibling, forward) = if is_leaf(second) {
        (first, true)
    } else if is_leaf(first) {
        (second, false)
    } else if holds(first) {
        return beside(session, first, pane);
    } else if holds(second) {
        return beside(session, second, pane);
    } else {
        return None;
    };
    let anchor = sibling
        .panes()
        .iter()
        .find_map(|p| tile_for_pane(session, p.as_str()))?;
    let direction = match (direction, forward) {
        (herdr::SplitDirection::Right, true) => Direction::Right,
        (herdr::SplitDirection::Right, false) => Direction::Left,
        (herdr::SplitDirection::Down, true) => Direction::Down,
        (herdr::SplitDirection::Down, false) => Direction::Up,
    };
    Some((anchor, direction, *ratio))
}

fn convert(node: &herdr::LayoutNode, by_pane: &HashMap<&str, TileId>) -> Option<Node> {
    match node {
        herdr::LayoutNode::Pane { pane_id } => by_pane
            .get(pane_id.as_ref()?.as_str())
            .map(|id| Node::leaf(*id)),
        herdr::LayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } => Some(Node::Split {
            axis: match direction {
                herdr::SplitDirection::Right => Axis::Horizontal,
                herdr::SplitDirection::Down => Axis::Vertical,
            },
            ratio: if ratio.is_finite() {
                ratio.clamp(MIN_RATIO, MAX_RATIO)
            } else {
                0.5
            },
            a: Box::new(convert(first, by_pane)?),
            b: Box::new(convert(second, by_pane)?),
        }),
    }
}

/// Equal columns, for a tab whose split tree herdr didn't give us.
fn side_by_side(tiles: &[TileId]) -> Node {
    match tiles {
        [only] => Node::leaf(*only),
        [first, rest @ ..] => Node::Split {
            axis: Axis::Horizontal,
            ratio: (1.0 / tiles.len() as f64).clamp(MIN_RATIO, MAX_RATIO),
            a: Box::new(Node::leaf(*first)),
            b: Box::new(side_by_side(rest)),
        },
        [] => unreachable!("a herdr tab always has a pane"),
    }
}

#[cfg(test)]
#[path = "adopt_tests.rs"]
pub(crate) mod tests;
