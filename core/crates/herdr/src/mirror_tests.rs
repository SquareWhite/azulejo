//! The push against a simulated herdr that behaves as herdr 0.9.1 did in the
//! probe (docs/herdr-notes.md question 10): moves keep the pane, a move
//! inside one tab or out of a zoomed tab is refused, an emptied tab goes,
//! ratios are `f32`, a pane moved to another workspace gets a new ID.

use std::collections::BTreeMap;
use std::sync::Mutex;

use proptest::prelude::*;

use super::*;
use crate::types::{TerminalId, WorkspaceId};

#[derive(Debug, Clone)]
struct SimTab {
    workspace: String,
    root: LayoutNode,
    zoomed: bool,
    focused: PaneId,
}

#[derive(Debug, Default)]
struct SimState {
    tabs: BTreeMap<String, SimTab>,
    /// Terminal per pane; terminals never change.
    terminals: BTreeMap<String, String>,
    next_tab: u32,
    next_pane: u32,
    calls: Vec<String>,
}

#[derive(Debug, Default)]
struct Sim(Mutex<SimState>);

fn f32_ratio(r: f64) -> f64 {
    f64::from(r as f32)
}

fn not_found(what: &str) -> crate::Error {
    crate::Error::Server {
        code: format!("{what}_not_found"),
        message: format!("{what} not found"),
    }
}

