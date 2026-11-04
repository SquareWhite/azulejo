//! Making a herdr tab's split tree match a target, pane by pane, without
//! closing anything (ADR 0012).
//!
//! herdr can't apply a tree to live panes (`layout.apply` replaces them), so
//! a push works with what herdr offers on live panes and keeps every process:
//! - the same shape: `layout.set_split_ratio` per split, `pane.swap` for
//!   panes in each other's places;
//! - anything else: a rebuild. Every pane but the anchor (the target's first)
//!   moves out to one staging tab, then each moves back beside its neighbour
//!   with the right direction and ratio. herdr refuses moves inside one tab,
//!   hence the staging tab; it deletes the tab when the last pane leaves;
//! - zoom last: herdr refuses moves out of a zoomed tab, and a split clears
//!   zoom.
//!
//! Panes in the tab that the target doesn't hold move out to a tab of their
//! own when they're known to belong elsewhere; an unknown one (split off in
//! the TUI a moment ago) makes the push wait, since the core is about to
//! adopt it.
//!
//! **herdr's own arrangement is never rewritten unasked** (ADR 0012,
//! amendment "Only Azulejo changes herdr's structure"). Unless the target
//! says [`MirrorTarget::restructure`], the push first compares the tab with
//! the target over the panes both hold. If they sit in another shape or
//! order, it writes nothing and fails with [`MirrorError::Diverged`]: the
//! caller follows herdr instead. Without `restructure` a push still sets
//! ratios and zoom, and still moves panes in and out of the tab (a merge, a
//! pane closed here), which leaves the panes that stay where herdr has them.

use std::collections::HashMap;
use std::future::Future;

use crate::client::Client;
use crate::error::Error;
use crate::types::{
    LayoutNode, MoveTo, Moved, PaneId, PaneInfo, SplitDirection, TabId, TabLayout, ZoomState,
};

/// A push writes a ratio only when herdr's is further off than this: half a
/// cell at 100 columns.
pub const RATIO_TOLERANCE: f64 = 0.005;

/// herdr keeps ratios as `f32`: what it reports back differs from what was
/// set by up to this much.
pub const ECHO_TOLERANCE: f64 = 1e-4;

/// What a herdr tab should look like.
#[derive(Debug, Clone, PartialEq)]
pub struct MirrorTarget {
    /// The herdr tab to mirror into. `None` makes one, by moving the
    /// target's first pane out to a new tab.
    pub tab: Option<TabId>,
    /// The tree: every leaf names a live pane, in any tab of the server.
    pub root: LayoutNode,
    /// The pane to zoom, if any.
    pub zoomed: Option<PaneId>,
    /// The label of a tab the push makes.
    pub label: Option<String>,
    /// Panes known to belong elsewhere (another tab's, or closed here): when
    /// found in the tab they move out to a tab of their own.
    pub known: Vec<PaneId>,
    /// Move out unknown panes too, instead of waiting for them to be
    /// adopted. Only with `restructure`.
    pub park_unknown: bool,
    /// The change comes from Azulejo (a user op or a command in the app), so
    /// the push may rearrange the panes already in the tab: swap them, or
    /// rebuild the tab in another shape. Without it, a tab whose panes sit
    /// in another shape or order than the target's fails with
    /// [`MirrorError::Diverged`] before anything is written.
    pub restructure: bool,
}

/// What a push did.
#[derive(Debug, Clone, PartialEq)]
pub struct MirrorOutcome {
    /// The tab's tree afterwards.
    pub layout: TabLayout,
    /// Every pane that moved, as herdr reported it after its last move.
    pub moved: Vec<PaneInfo>,
    /// How many herdr calls changed something.
    pub writes: usize,
}

#[derive(Debug)]
pub enum MirrorError {
    /// Panes nobody knows yet sit in the tab; retry once they're adopted.
    Unknown(Vec<PaneId>),
    /// A pane in the target doesn't exist (any more).
    Missing(PaneId),
    /// The target tab doesn't exist (any more).
    TabGone(TabId),
    /// The tab's panes sit in another shape or order than the target's and
    /// the target may not restructure: nothing was written. The layout is
    /// the tab as herdr has it; the caller follows it.
    Diverged(TabLayout),
    /// herdr refused a move.
    Refused {
        pane: PaneId,
        reason: String,
    },
    Herdr(Error),
}

