//! ADR 0012: reading a dynamic layout back from an arrangement made in herdr.

use std::collections::BTreeMap;

use azulejo_model::{
    Axis, Direction, FIT_TOLERANCE, LayoutKind, LayoutParams, Node, Orientation, Rect, Session,
    TabLayout, ThreeColumnParams, Tile, TileContent, TileId, TileParams, arrange, fit,
};
use proptest::prelude::*;

fn params() -> impl Strategy<Value = LayoutParams> {
    (
        0u8..5,
        0.2f64..0.8,
        prop::sample::select(vec![
            Orientation::Left,
            Orientation::Top,
            Orientation::Right,
            Orientation::Bottom,
        ]),
        any::<bool>(),
        1u8..4,
        0.2f64..0.75,
    )
        .prop_map(|(m, r, o, part, cm, cr)| {
            let mut p = LayoutParams {
                tile: TileParams {
                    masters: m,
                    ratio: r,
                    orientation: o,
                    rotate_part: part,
                },
                three_column: ThreeColumnParams {
                    masters: cm,
                    ratio: cr,
                },
            };
            p.clamp();
            p
        })
}

fn layout(kind: LayoutKind, params: LayoutParams, n: usize, weights: &[f64]) -> TabLayout {
    let order: Vec<TileId> = (0..n).map(|_| TileId::new()).collect();
    let weights: BTreeMap<TileId, f64> = order
        .iter()
        .zip(weights.iter().cycle())
        .filter(|(_, w)| (**w - 1.0).abs() > 1e-9)
        .map(|(t, w)| (*t, *w))
        .collect();
    TabLayout {
        kind,
        order,
        weights,
        params,
        ..TabLayout::default()
    }
}

fn rects(l: &TabLayout) -> Vec<(TileId, Rect)> {
    arrange(l.kind, &l.params, &l.order, &l.weights, None)
        .into_iter()
        .map(|p| (p.tile, p.rect))
        .collect()
}

fn close(a: &[(TileId, Rect)], b: &[(TileId, Rect)]) -> bool {
    a.len() == b.len()
        && a.iter().all(|(t, r)| {
            b.iter().any(|(u, s)| {
                t == u
                    && (r.x - s.x).abs() <= FIT_TOLERANCE
                    && (r.y - s.y).abs() <= FIT_TOLERANCE
                    && (r.width - s.width).abs() <= FIT_TOLERANCE
                    && (r.height - s.height).abs() <= FIT_TOLERANCE
            })
        })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2_000, ..ProptestConfig::default() })]

    /// A layout reads back from its own rects: same order, same rects.
    #[test]
    fn a_layout_fits_its_own_arrangement(
        n in 1usize..=9,
        params in params(),
        weights in prop::collection::vec(0.5f64..2.0, 1..5),
        kind in prop::sample::select(vec![LayoutKind::Tile, LayoutKind::ThreeColumn]),
    ) {
        let l = layout(kind, params, n, &weights);
        let r = rects(&l);
        let fitted = fit(&l, &r).expect("its own rects fit");
        prop_assert_eq!(&fitted.order, &l.order);
        prop_assert!(close(&rects(&fitted), &r));
    }

    /// Two tiles trading places is a swap in the order.
    #[test]
    fn traded_places_are_a_swap(
        n in 2usize..=8,
        params in params(),
        kind in prop::sample::select(vec![LayoutKind::Tile, LayoutKind::ThreeColumn]),
        i in 0usize..8,
        j in 0usize..8,
    ) {
        let l = layout(kind, params, n, &[1.0]);
        let (i, j) = (i % n, j % n);
        prop_assume!(i != j);
        let mut r = rects(&l);
        let (ri, rj) = (r[i].1, r[j].1);
        r[i].1 = rj;
        r[j].1 = ri;
        let fitted = fit(&l, &r).expect("a swap fits");
        let mut want = l.order.clone();
        let pos = |t: TileId| l.order.iter().position(|u| *u == t).unwrap();
        want.swap(pos(r[i].0), pos(r[j].0));
        prop_assert_eq!(fitted.order, want);
    }
}

fn tile_layout(n: usize) -> TabLayout {
    layout(LayoutKind::Tile, LayoutParams::default(), n, &[1.0])
}

#[test]
fn a_moved_master_edge_is_the_master_ratio() {
    let l = tile_layout(3);
    let mut r = rects(&l);
    // Master on the left, 0.5 → 0.62.
    for (i, (_, rect)) in r.iter_mut().enumerate() {
        if i == 0 {
            rect.width = 0.62;
        } else {
            rect.x = 0.62;
            rect.width = 0.38;
        }
    }
    let fitted = fit(&l, &r).unwrap();
    assert!((fitted.params.tile.ratio - 0.62).abs() < 1e-9);
    assert!(fitted.weights.is_empty());
}

