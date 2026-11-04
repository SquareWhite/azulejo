//! Mirroring each tab's layout into herdr, both ways (ADR 0012).
//!
//! - [`targets`] says what each herdr tab should look like: the tab's tree
//!   with only the live terminal panes on one machine left in, plus zoom.
//! - [`run`] is one machine's mirror task. It pushes a tab's target once the
//!   tab has been quiet for [`PUSH_QUIET`], and pulls a herdr tab
//!   [`PULL_QUIET`] after herdr last said its layout changed. It records
//!   herdr's tree after every push and pull, so our own writes coming back
//!   are recognised and dropped.
//! - [`absorb`] turns a tree changed in herdr into the tab's new
//!   arrangement.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use azulejo_herdr as herdr;
use azulejo_model::{
    Axis, Direction, HerdrRef, LayoutKind, MAX_RATIO, MIN_RATIO, Node, Session, Tab, TabId,
    TileContent, TileId,
};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::link::{HerdrPort, Internal, Machine};

/// How long a tab must be quiet before its layout goes to herdr: a held
/// resize key or a divider drag becomes one push of the settled layout.
pub const PUSH_QUIET: Duration = Duration::from_millis(150);

/// How long after herdr's last `layout_updated` for a tab its tree is read.
pub const PULL_QUIET: Duration = Duration::from_millis(100);

/// How long a push waits for unknown panes in its tab to be adopted before
/// moving them out to a tab of their own.
pub const UNKNOWN_GRACE: Duration = Duration::from_secs(2);

/// What the core tells a machine's mirror task.
#[derive(Debug, Clone, PartialEq)]
pub enum MirrorMsg {
    /// A tab's target changed; `None` stops pushing it.
    Target(TabId, Option<Box<herdr::MirrorTarget>>),
    /// herdr said a tab's layout changed.
    Pull(herdr::TabId),
    /// herdr (re)connected: what was recorded may be stale.
    Reset,
}

// ---- targets ---------------------------------------------------------------

/// A tab's tree with only some leaves kept: each split remembers its path in
/// the full tree, so changes to it can be mapped back.
#[derive(Debug, Clone, PartialEq)]
enum Pruned {
    Leaf(TileId),
    Split {
        path: Vec<bool>,
        axis: Axis,
        ratio: f64,
        a: Box<Pruned>,
        b: Box<Pruned>,
    },
}

impl Pruned {
    fn tiles(&self) -> Vec<TileId> {
        match self {
            Pruned::Leaf(t) => vec![*t],
            Pruned::Split { a, b, .. } => {
                let mut out = a.tiles();
                out.extend(b.tiles());
                out
            }
        }
    }
}

/// `node` without the leaves `keep` rejects; a split losing one side
/// collapses into the other, which takes its space.
fn prune(node: &Node, keep: &dyn Fn(TileId) -> bool) -> Option<Pruned> {
    fn walk(node: &Node, path: &mut Vec<bool>, keep: &dyn Fn(TileId) -> bool) -> Option<Pruned> {
        match node {
            Node::Leaf { id } => keep(*id).then_some(Pruned::Leaf(*id)),
            Node::Split { axis, ratio, a, b } => {
                path.push(false);
                let pa = walk(a, path, keep);
                path.pop();
                path.push(true);
                let pb = walk(b, path, keep);
                path.pop();
                match (pa, pb) {
                    (Some(a), Some(b)) => Some(Pruned::Split {
                        path: path.clone(),
                        axis: *axis,
                        ratio: *ratio,
                        a: Box::new(a),
                        b: Box::new(b),
                    }),
                    (one, None) | (None, one) => one,
                }
            }
        }
    }
    walk(node, &mut Vec::new(), keep)
}

fn direction(axis: Axis) -> herdr::SplitDirection {
    match axis {
        Axis::Horizontal => herdr::SplitDirection::Right,
        Axis::Vertical => herdr::SplitDirection::Down,
    }
}

fn to_herdr(p: &Pruned, pane: &dyn Fn(TileId) -> String) -> herdr::LayoutNode {
    match p {
        Pruned::Leaf(t) => herdr::LayoutNode::pane(pane(*t).as_str()),
        Pruned::Split {
            axis, ratio, a, b, ..
        } => herdr::LayoutNode::split(
            direction(*axis),
            *ratio,
            to_herdr(a, pane),
            to_herdr(b, pane),
        ),
    }
}

