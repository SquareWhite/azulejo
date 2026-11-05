//! Dragging a divider (`Session::set_split_ratio`, `layout.set_ratio`): which
//! split moves in a manual tree, how each dynamic kind turns the drag into
//! its parameters, the clamps, and evening a split out.

use azulejo_model::{
    Direction, LayoutKind, LayoutParams, Node, Orientation, Rect, Session, TabId, Tile,
    TileContent, TileId, TileParams,
};
use proptest::prelude::*;

const EPS: f64 = 1e-9;

/// A session with one tab of kind `kind` holding `n` terminals, made with
/// `split_auto` (a dynamic tab appends; a manual one builds the tall shape).
fn tab_of(kind: LayoutKind, n: usize, params: LayoutParams) -> (Session, TabId, Vec<TileId>) {
    let mut s = Session::new();
    s.prefs.default = kind;
    s.prefs.params = params;
    let (_, ws) = s.create_window("main", None);
    let first = Tile::new(TileContent::terminal());
    let mut tiles = vec![first.id];
    s.create_tab(ws, first, None).unwrap();
    let tab = s.focus().tab.unwrap();
    for _ in 1..n {
        let t = Tile::new(TileContent::terminal());
        tiles.push(t.id);
        s.split_auto(tab, t).unwrap();
    }
    (s, tab, tiles)
}