impl std::fmt::Display for MirrorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MirrorError::Unknown(p) => write!(f, "unknown panes in the tab: {p:?}"),
            MirrorError::Missing(p) => write!(f, "no pane {p}"),
            MirrorError::TabGone(t) => write!(f, "no tab {t}"),
            MirrorError::Diverged(l) => write!(
                f,
                "herdr's tab {} is arranged differently and the change isn't Azulejo's",
                l.tab_id
            ),
            MirrorError::Refused { pane, reason } => {
                write!(f, "herdr refused to move {pane}: {reason}")
            }
            MirrorError::Herdr(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MirrorError {}

impl From<Error> for MirrorError {
    fn from(e: Error) -> Self {
        MirrorError::Herdr(e)
    }
}

type Result<T> = std::result::Result<T, MirrorError>;

/// The herdr calls a push needs. [`Client`] implements it; tests use a
/// simulated herdr.
pub trait LayoutOps: Sync {
    fn panes(&self) -> impl Future<Output = crate::Result<Vec<PaneInfo>>> + Send;
    fn export(&self, tab: &TabId) -> impl Future<Output = crate::Result<TabLayout>> + Send;
    fn move_pane(
        &self,
        pane: &PaneId,
        to: &MoveTo,
    ) -> impl Future<Output = crate::Result<Moved>> + Send;
    fn swap(&self, a: &PaneId, b: &PaneId) -> impl Future<Output = crate::Result<()>> + Send;
    fn set_ratio(
        &self,
        tab: &TabId,
        path: &[bool],
        ratio: f64,
    ) -> impl Future<Output = crate::Result<TabLayout>> + Send;
    fn zoom(
        &self,
        pane: &PaneId,
        on: bool,
    ) -> impl Future<Output = crate::Result<ZoomState>> + Send;
}

impl LayoutOps for Client {
    fn panes(&self) -> impl Future<Output = crate::Result<Vec<PaneInfo>>> + Send {
        Client::panes(self)
    }
    fn export(&self, tab: &TabId) -> impl Future<Output = crate::Result<TabLayout>> + Send {
        self.layout_export(tab)
    }
    fn move_pane(
        &self,
        pane: &PaneId,
        to: &MoveTo,
    ) -> impl Future<Output = crate::Result<Moved>> + Send {
        Client::move_pane(self, pane, to)
    }
    fn swap(&self, a: &PaneId, b: &PaneId) -> impl Future<Output = crate::Result<()>> + Send {
        self.swap_panes(a, b)
    }
    fn set_ratio(
        &self,
        tab: &TabId,
        path: &[bool],
        ratio: f64,
    ) -> impl Future<Output = crate::Result<TabLayout>> + Send {
        self.set_split_ratio(tab, path, ratio)
    }
    fn zoom(
        &self,
        pane: &PaneId,
        on: bool,
    ) -> impl Future<Output = crate::Result<ZoomState>> + Send {
        self.zoom_pane(pane, on)
    }
}

impl Client {
    /// Makes a herdr tab look like `target` (ADR 0012). Never closes a pane.
    pub async fn mirror(&self, target: &MirrorTarget) -> Result<MirrorOutcome> {
        run(self, target).await
    }
}

/// Same shape: the same splits in the same directions, whatever the panes
/// and ratios.
pub fn same_shape(a: &LayoutNode, b: &LayoutNode) -> bool {
    match (a, b) {
        (LayoutNode::Pane { .. }, LayoutNode::Pane { .. }) => true,
        (
            LayoutNode::Split {
                direction: da,
                first: fa,
                second: sa,
                ..
            },
            LayoutNode::Split {
                direction: db,
                first: fb,
                second: sb,
                ..
            },
        ) => da == db && same_shape(fa, fb) && same_shape(sa, sb),
        _ => false,
    }
}

/// The same tree: shape, panes and ratios within `tolerance`.
pub fn same_tree(a: &LayoutNode, b: &LayoutNode, tolerance: f64) -> bool {
    match (a, b) {
        (LayoutNode::Pane { pane_id: pa }, LayoutNode::Pane { pane_id: pb }) => pa == pb,
        (
            LayoutNode::Split {
                direction: da,
                ratio: ra,
                first: fa,
                second: sa,
            },
            LayoutNode::Split {
                direction: db,
                ratio: rb,
                first: fb,
                second: sb,
            },
        ) => {
            da == db
                && (ra - rb).abs() <= tolerance
                && same_tree(fa, fb, tolerance)
                && same_tree(sa, sb, tolerance)
        }
        _ => false,
    }
}

/// Whether two exports of a tab show the same thing: tree, ratios (up to
/// herdr's `f32` rounding), zoom, and the zoomed pane. Focus alone doesn't
/// count: the TUI moves it all the time.
pub fn same_layout(a: &TabLayout, b: &TabLayout) -> bool {
    same_tree(&a.root, &b.root, ECHO_TOLERANCE)
        && a.zoomed == b.zoomed
        && (!a.zoomed || a.focused_pane_id == b.focused_pane_id)
}

/// Whether `layout` already shows `target` closely enough that a push would
/// write nothing.
pub fn satisfies(layout: &TabLayout, target: &MirrorTarget) -> bool {
    target.tab.as_ref().is_none_or(|t| *t == layout.tab_id)
        && same_tree(&layout.root, &target.root, RATIO_TOLERANCE)
        && zoom_ok(layout, target.zoomed.as_ref())
}

fn zoom_ok(layout: &TabLayout, want: Option<&PaneId>) -> bool {
    // herdr doesn't zoom a tab's only pane.
    if matches!(layout.root, LayoutNode::Pane { .. }) {
        return true;
    }
    match want {
        Some(p) => layout.zoomed && layout.focused_pane_id.as_ref() == Some(p),
        None => !layout.zoomed,
    }
}

/// Every split's path from the root (`true` = second child) with its node.
fn splits(node: &LayoutNode) -> Vec<(Vec<bool>, SplitDirection, f64)> {
    fn walk(n: &LayoutNode, path: &mut Vec<bool>, out: &mut Vec<(Vec<bool>, SplitDirection, f64)>) {
        if let LayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } = n
        {
            out.push((path.clone(), *direction, *ratio));
            path.push(false);
            walk(first, path, out);
            path.pop();
            path.push(true);
            walk(second, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(node, &mut Vec::new(), &mut out);
    out
}

/// `node` with only the panes `keep` accepts; a split losing a side
/// collapses into the other.
pub fn prune_tree(node: &LayoutNode, keep: &dyn Fn(&PaneId) -> bool) -> Option<LayoutNode> {
    match node {
        LayoutNode::Pane { pane_id } => pane_id.as_ref().filter(|p| keep(p)).map(|_| node.clone()),
        LayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } => match (prune_tree(first, keep), prune_tree(second, keep)) {
            (Some(a), Some(b)) => Some(LayoutNode::split(*direction, *ratio, a, b)),
            (one, None) | (None, one) => one,
        },
    }
}

/// Whether the panes `herdr` and `target` both hold sit in another shape or
/// order in one than in the other: getting from one to the other takes a
/// swap or a rebuild, not just ratios and panes coming or going.
pub fn diverged(herdr: &LayoutNode, target: &LayoutNode) -> bool {
    let theirs = herdr.panes();
    let ours = target.panes();
    let both = |p: &PaneId| theirs.contains(&p) && ours.contains(&p);
    match (prune_tree(herdr, &both), prune_tree(target, &both)) {
        (Some(h), Some(t)) => !same_shape(&h, &t) || h.panes() != t.panes(),
        _ => false,
    }
}

fn owned_panes(node: &LayoutNode) -> Vec<PaneId> {
    node.panes().into_iter().cloned().collect()
}

/// Runs a push against `ops`.
pub async fn run<O: LayoutOps>(ops: &O, target: &MirrorTarget) -> Result<MirrorOutcome> {
    let mut push = Push {
        ops,
        writes: 0,
        renamed: HashMap::new(),
        moved: Vec::new(),
    };
    push.run(target).await
}

struct Push<'a, O> {
    ops: &'a O,
    writes: usize,
    /// Pane IDs as they are now, for panes a cross-workspace move renamed.
    renamed: HashMap<PaneId, PaneId>,
    moved: Vec<PaneInfo>,
}

impl<O: LayoutOps> Push<'_, O> {
    fn id(&self, pane: &PaneId) -> PaneId {
        self.renamed
            .get(pane)
            .cloned()
            .unwrap_or_else(|| pane.clone())
    }

    async fn run(&mut self, target: &MirrorTarget) -> Result<MirrorOutcome> {
        let want = owned_panes(&target.root);
        let anchor = want.first().cloned().expect("a tree has a pane");
        let all = self.ops.panes().await?;
        for p in &want {
            if !all.iter().any(|q| &q.pane_id == p) {
                return Err(MirrorError::Missing(p.clone()));
            }
        }
        let tab = match &target.tab {
            Some(t) => {
                if !all.iter().any(|q| &q.tab_id == t) {
                    return Err(MirrorError::TabGone(t.clone()));
                }
                t.clone()
            }
            None => {
                // Our own tab: the anchor's, if it's alone there; else the
                // anchor alone in a new one.
                let home = all
                    .iter()
                    .find(|q| q.pane_id == anchor)
                    .map(|q| q.tab_id.clone())
                    .expect("checked above");
                if all.iter().filter(|q| q.tab_id == home).count() == 1 {
                    home
                } else {
                    self.new_tab_for(&anchor, target.label.clone()).await?
                }
            }
        };
        self.finish(target, tab, &want, &anchor).await
    }

    async fn new_tab_for(&mut self, anchor: &PaneId, label: Option<String>) -> Result<TabId> {
        self.unzoom_pane_tab(anchor).await?;
        let moved = self
            .move_checked(
                anchor,
                &MoveTo::NewTab {
                    workspace: None,
                    label,
                },
            )
            .await?;
        Ok(moved.pane.tab_id)
    }

    async fn finish(
        &mut self,
        target: &MirrorTarget,
        tab: TabId,
        want: &[PaneId],
        anchor: &PaneId,
    ) -> Result<MirrorOutcome> {
        let anchor = anchor.clone();
        let mut cur = self.ops.export(&tab).await?;
        // Unknown panes: wait for the core to adopt them. Checked before
        // anything changes.
        let unknown: Vec<PaneId> = cur
            .root
            .panes()
            .into_iter()
            .filter(|p| !want.contains(p) && !target.known.contains(p))
            .cloned()
            .collect();
        if !unknown.is_empty() && !(target.park_unknown && target.restructure) {
            return Err(MirrorError::Unknown(unknown));
        }
        // herdr's arrangement of the panes both hold stays unless Azulejo
        // asked for another.
        if !target.restructure && diverged(&cur.root, &target.root) {
            return Err(MirrorError::Diverged(cur));
        }

        // The anchor first, so parking can't empty the tab.
        if !cur.root.panes().contains(&&anchor) {
            self.unzoom(&cur).await?;
            let beside = cur.root.first_pane().cloned();
            self.move_checked(
                &anchor,
                &MoveTo::Tab {
                    tab: tab.clone(),
                    split: SplitDirection::Right,
                    target: beside,
                    ratio: None,
                },
            )
            .await?;
            cur = self.ops.export(&tab).await?;
        }

        // Panes that aren't ours: out to a tab of their own.
        let extras: Vec<PaneId> = cur
            .root
            .panes()
            .into_iter()
            .filter(|p| !want.contains(&self.original(p)))
            .cloned()
            .collect();
        if !extras.is_empty() {
            self.unzoom(&cur).await?;
            for p in extras {
                self.move_checked(
                    &p,
                    &MoveTo::NewTab {
                        workspace: None,
                        label: None,
                    },
                )
                .await?;
            }
            cur = self.ops.export(&tab).await?;
        }

        let here: Vec<PaneId> = owned_panes(&cur.root)
            .iter()
            .map(|p| self.original(p))
            .collect();
        let complete = want.iter().all(|p| here.contains(p));
        if complete && same_shape(&cur.root, &target.root) {
            self.arrange(&tab, &cur, target).await?;
        } else {
            self.rebuild(&tab, &cur, target, &anchor).await?;
        }

        // Zoom last. What's recorded is herdr's own answer to the zoom, not a
        // later read: a change someone makes right after it isn't ours.
        let mut layout = self.ops.export(&tab).await?;
        let zoom = target.zoomed.as_ref().map(|z| self.id(z));
        if !zoom_ok(&layout, zoom.as_ref()) {
            let state = match zoom {
                Some(z) => self.zoom(&z, true).await?,
                None => {
                    let any = layout
                        .focused_pane_id
                        .clone()
                        .or_else(|| layout.root.first_pane().cloned())
                        .expect("a tab has a pane");
                    self.zoom(&any, false).await?
                }
            };
            layout.zoomed = state.zoomed;
            if state.focused.is_some() {
                layout.focused_pane_id = state.focused;
            }
        }
        Ok(MirrorOutcome {
            layout,
            moved: std::mem::take(&mut self.moved),
            writes: self.writes,
        })
    }

    /// The target's name for a pane herdr may have renamed.
    fn original(&self, now: &PaneId) -> PaneId {
        self.renamed
            .iter()
            .find(|(_, v)| *v == now)
            .map(|(k, _)| k.clone())
            .unwrap_or_else(|| now.clone())
    }

    /// Same shape: swap panes into place, then fix ratios.
    async fn arrange(&mut self, tab: &TabId, cur: &TabLayout, target: &MirrorTarget) -> Result<()> {
        let want: Vec<PaneId> = owned_panes(&target.root)
            .iter()
            .map(|p| self.id(p))
            .collect();
        let mut leaves = owned_panes(&cur.root);
        if leaves != want {
            self.unzoom(cur).await?;
        }
        for i in 0..want.len() {
            if leaves[i] == want[i] {
                continue;
            }
            let j = leaves
                .iter()
                .position(|p| *p == want[i])
                .expect("same panes");
            self.ops.swap(&leaves[i], &want[i]).await?;
            self.writes += 1;
            leaves.swap(i, j);
        }
        let have = splits(&cur.root);
        for ((path, _, want), (_, _, now)) in splits(&target.root).into_iter().zip(have) {
            if (want - now).abs() > RATIO_TOLERANCE {
                self.ops.set_ratio(tab, &path, want).await?;
                self.writes += 1;
            }
        }
        Ok(())
    }

    /// Another shape, or panes to bring in: everything but the anchor out to
    /// a staging tab, then back in place one by one.
    async fn rebuild(
        &mut self,
        tab: &TabId,
        cur: &TabLayout,
        target: &MirrorTarget,
        anchor: &PaneId,
    ) -> Result<()> {
        self.unzoom(cur).await?;
        let anchor = self.id(anchor);
        let mut staging: Option<(TabId, PaneId)> = None;
        for p in owned_panes(&cur.root) {
            if p == anchor {
                continue;
            }
            let to = match &staging {
                None => MoveTo::NewTab {
                    workspace: None,
                    label: None,
                },
                Some((st, last)) => MoveTo::Tab {
                    tab: st.clone(),
                    split: SplitDirection::Right,
                    target: Some(last.clone()),
                    ratio: None,
                },
            };
            let moved = self.move_checked(&p, &to).await?;
            staging = Some((moved.pane.tab_id.clone(), moved.pane.pane_id.clone()));
        }
        self.build(tab, &target.root, &anchor).await
    }

    /// Builds `node` in `tab` from `rep`, the pane standing for all of it.
    fn build<'b>(
        &'b mut self,
        tab: &'b TabId,
        node: &'b LayoutNode,
        rep: &'b PaneId,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<()>> + Send + 'b>>
    where
        O: Sync,
    {
        Box::pin(async move {
            let LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } = node
            else {
                return Ok(());
            };
            let q = second.first_pane().cloned().expect("a tree has a pane");
            let q = self.id(&q);
            let moved = self
                .move_checked(
                    &q,
                    &MoveTo::Tab {
                        tab: tab.clone(),
                        split: *direction,
                        target: Some(rep.clone()),
                        ratio: Some(*ratio),
                    },
                )
                .await?;
            let q = moved.pane.pane_id;
            self.build(tab, first, rep).await?;
            self.build(tab, second, &q).await
        })
    }

    /// A move that must happen. herdr refuses to move out of a zoomed tab:
    /// unzoom it and try once more.
    async fn move_checked(&mut self, pane: &PaneId, to: &MoveTo) -> Result<Moved> {
        let pane = self.id(pane);
        let mut moved = self.ops.move_pane(&pane, to).await?;
        if !moved.changed && moved.reason.as_deref() == Some("zoomed_tab") {
            self.zoom(&pane, false).await?;
            moved = self.ops.move_pane(&pane, to).await?;
        }
        if !moved.changed {
            return Err(MirrorError::Refused {
                pane,
                reason: moved.reason.unwrap_or_else(|| "unchanged".into()),
            });
        }
        self.writes += 1;
        if moved.pane.pane_id != pane {
            let original = self.original(&pane);
            self.renamed.insert(original, moved.pane.pane_id.clone());
        }
        self.moved
            .retain(|p| p.terminal_id != moved.pane.terminal_id);
        self.moved.push(moved.pane.clone());
        Ok(moved)
    }

    async fn unzoom(&mut self, cur: &TabLayout) -> Result<()> {
        if cur.zoomed {
            let any = cur
                .focused_pane_id
                .clone()
                .or_else(|| cur.root.first_pane().cloned())
                .expect("a tab has a pane");
            self.zoom(&any, false).await?;
        }
        Ok(())
    }

    /// Unzooms the tab `pane` is in, if it's zoomed.
    async fn unzoom_pane_tab(&mut self, pane: &PaneId) -> Result<()> {
        let all = self.ops.panes().await?;
        if let Some(p) = all.iter().find(|p| &p.pane_id == pane) {
            let layout = self.ops.export(&p.tab_id).await?;
            self.unzoom(&layout).await?;
        }
        Ok(())
    }

    async fn zoom(&mut self, pane: &PaneId, on: bool) -> Result<ZoomState> {
        let state = self.ops.zoom(pane, on).await?;
        self.writes += 1;
        Ok(state)
    }
}

#[cfg(test)]
#[path = "mirror_tests.rs"]
mod tests;