/// A tab's live terminal panes on `machine`, by tile.
fn live_panes<'a>(tab: &'a Tab, machine: &Machine) -> HashMap<TileId, &'a HerdrRef> {
    tab.tiles
        .iter()
        .filter_map(|t| match &t.content {
            TileContent::Terminal {
                herdr: Some(h),
                ended: false,
                ..
            } if h.machine == *machine => Some((t.id, h)),
            _ => None,
        })
        .collect()
}

/// Whether a tab still has terminals on `machine` waiting for their pane.
fn waiting(tab: &Tab, workspace_machine: &Machine, machine: &Machine) -> bool {
    workspace_machine == machine
        && tab.tiles.iter().any(|t| {
            matches!(
                &t.content,
                TileContent::Terminal {
                    herdr: None,
                    ended: false,
                    ..
                }
            )
        })
}

/// What every tab with terminals on `machine` should look like in herdr.
/// Tabs whose terminals are still getting panes are left out until they
/// have them. Each herdr tab goes to at most one Azulejo tab: the one with
/// the most panes there (ties to the first in window order); a tab that gets
/// none makes its own.
pub fn targets(session: &Session, machine: &Machine) -> BTreeMap<TabId, herdr::MirrorTarget> {
    let known: Vec<herdr::PaneId> = session
        .tiles()
        .filter_map(|(_, t)| match &t.content {
            TileContent::Terminal { herdr: Some(h), .. } if h.machine == *machine => {
                Some(herdr::PaneId(h.pane_id.clone()))
            }
            _ => None,
        })
        .chain(
            session
                .detached_herdr
                .iter()
                .filter(|h| h.machine == *machine)
                .map(|h| herdr::PaneId(h.pane_id.clone())),
        )
        .collect();

    struct Candidate {
        tab: TabId,
        target: herdr::MirrorTarget,
        /// herdr tabs holding this tab's panes, most first.
        homes: Vec<(String, usize)>,
        rank: usize,
    }
    let mut candidates = Vec::new();
    let mut rank = 0;
    for ws in session.windows.iter().flat_map(|w| &w.workspaces) {
        for tab in &ws.tabs {
            rank += 1;
            if waiting(tab, &ws.machine, machine) {
                continue;
            }
            let live = live_panes(tab, machine);
            let Some(pruned) = prune(&tab.root, &|t| live.contains_key(&t)) else {
                continue;
            };
            let pane = |t: TileId| live[&t].pane_id.clone();
            let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
            for (i, t) in pruned.tiles().iter().enumerate() {
                let e = counts.entry(live[t].tab_id.clone()).or_insert((0, i));
                e.0 += 1;
            }
            let mut homes: Vec<(String, (usize, usize))> = counts.into_iter().collect();
            homes.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.1.1.cmp(&b.1.1)));
            let zoom_tile = match tab.layout.kind {
                LayoutKind::Monocle => Some(tab.focused),
                _ => tab.zoomed,
            };
            let label = tab.name.clone().or_else(|| {
                pruned
                    .tiles()
                    .first()
                    .and_then(|t| tab.tile(*t))
                    .and_then(|t| t.title.clone())
            });
            candidates.push(Candidate {
                tab: tab.id,
                target: herdr::MirrorTarget {
                    tab: None,
                    root: to_herdr(&pruned, &pane),
                    zoomed: zoom_tile
                        .filter(|z| live.contains_key(z))
                        .map(|z| herdr::PaneId(pane(z))),
                    label,
                    known: Vec::new(),
                    park_unknown: false,
                },
                homes: homes.into_iter().map(|(t, (n, _))| (t, n)).collect(),
                rank,
            });
        }
    }
    // Most panes first claim their herdr tab.
    candidates.sort_by(|a, b| {
        let most = |c: &Candidate| c.homes.first().map_or(0, |h| h.1);
        most(b).cmp(&most(a)).then(a.rank.cmp(&b.rank))
    });
    let mut taken: HashSet<String> = HashSet::new();
    let mut out = BTreeMap::new();
    for mut c in candidates {
        let home = c.homes.iter().find(|(t, _)| !taken.contains(t));
        if let Some((t, _)) = home {
            taken.insert(t.clone());
            c.target.tab = Some(herdr::TabId(t.clone()));
        }
        let want: HashSet<&str> = c.target.root.panes().iter().map(|p| p.as_str()).collect();
        c.target.known = known
            .iter()
            .filter(|p| !want.contains(p.as_str()))
            .cloned()
            .collect();
        out.insert(c.tab, c.target);
    }
    out
}