fn rect(s: &Session, tab: TabId, tile: TileId) -> Rect {
    s.tab(tab)
        .unwrap()
        .root
        .rects()
        .into_iter()
        .find(|(t, _)| *t == tile)
        .unwrap()
        .1
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn same_rect(a: Rect, b: Rect) -> bool {
    close(a.x, b.x) && close(a.y, b.y) && close(a.width, b.width) && close(a.height, b.height)
}

/// The shape of a tree without its ratios: axes and leaves.
fn shape(n: &Node) -> String {
    match n {
        Node::Leaf { id } => id.to_string(),
        Node::Split { axis, a, b, .. } => format!("{axis:?}({},{})", shape(a), shape(b)),
    }
}

#[test]
fn a_manual_split_takes_the_ratio_and_its_nested_splits_keep_theirs() {
    // a | (b / c), built by hand.
    let (mut s, tab, t) = tab_of(LayoutKind::Manual, 1, LayoutParams::default());
    let b = Tile::new(TileContent::terminal());
    let c = Tile::new(TileContent::terminal());
    let (bid, cid) = (b.id, c.id);
    s.split(t[0], Direction::Right, b).unwrap();
    s.split(bid, Direction::Down, c).unwrap();
    s.set_split_ratio(tab, &[true], Some(0.3)).unwrap();
    // The root's divider: a grows, b and c keep their 30/70 heights.
    assert!(s.set_split_ratio(tab, &[], Some(0.7)).unwrap());
    let root = &s.tab(tab).unwrap().root;
    assert_eq!(root.ratios(), vec![0.7, 0.3]);
    assert!(close(rect(&s, tab, bid).height, 0.3));
    assert!(close(rect(&s, tab, cid).x, 0.7));
    // The same value again changes nothing.
    assert!(!s.set_split_ratio(tab, &[], Some(0.7)).unwrap());
    // Clamped to 5–95 %.
    s.set_split_ratio(tab, &[true], Some(1.4)).unwrap();
    assert_eq!(s.tab(tab).unwrap().root.ratios(), vec![0.7, 0.95]);
    s.set_split_ratio(tab, &[], Some(-3.0)).unwrap();
    assert_eq!(s.tab(tab).unwrap().root.ratios(), vec![0.05, 0.95]);
    s.check_invariants().unwrap();
}

#[test]
fn bad_paths_and_values_are_refused_and_change_nothing() {
    let (mut s, tab, _) = tab_of(LayoutKind::Manual, 3, LayoutParams::default());
    let before = s.clone();
    // A leaf, past a leaf, a non-finite ratio.
    assert!(s.set_split_ratio(tab, &[false], Some(0.5)).is_err());
    assert!(s.set_split_ratio(tab, &[false, true], Some(0.5)).is_err());
    assert!(s.set_split_ratio(tab, &[], Some(f64::NAN)).is_err());
    assert!(s.set_split_ratio(TabId::new(), &[], Some(0.5)).is_err());
    assert_eq!(s, before);
    // A lone tile has no split at all.
    let (mut lone, tab, _) = tab_of(LayoutKind::Manual, 1, LayoutParams::default());
    assert!(lone.set_split_ratio(tab, &[], Some(0.5)).is_err());
    // Paths read as a/b steps.
    assert_eq!(azulejo_model::parse_path("ba").unwrap(), vec![true, false]);
    assert_eq!(azulejo_model::parse_path("").unwrap(), Vec::<bool>::new());
    assert!(azulejo_model::parse_path("bx").is_err());
}

#[test]
fn evening_a_manual_split_gives_every_tile_in_the_row_the_same_share() {
    // a | b | c as a | (b | c) with odd ratios, then evened out.
    let (mut s, tab, t) = tab_of(LayoutKind::Manual, 1, LayoutParams::default());
    let b = Tile::new(TileContent::terminal());
    let c = Tile::new(TileContent::terminal());
    let (bid, cid) = (b.id, c.id);
    s.split(t[0], Direction::Right, b).unwrap();
    s.split(bid, Direction::Right, c).unwrap();
    s.set_split_ratio(tab, &[], Some(0.8)).unwrap();
    s.set_split_ratio(tab, &[true], Some(0.2)).unwrap();
    assert!(s.set_split_ratio(tab, &[], None).unwrap());
    assert!(s.set_split_ratio(tab, &[true], None).unwrap());
    for id in [t[0], bid, cid] {
        assert!(close(rect(&s, tab, id).width, 1.0 / 3.0), "{id}");
    }
}

#[test]
fn tiles_master_edge_is_the_master_ratio() {
    let (mut s, tab, t) = tab_of(LayoutKind::Tile, 4, LayoutParams::default());
    // master | stack: the root split of the derived tree.
    let stack_before: Vec<Rect> = t[1..].iter().map(|x| rect(&s, tab, *x)).collect();
    assert!(s.set_split_ratio(tab, &[], Some(0.62)).unwrap());
    let l = &s.tab(tab).unwrap().layout;
    assert_eq!(l.kind, LayoutKind::Tile);
    assert!(close(l.params.tile.ratio, 0.62));
    assert!(l.weights.is_empty(), "stack sizes stay");
    for (x, before) in t[1..].iter().zip(stack_before) {
        let now = rect(&s, tab, *x);
        assert!(close(now.height, before.height));
        assert!(close(now.x, 0.62));
    }
    // Clamped to Tile's range.
    s.set_split_ratio(tab, &[], Some(0.95)).unwrap();
    assert!(close(s.tab(tab).unwrap().layout.params.tile.ratio, 0.8));
    s.set_split_ratio(tab, &[], Some(0.01)).unwrap();
    assert!(close(s.tab(tab).unwrap().layout.params.tile.ratio, 0.2));
    // Evened out: master area as wide as the stack.
    s.set_split_ratio(tab, &[], None).unwrap();
    assert!(close(s.tab(tab).unwrap().layout.params.tile.ratio, 0.5));
    s.check_invariants().unwrap();
}

#[test]
fn with_the_master_on_the_right_the_edge_still_follows_the_pointer() {
    let params = LayoutParams {
        tile: TileParams {
            orientation: Orientation::Right,
            ..TileParams::default()
        },
        ..LayoutParams::default()
    };
    let (mut s, tab, t) = tab_of(LayoutKind::Tile, 3, params);
    // stack | master: the divider at 0.3 gives the master 70 %.
    assert!(s.set_split_ratio(tab, &[], Some(0.3)).unwrap());
    assert!(close(s.tab(tab).unwrap().layout.params.tile.ratio, 0.7));
    assert!(close(rect(&s, tab, t[0]).x, 0.3));
}

#[test]
fn a_stack_edge_resizes_only_the_two_tiles_beside_it() {
    let (mut s, tab, t) = tab_of(LayoutKind::Tile, 4, LayoutParams::default());
    // Stack: t1, t2, t3, each a third. The derived stack is a chain
    // t1 / (t2 / t3) under the root's `b`; its first divider is at "b".
    let t3 = rect(&s, tab, t[3]);
    let (axis, _, area) = s.tab(tab).unwrap().root.split_at(&[true]).unwrap();
    assert_eq!(axis, azulejo_model::Axis::Vertical);
    assert!(close(area.height, 1.0));
    // Drag the t1|t2 divider down to y = 0.5.
    assert!(s.set_split_ratio(tab, &[true], Some(0.5)).unwrap());
    assert!(close(rect(&s, tab, t[1]).height, 0.5));
    assert!(close(rect(&s, tab, t[2]).y, 0.5));
    assert!(
        same_rect(rect(&s, tab, t[3]), t3),
        "a tile away from the divider keeps its size"
    );
    // The master doesn't move.
    assert!(close(rect(&s, tab, t[0]).width, 0.5));
    // Each tile keeps at least 5 % of the stack.
    s.set_split_ratio(tab, &[true], Some(0.95)).unwrap();
    assert!(rect(&s, tab, t[2]).height >= 0.05 - EPS);
    assert!(same_rect(rect(&s, tab, t[3]), t3));
    // Evened out: the two tiles beside it share their space equally.
    s.set_split_ratio(tab, &[true], None).unwrap();
    assert!(close(
        rect(&s, tab, t[1]).height,
        rect(&s, tab, t[2]).height
    ));
    assert!(same_rect(rect(&s, tab, t[3]), t3));
    s.check_invariants().unwrap();
}

#[test]
fn three_columns_keep_the_centre_centred() {
    let (mut s, tab, t) = tab_of(LayoutKind::ThreeColumn, 3, LayoutParams::default());
    // order: centre master t0, right t1, left t2. Default 2/7, 3/7, 2/7.
    let left = rect(&s, tab, t[2]);
    assert!(close(left.width, 2.0 / 7.0));
    // The left|centre divider is the root's; drag it to x = 0.2.
    assert!(s.set_split_ratio(tab, &[], Some(0.2)).unwrap());
    assert!(close(rect(&s, tab, t[2]).width, 0.2));
    assert!(
        close(rect(&s, tab, t[1]).width, 0.2),
        "the right column follows"
    );
    assert!(close(rect(&s, tab, t[0]).x, 0.2));
    // The centre|right divider sits in the root's `b`: drag it to x = 0.7,
    // which leaves 0.3 for each side column.
    let (_, ratio_b, area) = s.tab(tab).unwrap().root.split_at(&[true]).unwrap();
    assert!(close(area.x + area.width * ratio_b, 0.8));
    let r = (0.7 - area.x) / area.width;
    assert!(s.set_split_ratio(tab, &[true], Some(r)).unwrap());
    assert!(close(rect(&s, tab, t[1]).width, 0.3));
    assert!(close(rect(&s, tab, t[2]).width, 0.3));
    // Clamped to Three Column's range (centre weight 0.2–0.75).
    s.set_split_ratio(tab, &[], Some(0.45)).unwrap();
    assert!(close(
        s.tab(tab).unwrap().layout.params.three_column.ratio,
        0.2
    ));
    // Evened out: equal columns.
    s.set_split_ratio(tab, &[], None).unwrap();
    for x in &t {
        assert!(close(rect(&s, tab, *x).width, 1.0 / 3.0));
    }
    assert_eq!(s.tab(tab).unwrap().layout.kind, LayoutKind::ThreeColumn);
    s.check_invariants().unwrap();
}

#[test]
fn two_columns_take_the_ratio_as_it_is() {
    let (mut s, tab, t) = tab_of(LayoutKind::ThreeColumn, 2, LayoutParams::default());
    assert!(s.set_split_ratio(tab, &[], Some(0.4)).unwrap());
    assert!(close(rect(&s, tab, t[0]).width, 0.4));
    assert!(close(
        s.tab(tab).unwrap().layout.params.three_column.ratio,
        0.4
    ));
}

#[test]
fn lined_up_groups_move_together() {
    // Master on top, two masters side by side, two stack tiles side by side:
    // the vertical line at x = 0.5 runs through both groups.
    let params = LayoutParams {
        tile: TileParams {
            masters: 2,
            orientation: Orientation::Top,
            ..TileParams::default()
        },
        ..LayoutParams::default()
    };
    let (mut s, tab, t) = tab_of(LayoutKind::Tile, 4, params);
    let root = s.tab(tab).unwrap().root.clone();
    let Node::Split { axis, .. } = &root else {
        panic!()
    };
    assert_eq!(*axis, azulejo_model::Axis::Horizontal, "{}", shape(&root));
    assert!(s.set_split_ratio(tab, &[], Some(0.3)).unwrap());
    for x in [t[0], t[2]] {
        assert!(
            close(rect(&s, tab, x).width, 0.3),
            "{}",
            shape(&s.tab(tab).unwrap().root)
        );
    }
    for x in [t[1], t[3]] {
        assert!(close(rect(&s, tab, x).x, 0.3));
    }
    // The master edge, under the root's `a`: the ratio, for both halves.
    assert!(s.set_split_ratio(tab, &[false], Some(0.7)).unwrap());
    assert!(close(s.tab(tab).unwrap().layout.params.tile.ratio, 0.7));
    assert!(close(rect(&s, tab, t[3]).y, 0.7));
}

#[test]
fn monocle_has_no_dividers() {
    let (mut s, tab, _) = tab_of(LayoutKind::Monocle, 3, LayoutParams::default());
    let before = s.clone();
    assert!(s.set_split_ratio(tab, &[], Some(0.3)).is_err());
    assert_eq!(s, before);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2_000, ..ProptestConfig::default() })]

    /// Any drag of any divider, in any kind with dividers, keeps the
    /// invariants and never moves a tile to another place: whatever lay
    /// left of or above another tile still does, and every tile keeps a
    /// positive size. A manual tree keeps its shape and the split lands
    /// exactly on the clamped ratio. (A dynamic tab's derived tree can
    /// change shape when two edges come to line up, e.g. both held at the
    /// same clamp, as `freeze` cuts along the first line across the tab.)
    #[test]
    fn drags_keep_the_shape(
        kind in prop::sample::select(vec![LayoutKind::Manual, LayoutKind::Tile, LayoutKind::ThreeColumn]),
        n in 2usize..9,
        masters in 0u8..4,
        orientation in prop::sample::select(vec![
            Orientation::Left, Orientation::Top, Orientation::Right, Orientation::Bottom,
        ]),
        part in any::<bool>(),
        drags in prop::collection::vec((any::<u64>(), prop::option::of(-0.2f64..1.2)), 1..12),
    ) {
        let mut params = LayoutParams {
            tile: TileParams { masters, orientation, rotate_part: part, ..TileParams::default() },
            ..LayoutParams::default()
        };
        params.three_column.masters = masters.max(1);
        params.clamp();
        let (mut s, tab, _) = tab_of(kind, n, params);
        for (pick, ratio) in drags {
            let root = s.tab(tab).unwrap().root.clone();
            let mut splits = Vec::new();
            fn walk(n: &Node, path: &mut Vec<bool>, out: &mut Vec<Vec<bool>>) {
                if let Node::Split { a, b, .. } = n {
                    out.push(path.clone());
                    path.push(false);
                    walk(a, path, out);
                    path.pop();
                    path.push(true);
                    walk(b, path, out);
                    path.pop();
                }
            }
            walk(&root, &mut Vec::new(), &mut splits);
            let path = splits[(pick % splits.len() as u64) as usize].clone();
            let before = s.clone();
            match s.set_split_ratio(tab, &path, ratio) {
                Ok(_) => {}
                Err(e) => {
                    prop_assert_eq!(&s, &before, "refused ({}) but changed", e);
                    continue;
                }
            }
            prop_assert_eq!(s.check_invariants(), Ok(()));
            let after = s.tab(tab).unwrap();
            if kind == LayoutKind::Manual {
                prop_assert_eq!(shape(&after.root), shape(&root));
            }
            prop_assert_eq!(after.layout.kind, kind);
            let was = root.rects();
            let now = after.root.rects();
            let at = |rs: &[(TileId, Rect)], t: TileId| rs.iter().find(|(x, _)| *x == t).unwrap().1;
            // Neighbours (side by side, or one above the other, before and
            // after) keep their order.
            let overlap = |a0: f64, a1: f64, b0: f64, b1: f64| a1.min(b1) - a0.max(b0) > 1e-9;
            for (p, rp) in &was {
                for (q, rq) in &was {
                    let (np, nq) = (at(&now, *p), at(&now, *q));
                    if rp.right() <= rq.x + 1e-9
                        && overlap(rp.y, rp.bottom(), rq.y, rq.bottom())
                        && overlap(np.y, np.bottom(), nq.y, nq.bottom())
                    {
                        prop_assert!(np.right() <= nq.x + 1e-9, "{} no longer left of {}", p, q);
                    }
                    if rp.bottom() <= rq.y + 1e-9
                        && overlap(rp.x, rp.right(), rq.x, rq.right())
                        && overlap(np.x, np.right(), nq.x, nq.right())
                    {
                        prop_assert!(np.bottom() <= nq.y + 1e-9, "{} no longer above {}", p, q);
                    }
                }
            }
            for (_, r) in after.root.rects() {
                prop_assert!(r.width > 0.0 && r.height > 0.0);
            }
            if kind == LayoutKind::Manual
                && let Some(r) = ratio
            {
                let (_, now, _) = after.root.split_at(&path).unwrap();
                prop_assert_eq!(now, r.clamp(0.05, 0.95));
            }
        }
    }
}