/// `node` without `pane`; `None` when nothing is left.
fn remove(node: &LayoutNode, pane: &PaneId) -> Option<LayoutNode> {
    match node {
        LayoutNode::Pane { pane_id } => (pane_id.as_ref() != Some(pane)).then(|| node.clone()),
        LayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } => match (remove(first, pane), remove(second, pane)) {
            (Some(a), Some(b)) => Some(LayoutNode::split(*direction, *ratio, a, b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        },
    }
}

/// `node` with the leaf `target` replaced by `replace(target)`.
fn replace_leaf(
    node: &mut LayoutNode,
    target: &PaneId,
    with: &dyn Fn(LayoutNode) -> LayoutNode,
) -> bool {
    match node {
        LayoutNode::Pane { pane_id } if pane_id.as_ref() == Some(target) => {
            *node = with(node.clone());
            true
        }
        LayoutNode::Pane { .. } => false,
        LayoutNode::Split { first, second, .. } => {
            replace_leaf(first, target, with) || replace_leaf(second, target, with)
        }
    }
}

fn rename_leaf(node: &mut LayoutNode, from: &PaneId, to: &PaneId) {
    replace_leaf(node, from, &|_| LayoutNode::pane(to.as_str()));
}

impl SimState {
    fn tab_of(&self, pane: &PaneId) -> Option<String> {
        self.tabs
            .iter()
            .find(|(_, t)| t.root.panes().contains(&pane))
            .map(|(id, _)| id.clone())
    }

    fn new_tab_id(&mut self, workspace: &str) -> String {
        self.next_tab += 1;
        format!("{workspace}:t{}", 100 + self.next_tab)
    }

    fn info(&self, pane: &PaneId) -> PaneInfo {
        let tab = self.tab_of(pane).expect("pane exists");
        PaneInfo {
            pane_id: pane.clone(),
            terminal_id: TerminalId(self.terminals[pane.as_str()].clone()),
            workspace_id: WorkspaceId(self.tabs[&tab].workspace.clone()),
            tab_id: TabId(tab),
            focused: false,
            label: None,
            cwd: None,
            foreground_cwd: None,
            terminal_title: None,
            agent: None,
            agent_status: Default::default(),
            scroll: None,
            revision: 0,
        }
    }

    fn export(&self, tab: &str) -> crate::Result<TabLayout> {
        let t = self.tabs.get(tab).ok_or_else(|| not_found("tab"))?;
        Ok(TabLayout {
            workspace_id: WorkspaceId(t.workspace.clone()),
            tab_id: TabId(tab.to_owned()),
            zoomed: t.zoomed,
            focused_pane_id: Some(t.focused.clone()),
            root: t.root.clone(),
        })
    }

    /// Takes `pane` out of its tab; the tab goes when it empties.
    fn take(&mut self, pane: &PaneId) -> String {
        let tab = self.tab_of(pane).expect("pane exists");
        let t = self.tabs.get_mut(&tab).unwrap();
        match remove(&t.root, pane) {
            Some(root) => {
                if t.focused == *pane {
                    t.focused = root.first_pane().unwrap().clone();
                }
                t.root = root;
            }
            None => {
                self.tabs.remove(&tab);
            }
        }
        tab
    }

    /// A pane landing in `workspace` keeps its ID there, else gets a new one.
    fn land(&mut self, pane: &PaneId, from_ws: &str, to_ws: &str) -> PaneId {
        if from_ws == to_ws {
            return pane.clone();
        }
        self.next_pane += 1;
        let id = PaneId(format!("{to_ws}:p{}", 100 + self.next_pane));
        let term = self.terminals.remove(pane.as_str()).unwrap();
        self.terminals.insert(id.0.clone(), term);
        id
    }

    fn move_pane(&mut self, pane: &PaneId, to: &MoveTo) -> crate::Result<Moved> {
        self.calls.push(format!("move {pane} {to:?}"));
        let src = self.tab_of(pane).ok_or_else(|| not_found("pane"))?;
        let refuse = |s: &Self, reason: &str| {
            Ok(Moved {
                pane: s.info(pane),
                changed: false,
                reason: Some(reason.into()),
            })
        };
        if self.tabs[&src].zoomed {
            return refuse(self, "zoomed_tab");
        }
        let from_ws = self.tabs[&src].workspace.clone();
        match to {
            MoveTo::Tab {
                tab,
                split,
                target,
                ratio,
            } => {
                if tab.as_str() == src {
                    return refuse(self, "same_tab");
                }
                let dest = self
                    .tabs
                    .get(tab.as_str())
                    .ok_or_else(|| not_found("tab"))?;
                let to_ws = dest.workspace.clone();
                let target = target.clone().unwrap_or_else(|| dest.focused.clone());
                if !dest.root.panes().contains(&&target) {
                    return Err(not_found("pane"));
                }
                self.take(pane);
                let id = self.land(pane, &from_ws, &to_ws);
                let ratio = f32_ratio(ratio.unwrap_or(0.5));
                let dest = self.tabs.get_mut(tab.as_str()).unwrap();
                let (split, new) = (*split, id.clone());
                replace_leaf(&mut dest.root, &target, &|leaf| {
                    LayoutNode::split(split, ratio, leaf, LayoutNode::pane(new.as_str()))
                });
                dest.zoomed = false;
                Ok(Moved {
                    pane: self.info(&id),
                    changed: true,
                    reason: None,
                })
            }
            MoveTo::NewTab { workspace, .. } => {
                let to_ws = workspace
                    .as_ref()
                    .map(|w| w.0.clone())
                    .unwrap_or_else(|| from_ws.clone());
                self.take(pane);
                let id = self.land(pane, &from_ws, &to_ws);
                let tab = self.new_tab_id(&to_ws);
                self.tabs.insert(
                    tab,
                    SimTab {
                        workspace: to_ws,
                        root: LayoutNode::pane(id.as_str()),
                        zoomed: false,
                        focused: id.clone(),
                    },
                );
                Ok(Moved {
                    pane: self.info(&id),
                    changed: true,
                    reason: None,
                })
            }
        }
    }

    fn swap(&mut self, a: &PaneId, b: &PaneId) -> crate::Result<()> {
        self.calls.push(format!("swap {a} {b}"));
        let ta = self.tab_of(a).ok_or_else(|| not_found("pane"))?;
        if self.tab_of(b).as_ref() != Some(&ta) {
            return Err(crate::Error::Server {
                code: "invalid_request".into(),
                message: "panes in different tabs".into(),
            });
        }
        let t = self.tabs.get_mut(&ta).unwrap();
        let tmp = PaneId("__swap".into());
        rename_leaf(&mut t.root, a, &tmp);
        rename_leaf(&mut t.root, b, a);
        rename_leaf(&mut t.root, &tmp, b);
        Ok(())
    }

    fn set_ratio(&mut self, tab: &TabId, path: &[bool], ratio: f64) -> crate::Result<TabLayout> {
        self.calls.push(format!("ratio {tab} {path:?} {ratio}"));
        let t = self
            .tabs
            .get_mut(tab.as_str())
            .ok_or_else(|| not_found("tab"))?;
        let mut node = &mut t.root;
        for step in path {
            match node {
                LayoutNode::Split { first, second, .. } => {
                    node = if *step { second } else { first };
                }
                LayoutNode::Pane { .. } => return Err(not_found("split")),
            }
        }
        match node {
            LayoutNode::Split { ratio: r, .. } => *r = f32_ratio(ratio),
            LayoutNode::Pane { .. } => return Err(not_found("split")),
        }
        self.export(tab.as_str())
    }

    fn zoom(&mut self, pane: &PaneId, on: bool) -> crate::Result<ZoomState> {
        self.calls.push(format!("zoom {pane} {on}"));
        let tab = self.tab_of(pane).ok_or_else(|| not_found("pane"))?;
        let t = self.tabs.get_mut(&tab).unwrap();
        // herdr refuses to zoom a tab's only pane (`single_pane`).
        if !matches!(t.root, LayoutNode::Pane { .. }) {
            t.zoomed = on;
            if on {
                t.focused = pane.clone();
            }
        }
        Ok(ZoomState {
            zoomed: t.zoomed,
            focused: Some(t.focused.clone()),
        })
    }
}

impl LayoutOps for Sim {
    fn panes(&self) -> impl Future<Output = crate::Result<Vec<PaneInfo>>> + Send {
        let s = self.0.lock().unwrap();
        let out: Vec<PaneInfo> = s
            .tabs
            .values()
            .flat_map(|t| t.root.panes().into_iter().cloned().collect::<Vec<_>>())
            .map(|p| s.info(&p))
            .collect();
        async move { Ok(out) }
    }
    fn export(&self, tab: &TabId) -> impl Future<Output = crate::Result<TabLayout>> + Send {
        let r = self.0.lock().unwrap().export(tab.as_str());
        async move { r }
    }
    fn move_pane(
        &self,
        pane: &PaneId,
        to: &MoveTo,
    ) -> impl Future<Output = crate::Result<Moved>> + Send {
        let r = self.0.lock().unwrap().move_pane(pane, to);
        async move { r }
    }
    fn swap(&self, a: &PaneId, b: &PaneId) -> impl Future<Output = crate::Result<()>> + Send {
        let r = self.0.lock().unwrap().swap(a, b);
        async move { r }
    }
    fn set_ratio(
        &self,
        tab: &TabId,
        path: &[bool],
        ratio: f64,
    ) -> impl Future<Output = crate::Result<TabLayout>> + Send {
        let r = self.0.lock().unwrap().set_ratio(tab, path, ratio);
        async move { r }
    }
    fn zoom(
        &self,
        pane: &PaneId,
        on: bool,
    ) -> impl Future<Output = crate::Result<ZoomState>> + Send {
        let r = self.0.lock().unwrap().zoom(pane, on);
        async move { r }
    }
}

impl Sim {
    /// Tabs given as (workspace, tree); terminals named after their first
    /// pane IDs.
    fn new(tabs: Vec<(&str, LayoutNode)>) -> Sim {
        let mut s = SimState::default();
        for (i, (ws, root)) in tabs.into_iter().enumerate() {
            for p in root.panes() {
                s.terminals.insert(p.0.clone(), format!("term_{}", p.0));
            }
            let focused = root.first_pane().unwrap().clone();
            s.tabs.insert(
                format!("{ws}:t{}", i + 1),
                SimTab {
                    workspace: ws.into(),
                    root,
                    zoomed: false,
                    focused,
                },
            );
        }
        Sim(Mutex::new(s))
    }

    fn terminals(&self) -> Vec<String> {
        let mut t: Vec<String> = self.0.lock().unwrap().terminals.values().cloned().collect();
        t.sort();
        t
    }

    fn calls(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap().calls)
    }
}