/// Where a new terminal tile's pane is split off, so it's born in place:
/// the pane beside it in its tab's tree (the split's direction and ratio).
/// herdr always puts the new pane second; a tile that goes first is swapped
/// into place by the next push. `None` when the tab has no pane on the
/// machine yet: the terminal gets a herdr tab of its own.
pub fn split_for(session: &Session, tile: TileId) -> Option<herdr::SplitSpec> {
    let path = session.locate(tile).ok()?;
    let (_, ws_id, tab_id) = session.ids_at(path);
    let machine = session.workspace(ws_id).ok()?.machine.clone();
    let tab = session.tab(tab_id).ok()?;
    let live = live_panes(tab, &machine);
    // From the tile up: the nearest split whose other side holds a pane.
    fn find(
        node: &Node,
        tile: TileId,
        live: &HashMap<TileId, &HerdrRef>,
    ) -> Option<(String, Axis, f64)> {
        let Node::Split { axis, ratio, a, b } = node else {
            return None;
        };
        let (mine, other, first) = if a.contains(tile) {
            (a, b, true)
        } else if b.contains(tile) {
            (b, a, false)
        } else {
            return None;
        };
        if let Some(found) = find(mine, tile, live) {
            return Some(found);
        }
        // The pane nearest the tile on the other side.
        let leaves = other.tiles();
        let pick = if first {
            leaves.iter().find(|t| live.contains_key(t))
        } else {
            leaves.iter().rev().find(|t| live.contains_key(t))
        };
        pick.map(|t| (live[t].pane_id.clone(), *axis, *ratio))
    }
    let (pane, axis, ratio) = find(&tab.root, tile, &live)?;
    Some(herdr::SplitSpec {
        target: herdr::PaneId(pane),
        direction: direction(axis),
        ratio: Some(ratio),
        cwd: None,
        env: Vec::new(),
    })
}

// ---- pull: herdr's changes into the model ----------------------------------

fn same_shape(p: &Pruned, h: &herdr::LayoutNode) -> bool {
    match (p, h) {
        (Pruned::Leaf(_), herdr::LayoutNode::Pane { .. }) => true,
        (
            Pruned::Split { axis, a, b, .. },
            herdr::LayoutNode::Split {
                direction: d,
                first,
                second,
                ..
            },
        ) => direction(*axis) == *d && same_shape(a, first) && same_shape(b, second),
        _ => false,
    }
}

fn clamp(r: f64) -> f64 {
    if r.is_finite() {
        r.clamp(MIN_RATIO, MAX_RATIO)
    } else {
        0.5
    }
}

/// Ratio changes, as (full-tree path, ratio), where herdr's tree has the
/// pruned tree's shape.
fn ratio_changes(p: &Pruned, h: &herdr::LayoutNode, out: &mut Vec<(Vec<bool>, f64)>) {
    if let (
        Pruned::Split {
            path, ratio, a, b, ..
        },
        herdr::LayoutNode::Split {
            ratio: hr,
            first,
            second,
            ..
        },
    ) = (p, h)
    {
        if (ratio - hr).abs() > herdr::ECHO_TOLERANCE {
            out.push((path.clone(), clamp(*hr)));
        }
        ratio_changes(a, first, out);
        ratio_changes(b, second, out);
    }
}

fn set_ratio(node: &mut Node, path: &[bool], value: f64) {
    match (node, path.split_first()) {
        (Node::Split { ratio, .. }, None) => *ratio = value,
        (Node::Split { a, b, .. }, Some((second, rest))) => {
            set_ratio(if *second { b } else { a }, rest, value)
        }
        _ => {}
    }
}

fn relabel(node: &mut Node, map: &HashMap<TileId, TileId>) {
    match node {
        Node::Leaf { id } => {
            if let Some(to) = map.get(id) {
                *id = *to;
            }
        }
        Node::Split { a, b, .. } => {
            relabel(a, map);
            relabel(b, map);
        }
    }
}

fn from_herdr(h: &herdr::LayoutNode, tile: &dyn Fn(&str) -> Option<TileId>) -> Option<Node> {
    match h {
        herdr::LayoutNode::Pane { pane_id } => tile(pane_id.as_ref()?.as_str()).map(Node::leaf),
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
            ratio: clamp(*ratio),
            a: Box::new(from_herdr(first, tile)?),
            b: Box::new(from_herdr(second, tile)?),
        }),
    }
}