#[test]
fn a_moved_stack_edge_is_stack_weights() {
    let l = tile_layout(3);
    let mut r = rects(&l);
    r[1].1.height = 0.7;
    r[2].1.y = 0.7;
    r[2].1.height = 0.3;
    let fitted = fit(&l, &r).unwrap();
    let w1 = fitted.weights[&l.order[1]];
    let w2 = fitted.weights[&l.order[2]];
    assert!((w1 / w2 - 0.7 / 0.3).abs() < 1e-9);
    assert!(close(&rects(&fitted), &r));
}

#[test]
fn a_shape_the_rule_cant_make_doesnt_fit() {
    // Three tiles side by side: Tile with one master can't do that.
    let l = tile_layout(3);
    let r: Vec<(TileId, Rect)> = l
        .order
        .iter()
        .enumerate()
        .map(|(i, t)| {
            (
                *t,
                Rect {
                    x: i as f64 / 3.0,
                    y: 0.0,
                    width: 1.0 / 3.0,
                    height: 1.0,
                },
            )
        })
        .collect();
    assert!(fit(&l, &r).is_none());
}

/// A Tile tab with `n` terminals, as the session makes it.
fn session_with_tile_tab(n: usize) -> (Session, azulejo_model::TabId, Vec<TileId>) {
    let mut s = Session::new();
    s.prefs.default = LayoutKind::Tile;
    let (_, ws) = s.create_window("main", None);
    s.create_tab(ws, Tile::new(TileContent::terminal()), None)
        .unwrap();
    let tab = s.focus().tab.unwrap();
    let first = s.focused_tile().unwrap();
    for _ in 1..n {
        s.split(first, Direction::Right, Tile::new(TileContent::terminal()))
            .unwrap();
    }
    let order = s.tab(tab).unwrap().layout.order.clone();
    (s, tab, order)
}

#[test]
fn apply_tree_absorbs_what_the_rule_can_make_and_freezes_otherwise() {
    let (mut s, tab, order) = session_with_tile_tab(3);
    // The master ratio moved in herdr: absorbed, still Tile.
    let mut root = s.tab(tab).unwrap().root.clone();
    if let Node::Split { ratio, .. } = &mut root {
        *ratio = 0.6;
    }
    assert!(s.apply_tree(tab, root.clone()).unwrap());
    let t = s.tab(tab).unwrap();
    assert_eq!(t.layout.kind, LayoutKind::Tile);
    assert!((t.layout.params.tile.ratio - 0.6).abs() < 1e-9);
    assert!(!s.apply_tree(tab, root).unwrap(), "same again: no change");

    // Three in a row: Tile can't, so the tab freezes to that tree.
    let row = Node::Split {
        axis: Axis::Horizontal,
        ratio: 1.0 / 3.0,
        a: Box::new(Node::leaf(order[0])),
        b: Box::new(Node::Split {
            axis: Axis::Horizontal,
            ratio: 0.5,
            a: Box::new(Node::leaf(order[1])),
            b: Box::new(Node::leaf(order[2])),
        }),
    };
    assert!(s.apply_tree(tab, row.clone()).unwrap());
    let t = s.tab(tab).unwrap();
    assert_eq!(t.layout.kind, LayoutKind::Manual);
    assert_eq!(t.layout.previous, Some(LayoutKind::Tile));
    assert_eq!(t.root, row);
    s.check_invariants().unwrap();
    // Toggling back gives Tile again.
    s.toggle_layout(tab, LayoutKind::Manual).unwrap();
    assert_eq!(s.tab(tab).unwrap().layout.kind, LayoutKind::Tile);
    s.check_invariants().unwrap();

    // A tree with other tiles is refused.
    let stray = Node::leaf(TileId::new());
    assert!(s.apply_tree(tab, stray).is_err());
}

#[test]
fn monocle_takes_only_the_order() {
    let (mut s, tab, order) = session_with_tile_tab(3);
    s.set_layout(tab, LayoutKind::Monocle).unwrap();
    let root = s.tab(tab).unwrap().root.clone();
    // Master and the last stack tile trade places.
    let swapped = {
        let mut r = root.clone();
        swap_leaves(&mut r, order[0], order[2]);
        r
    };
    assert!(s.apply_tree(tab, swapped).unwrap());
    let t = s.tab(tab).unwrap();
    assert_eq!(t.layout.kind, LayoutKind::Monocle);
    assert_eq!(t.layout.order, vec![order[2], order[1], order[0]]);
    s.check_invariants().unwrap();
}

fn swap_leaves(node: &mut Node, a: TileId, b: TileId) {
    match node {
        Node::Leaf { id } if *id == a => *id = b,
        Node::Leaf { id } if *id == b => *id = a,
        Node::Leaf { .. } => {}
        Node::Split { a: x, b: y, .. } => {
            swap_leaves(x, a, b);
            swap_leaves(y, a, b);
        }
    }
}