fn p(id: &str) -> LayoutNode {
    LayoutNode::pane(id)
}

fn right(r: f64, a: LayoutNode, b: LayoutNode) -> LayoutNode {
    LayoutNode::split(SplitDirection::Right, r, a, b)
}

fn down(r: f64, a: LayoutNode, b: LayoutNode) -> LayoutNode {
    LayoutNode::split(SplitDirection::Down, r, a, b)
}

fn target(tab: Option<&str>, root: LayoutNode) -> MirrorTarget {
    MirrorTarget {
        tab: tab.map(TabId::from),
        root,
        zoomed: None,
        label: None,
        known: Vec::new(),
        park_unknown: false,
    }
}

fn block<T>(f: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn ratios_only_set_the_splits_that_are_off() {
    let sim = Sim::new(vec![(
        "w1",
        right(0.5, p("w1:p1"), down(0.5, p("w1:p2"), p("w1:p3"))),
    )]);
    let t = target(
        Some("w1:t1"),
        right(0.6, p("w1:p1"), down(0.502, p("w1:p2"), p("w1:p3"))),
    );
    let out = block(run(&sim, &t)).unwrap();
    assert_eq!(sim.calls(), ["ratio w1:t1 [] 0.6"]);
    assert_eq!(out.writes, 1);
    assert!(satisfies(&out.layout, &t));
    // Again: nothing to write.
    let out = block(run(&sim, &t)).unwrap();
    assert_eq!(out.writes, 0);
    assert!(sim.calls().is_empty());
}

#[test]
fn panes_in_each_others_places_are_swapped() {
    let sim = Sim::new(vec![(
        "w1",
        right(0.5, p("w1:p1"), down(0.5, p("w1:p2"), p("w1:p3"))),
    )]);
    let t = target(
        Some("w1:t1"),
        right(0.5, p("w1:p3"), down(0.5, p("w1:p1"), p("w1:p2"))),
    );
    let out = block(run(&sim, &t)).unwrap();
    assert!(sim.calls().iter().all(|c| c.starts_with("swap")));
    assert!(satisfies(&out.layout, &t));
}

#[test]
fn a_tab_per_tile_merges_into_one_tab_and_keeps_every_terminal() {
    // The old mapping (ADR 0002): one herdr tab per terminal tile.
    let sim = Sim::new(vec![
        ("w1", p("w1:p1")),
        ("w1", p("w1:p2")),
        ("w1", p("w1:p3")),
    ]);
    let before = sim.terminals();
    let t = target(
        Some("w1:t1"),
        right(0.5, p("w1:p1"), down(0.5, p("w1:p2"), p("w1:p3"))),
    );
    let out = block(run(&sim, &t)).unwrap();
    assert!(satisfies(&out.layout, &t), "{:?}", out.layout.root);
    assert_eq!(sim.terminals(), before, "no terminal lost");
    let s = sim.0.lock().unwrap();
    assert_eq!(
        s.tabs.len(),
        1,
        "emptied tabs are gone: {:?}",
        s.tabs.keys()
    );
    assert!(
        s.calls
            .iter()
            .all(|c| c.starts_with("move") || c.starts_with("ratio")),
        "{:?}",
        s.calls
    );
}

#[test]
fn a_different_shape_is_rebuilt_through_a_staging_tab() {
    // Tile turned from master-left to master-top.
    let sim = Sim::new(vec![(
        "w1",
        right(0.5, p("w1:p1"), down(0.5, p("w1:p2"), p("w1:p3"))),
    )]);
    let t = target(
        Some("w1:t1"),
        down(0.5, p("w1:p1"), right(0.5, p("w1:p2"), p("w1:p3"))),
    );
    let out = block(run(&sim, &t)).unwrap();
    assert!(satisfies(&out.layout, &t));
    assert_eq!(sim.0.lock().unwrap().tabs.len(), 1, "staging tab gone");
}

#[test]
fn zoom_goes_on_last_and_off_when_not_wanted() {
    let sim = Sim::new(vec![("w1", p("w1:p1")), ("w1", p("w1:p2"))]);
    let mut t = target(Some("w1:t1"), right(0.5, p("w1:p1"), p("w1:p2")));
    t.zoomed = Some("w1:p2".into());
    let out = block(run(&sim, &t)).unwrap();
    assert!(out.layout.zoomed);
    assert_eq!(out.layout.focused_pane_id, Some("w1:p2".into()));
    assert!(satisfies(&out.layout, &t));
    // A different shape while zoomed: herdr refuses moves out of a zoomed
    // tab, so the push unzooms, rebuilds, and zooms again.
    let mut t = target(Some("w1:t1"), down(0.3, p("w1:p2"), p("w1:p1")));
    t.zoomed = Some("w1:p1".into());
    let out = block(run(&sim, &t)).unwrap();
    assert!(satisfies(&out.layout, &t), "{out:?}");
    t.zoomed = None;
    let out = block(run(&sim, &t)).unwrap();
    assert!(!out.layout.zoomed);
}

#[test]
fn a_lone_pane_isnt_zoomed_and_thats_not_a_mismatch() {
    // herdr refuses (`single_pane`): a Monocle tab with one terminal and a
    // browser has nothing to zoom in herdr.
    let sim = Sim::new(vec![("w1", p("w1:p1"))]);
    let mut t = target(Some("w1:t1"), p("w1:p1"));
    t.zoomed = Some("w1:p1".into());
    let out = block(run(&sim, &t)).unwrap();
    assert_eq!(out.writes, 0, "{:?}", sim.calls());
    assert!(!out.layout.zoomed);
    assert!(satisfies(&out.layout, &t));
}

#[test]
fn unknown_panes_make_the_push_wait_and_known_ones_move_out() {
    let sim = Sim::new(vec![(
        "w1",
        right(0.5, p("w1:p1"), down(0.5, p("w1:p2"), p("w1:p9"))),
    )]);
    let mut t = target(Some("w1:t1"), right(0.5, p("w1:p1"), p("w1:p2")));
    let err = block(run(&sim, &t)).unwrap_err();
    assert!(matches!(err, MirrorError::Unknown(ref p) if p == &[PaneId::from("w1:p9")]));
    assert!(sim.calls().is_empty(), "nothing changed while waiting");
    // Closed here (detached): it moves out to a tab of its own.
    t.known = vec!["w1:p9".into()];
    let out = block(run(&sim, &t)).unwrap();
    assert!(satisfies(&out.layout, &t));
    let s = sim.0.lock().unwrap();
    assert_eq!(s.tabs.len(), 2);
    assert!(s.tab_of(&"w1:p9".into()).is_some(), "parked, not closed");
}

#[test]
fn without_a_tab_the_anchor_gets_one_of_its_own() {
    // Two Azulejo tabs once shared w1:t1; the other one keeps it.
    let sim = Sim::new(vec![(
        "w1",
        right(0.5, p("w1:p1"), down(0.5, p("w1:p2"), p("w1:p3"))),
    )]);
    let mut t = target(None, down(0.5, p("w1:p2"), p("w1:p3")));
    t.label = Some("logs".into());
    t.known = vec!["w1:p1".into()];
    let out = block(run(&sim, &t)).unwrap();
    assert_ne!(out.layout.tab_id.as_str(), "w1:t1");
    assert!(same_tree(&out.layout.root, &t.root, RATIO_TOLERANCE));
    assert_eq!(
        sim.0.lock().unwrap().export("w1:t1").unwrap().root,
        p("w1:p1")
    );
}

#[test]
fn panes_from_another_workspace_come_in_under_their_new_ids() {
    let sim = Sim::new(vec![("w1", p("w1:p1")), ("w2", p("w2:p1"))]);
    let t = target(Some("w1:t1"), right(0.4, p("w1:p1"), p("w2:p1")));
    let out = block(run(&sim, &t)).unwrap();
    let moved = &out.moved[0];
    assert_eq!(moved.terminal_id.as_str(), "term_w2:p1");
    assert_eq!(moved.workspace_id.as_str(), "w1");
    let leaves: Vec<&str> = out.layout.root.panes().iter().map(|p| p.as_str()).collect();
    assert_eq!(leaves, ["w1:p1", moved.pane_id.as_str()]);
}

#[test]
fn a_missing_pane_or_tab_fails_before_changing_anything() {
    let sim = Sim::new(vec![("w1", p("w1:p1"))]);
    let err = block(run(
        &sim,
        &target(Some("w1:t1"), right(0.5, p("w1:p1"), p("w1:p7"))),
    ))
    .unwrap_err();
    assert!(matches!(err, MirrorError::Missing(_)));
    let err = block(run(&sim, &target(Some("w1:t9"), p("w1:p1")))).unwrap_err();
    assert!(matches!(err, MirrorError::TabGone(_)));
    assert!(sim.calls().is_empty());
}

#[test]
fn echo_comparison_allows_f32_rounding_but_not_a_cell() {
    let a = TabLayout {
        workspace_id: "w1".into(),
        tab_id: "w1:t1".into(),
        zoomed: false,
        focused_pane_id: Some("w1:p1".into()),
        root: right(1.0 / 3.0, p("w1:p1"), p("w1:p2")),
    };
    let mut b = a.clone();
    b.root = right(f32_ratio(1.0 / 3.0), p("w1:p1"), p("w1:p2"));
    b.focused_pane_id = Some("w1:p2".into());
    assert!(same_layout(&a, &b), "f32 rounding and focus don't count");
    b.root = right(0.35, p("w1:p1"), p("w1:p2"));
    assert!(!same_layout(&a, &b));
    b.root = a.root.clone();
    b.zoomed = true;
    assert!(!same_layout(&a, &b));
}

// ---- property: any start, any target → the target, no pane lost ---------

/// A random tree over `panes` (in order).
fn tree(panes: Vec<String>) -> BoxedStrategy<LayoutNode> {
    if panes.len() == 1 {
        return Just(p(&panes[0])).boxed();
    }
    let n = panes.len();
    (1..n, any::<bool>(), 0.1f64..0.9)
        .prop_flat_map(move |(cut, right_dir, ratio)| {
            let (a, b) = panes.split_at(cut);
            let dir = if right_dir {
                SplitDirection::Right
            } else {
                SplitDirection::Down
            };
            (tree(a.to_vec()), tree(b.to_vec()))
                .prop_map(move |(a, b)| LayoutNode::split(dir, ratio, a, b))
        })
        .boxed()
}

/// A server: panes p1..pn spread over 1..=3 tabs in one workspace, some
/// zoomed; a target over a non-empty subset in a random order, mirrored into
/// one of the tabs or none, maybe zoomed.
fn scenario() -> impl Strategy<Value = (Vec<LayoutNode>, Vec<bool>, MirrorTarget)> {
    (1usize..=7, 1usize..=3)
        .prop_flat_map(|(n, tabs)| {
            let tabs = tabs.min(n);
            let panes: Vec<String> = (1..=n).map(|i| format!("w1:p{i}")).collect();
            (
                Just(panes.clone()),
                proptest::sample::subsequence((1..n).collect::<Vec<_>>(), tabs - 1),
                Just(panes.clone()).prop_shuffle(),
                proptest::collection::vec(any::<bool>(), tabs),
                1..=n,
                0..=tabs,
                any::<bool>(),
            )
        })
        .prop_flat_map(|(panes, cuts, shuffled, zooms, take, tab, zoom)| {
            let mut groups = Vec::new();
            let mut start = 0;
            for c in cuts.iter().copied().chain(std::iter::once(panes.len())) {
                groups.push(panes[start..c].to_vec());
                start = c;
            }
            let start_trees: Vec<BoxedStrategy<LayoutNode>> =
                groups.into_iter().map(tree).collect();
            let chosen: Vec<String> = shuffled[..take].to_vec();
            let zoom_pane = zoom.then(|| chosen[0].clone());
            (
                start_trees,
                Just(zooms),
                tree(chosen),
                Just(tab),
                Just(zoom_pane),
                Just(panes),
            )
        })
        .prop_map(|(start, zooms, root, tab, zoom, panes)| {
            let want: Vec<PaneId> = owned_panes(&root);
            let known = panes
                .iter()
                .map(|p| PaneId(p.clone()))
                .filter(|p| !want.contains(p))
                .collect();
            let t = MirrorTarget {
                tab: (tab > 0 && tab <= start.len()).then(|| TabId(format!("w1:t{tab}"))),
                root,
                zoomed: zoom.map(PaneId),
                label: None,
                known,
                park_unknown: false,
            };
            (start, zooms, t)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]
    #[test]
    fn any_layout_reaches_its_target_without_losing_a_pane((start, zooms, t) in scenario()) {
        let sim = Sim::new(start.into_iter().map(|r| ("w1", r)).collect());
        {
            let mut s = sim.0.lock().unwrap();
            for (tab, z) in s.tabs.values_mut().zip(zooms) {
                // herdr never zooms a tab's only pane.
                tab.zoomed = z && !matches!(tab.root, LayoutNode::Pane { .. });
            }
        }
        let before = sim.terminals();
        let out = block(run(&sim, &t)).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(same_tree(&out.layout.root, &t.root, RATIO_TOLERANCE),
            "got {:?}\nwant {:?}", out.layout.root, t.root);
        prop_assert!(zoom_ok(&out.layout, t.zoomed.as_ref()));
        if let Some(tab) = &t.tab {
            prop_assert_eq!(&out.layout.tab_id, tab);
        }
        prop_assert_eq!(sim.terminals(), before);
        // Every pane not in the target sits in some other tab.
        let s = sim.0.lock().unwrap();
        for p in &t.known {
            let home = s.tab_of(p);
            prop_assert!(home.is_some());
            prop_assert_ne!(home.unwrap(), out.layout.tab_id.0.clone());
        }
        drop(s);
        // A second push writes nothing.
        let again = block(run(&sim, &MirrorTarget { tab: Some(out.layout.tab_id.clone()), ..t.clone() }))
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(again.writes, 0, "{:?}", sim.calls());
    }
}