/// Where `tile` sat in `old`: beside which tiles, on which side, with what
/// ratio (the split's first child's share).
fn old_place(old: &Node, tile: TileId) -> Option<(Vec<TileId>, Direction, f64)> {
    let Node::Split { axis, ratio, a, b } = old else {
        return None;
    };
    let leaf = |n: &Node| matches!(n, Node::Leaf { id } if *id == tile);
    if leaf(a) {
        let dir = if *axis == Axis::Horizontal {
            Direction::Left
        } else {
            Direction::Up
        };
        return Some((b.tiles(), dir, *ratio));
    }
    if leaf(b) {
        let dir = if *axis == Axis::Horizontal {
            Direction::Right
        } else {
            Direction::Down
        };
        return Some((a.tiles(), dir, *ratio));
    }
    old_place(a, tile).or_else(|| old_place(b, tile))
}

/// The tab's tree built from herdr's, with the tiles herdr doesn't have
/// (browsers, previews, files, other machines) put back beside their old
/// neighbours.
fn rebuild(old: &Node, herdr_tree: Node, others: &[TileId]) -> Node {
    let mut root = herdr_tree;
    let mut pending: Vec<TileId> = others.to_vec();
    loop {
        let before = pending.len();
        pending.retain(|t| {
            let Some((beside, dir, ratio)) = old_place(old, *t) else {
                return true;
            };
            let Some(anchor) = beside.iter().find(|b| root.contains(**b)) else {
                return true;
            };
            root.split_with_ratio(*anchor, dir, *t, ratio).is_err()
        });
        if pending.is_empty() || pending.len() == before {
            break;
        }
    }
    for t in pending {
        let _ = root.split_auto(t);
    }
    root
}

/// Applies a tab layout read from `machine`'s herdr after someone changed it
/// there (ADR 0012). Ratios and moved panes map onto the tab's own tree;
/// another shape replaces it, with the tiles herdr doesn't have put back.
/// Dynamic tabs absorb what their rule can make and freeze otherwise
/// ([`Session::apply_tree`]). Zoom follows; unzooming a Monocle tab in herdr
/// leaves Monocle. Returns whether anything changed; nothing does while
/// herdr's panes and the tab's don't match (a split or close on its way).
pub fn absorb(session: &mut Session, machine: &Machine, layout: &herdr::TabLayout) -> bool {
    let panes: Vec<String> = layout.root.panes().iter().map(|p| p.0.clone()).collect();
    // The Azulejo tab showing these panes.
    let mut owner: Option<TabId> = None;
    let mut by_pane: HashMap<String, TileId> = HashMap::new();
    for ws in session.windows.iter().flat_map(|w| &w.workspaces) {
        for tab in &ws.tabs {
            for (tile, h) in live_panes(tab, machine) {
                if panes.contains(&h.pane_id) {
                    if owner.is_some_and(|o| o != tab.id) {
                        return false; // spread over tabs: not mirrored yet
                    }
                    owner = Some(tab.id);
                    by_pane.insert(h.pane_id.clone(), tile);
                }
            }
        }
    }
    let Some(tab_id) = owner else {
        return false;
    };
    if by_pane.len() != panes.len() {
        return false; // a pane nobody shows yet
    }
    let tab = session.tab(tab_id).expect("found above");
    let live: HashSet<TileId> = live_panes(tab, machine).into_keys().collect();
    if live.len() != panes.len() {
        return false; // the tab has a pane herdr's tab doesn't
    }
    let Some(pruned) = prune(&tab.root, &|t| live.contains(&t)) else {
        return false;
    };
    let tile_of = |p: &str| by_pane.get(p).copied();
    let mut changed = false;

    let new_root = if same_shape(&pruned, &layout.root) {
        let mut root = tab.root.clone();
        let mut ratios = Vec::new();
        ratio_changes(&pruned, &layout.root, &mut ratios);
        for (path, r) in ratios {
            set_ratio(&mut root, &path, r);
        }
        let ours = pruned.tiles();
        let theirs: Vec<TileId> = panes.iter().filter_map(|p| tile_of(p)).collect();
        if ours != theirs {
            let map: HashMap<TileId, TileId> = ours.into_iter().zip(theirs).collect();
            relabel(&mut root, &map);
        }
        root
    } else {
        let Some(tree) = from_herdr(&layout.root, &tile_of) else {
            return false;
        };
        let others: Vec<TileId> = tab
            .root
            .tiles()
            .into_iter()
            .filter(|t| !live.contains(t))
            .collect();
        rebuild(&tab.root, tree, &others)
    };
    let kind = tab.layout.kind;
    let (zoomed, focused) = (tab.zoomed, tab.focused);
    if new_root != tab.root {
        changed |= session.apply_tree(tab_id, new_root).unwrap_or(false);
    }

    // Zoom (and Monocle's top tile).
    let herdr_focus = layout
        .focused_pane_id
        .as_ref()
        .and_then(|p| tile_of(p.as_str()));
    match (kind, layout.zoomed, herdr_focus) {
        (LayoutKind::Monocle, false, _) => {
            changed |= session
                .toggle_layout(tab_id, LayoutKind::Monocle)
                .unwrap_or(false);
        }
        (LayoutKind::Monocle, true, Some(f)) if f != focused => {
            changed |= session.set_tab_focus(tab_id, f).unwrap_or(false);
        }
        (LayoutKind::Monocle, ..) => {}
        (_, true, Some(f)) if zoomed != Some(f) => {
            changed |= session.set_tab_zoom(tab_id, Some(f)).unwrap_or(false);
        }
        (_, false, _) if zoomed.is_some_and(|z| live.contains(&z)) => {
            changed |= session.set_tab_zoom(tab_id, None).unwrap_or(false);
        }
        _ => {}
    }
    changed
}

// ---- the task ----------------------------------------------------------------

/// `AZJ_DEBUG_MIRROR=1` logs every push and every pulled change.
pub(crate) fn debug() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("AZJ_DEBUG_MIRROR").is_some_and(|v| v != "0"))
}

struct Pending {
    target: herdr::MirrorTarget,
    due: Instant,
    /// When the push first met panes nobody knows.
    unknown_since: Option<Instant>,
    failures: u32,
}

/// One machine's mirror task: debounced pushes and pulls, one at a time.
pub(crate) async fn run<P: HerdrPort>(
    machine: Machine,
    port: Arc<P>,
    mut rx: mpsc::UnboundedReceiver<MirrorMsg>,
    out: impl Fn(Internal),
    log: impl Fn(String),
) {
    let mut pushes: HashMap<TabId, Pending> = HashMap::new();
    let mut pulls: HashMap<herdr::TabId, Instant> = HashMap::new();
    // herdr's tree after our last push or pull, per herdr tab.
    let mut synced: HashMap<herdr::TabId, herdr::TabLayout> = HashMap::new();
    loop {
        let next = pushes
            .values()
            .map(|p| p.due)
            .chain(pulls.values().copied())
            .min();
        let msg = match next {
            Some(at) => tokio::select! {
                m = rx.recv() => Some(m),
                () = tokio::time::sleep_until(at) => None,
            },
            None => Some(rx.recv().await),
        };
        match msg {
            Some(None) => return,
            Some(Some(MirrorMsg::Target(tab, None))) => {
                pushes.remove(&tab);
            }
            Some(Some(MirrorMsg::Target(tab, Some(target)))) => {
                let unknown_since = pushes.get(&tab).and_then(|p| p.unknown_since);
                pushes.insert(
                    tab,
                    Pending {
                        target: *target,
                        due: Instant::now() + PUSH_QUIET,
                        unknown_since,
                        failures: 0,
                    },
                );
            }
            Some(Some(MirrorMsg::Pull(tab))) => {
                pulls.insert(tab, Instant::now() + PULL_QUIET);
            }
            Some(Some(MirrorMsg::Reset)) => synced.clear(),
            None => {
                let now = Instant::now();
                // Pulls first: a change made in herdr is read before a push
                // could overwrite it.
                let due: Vec<herdr::TabId> = pulls
                    .iter()
                    .filter(|(_, at)| **at <= now)
                    .map(|(t, _)| t.clone())
                    .collect();
                for tab in due {
                    pulls.remove(&tab);
                    if pull(&machine, &*port, &tab, &mut synced, &out, &log).await {
                        // The core's next target reflects it; a push queued
                        // before would undo it.
                        for p in pushes.values_mut() {
                            if p.target.tab.as_ref() == Some(&tab) {
                                p.due = p.due.max(now + PUSH_QUIET);
                            }
                        }
                    }
                }
                let due: Vec<TabId> = pushes
                    .iter()
                    .filter(|(_, p)| p.due <= now)
                    .map(|(t, _)| *t)
                    .collect();
                for tab in due {
                    let Some(mut p) = pushes.remove(&tab) else {
                        continue;
                    };
                    // A pull still due for this herdr tab goes first.
                    if let Some(h) = &p.target.tab
                        && pulls.contains_key(h)
                    {
                        p.due = now + PULL_QUIET;
                        pushes.insert(tab, p);
                        continue;
                    }
                    // herdr changed since we last looked, and the event
                    // hasn't come yet: read it first, so a push computed
                    // before a TUI drag doesn't undo it.
                    if let Some(h) = p.target.tab.clone()
                        && synced.contains_key(&h)
                        && pull(&machine, &*port, &h, &mut synced, &out, &log).await
                    {
                        p.due = now + PUSH_QUIET;
                        pushes.insert(tab, p);
                        continue;
                    }
                    if p.unknown_since.is_some_and(|t| now - t >= UNKNOWN_GRACE) {
                        p.target.park_unknown = true;
                    }
                    let result = port.mirror(p.target.clone()).await;
                    if debug() {
                        log(format!(
                            "mirror: push {tab} into {:?} ({:?}): {}",
                            p.target.tab,
                            p.target.root,
                            match &result {
                                Ok(o) => format!("{} writes, {:?}", o.writes, o.layout.root),
                                Err(e) => e.to_string(),
                            }
                        ));
                    }
                    match result {
                        Ok(outcome) => {
                            let mut layout = outcome.layout;
                            // Changed in herdr between our writes and our
                            // read: record what we wrote, so the pull that
                            // follows sees the change instead of an echo.
                            if !herdr::satisfies(&layout, &p.target) {
                                layout.root = p.target.root.clone();
                                layout.zoomed = p.target.zoomed.is_some();
                                if p.target.zoomed.is_some() {
                                    layout.focused_pane_id = p.target.zoomed.clone();
                                }
                                pulls.insert(layout.tab_id.clone(), now + PULL_QUIET);
                            }
                            synced.insert(layout.tab_id.clone(), layout);
                        }
                        Err(herdr::MirrorError::Unknown(_)) => {
                            p.unknown_since.get_or_insert(now);
                            p.due = now + Duration::from_millis(250);
                            pushes.insert(tab, p);
                        }
                        Err(e) => {
                            p.failures += 1;
                            if p.failures < 3 {
                                p.due = now + Duration::from_millis(500);
                                pushes.insert(tab, p);
                            } else {
                                log(format!("herdr: mirroring tab {tab}: {e}"));
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Reads a herdr tab and compares it with what was recorded. Returns whether
/// someone changed its layout there (and the core was told).
async fn pull<P: HerdrPort>(
    machine: &Machine,
    port: &P,
    tab: &herdr::TabId,
    synced: &mut HashMap<herdr::TabId, herdr::TabLayout>,
    out: &impl Fn(Internal),
    log: &impl Fn(String),
) -> bool {
    let layout = match port.layout_export(tab.0.clone()).await {
        Ok(layout) => layout,
        Err(e) => {
            synced.remove(tab);
            if !e.contains("not_found") && !e.contains("not found") {
                log(format!("herdr: reading tab {tab}: {e}"));
            }
            return false;
        }
    };
    let Some(before) = synced.get(tab) else {
        // Nothing to compare with: Azulejo's next push decides.
        synced.insert(tab.clone(), layout);
        return false;
    };
    if herdr::same_layout(before, &layout) {
        return false; // our own write coming back
    }
    let leaves = |l: &herdr::TabLayout| {
        let mut p: Vec<String> = l.root.panes().iter().map(|p| p.0.clone()).collect();
        p.sort();
        p
    };
    let same_panes = leaves(before) == leaves(&layout);
    if debug() {
        log(format!(
            "mirror: pulled {tab}: {:?} (was {:?}){}",
            layout.root,
            before.root,
            if same_panes { ", absorbing" } else { "" }
        ));
    }
    synced.insert(tab.clone(), layout.clone());
    if same_panes {
        out(Internal::MirrorPulled {
            machine: machine.clone(),
            layout: Box::new(layout),
        });
    }
    same_panes
}

#[cfg(test)]
#[path = "mirror_tests.rs"]
mod tests;
