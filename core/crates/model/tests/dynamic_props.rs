//! ADR 0010's invariants for the dynamic layout rules.

use std::collections::BTreeMap;

use azulejo_model::{
    Direction, LayoutKind, LayoutParams, Node, Orientation, Rect, Session, Tile, TileContent,
    TileId, TileParams, arrange, freeze,
};
use proptest::prelude::*;

const EPS: f64 = 1e-6;

fn params() -> impl Strategy<Value = LayoutParams> {
    (
        0u8..14,
        -0.5f64..1.5,
        prop::sample::select(vec![
            Orientation::Left,
            Orientation::Top,
            Orientation::Right,
            Orientation::Bottom,
        ]),
        any::<bool>(),
        0u8..14,
        -0.5f64..1.5,
    )
        .prop_map(|(m, r, o, part, cm, cr)| {
            let mut p = LayoutParams {
                tile: TileParams {
                    masters: m,
                    ratio: r,
                    orientation: o,
                    rotate_part: part,
                },
                three_column: azulejo_model::ThreeColumnParams {
                    masters: cm,
                    ratio: cr,
                },
            };
            p.clamp();
            p
        })
}

fn setup(n: usize, weights: &[f64]) -> (Vec<TileId>, BTreeMap<TileId, f64>) {
    let order: Vec<TileId> = (0..n).map(|_| TileId::new()).collect();
    let w = order
        .iter()
        .zip(weights.iter().cycle())
        .map(|(t, w)| (*t, *w))
        .collect();
    (order, w)
}

fn area(r: &Rect) -> f64 {
    r.width * r.height
}

fn overlap(a: &Rect, b: &Rect) -> f64 {
    let w = (a.right().min(b.right()) - a.x.max(b.x)).max(0.0);
    let h = (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.0);
    w * h
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2_000, ..ProptestConfig::default() })]

    /// Every tile placed once, inside the square; guillotine kinds don't
    /// overlap and cover the square; the rule is deterministic; freezing
    /// reproduces the rects.
    #[test]
    fn arrange_places_every_tile_once(
        n in 1usize..=20,
        params in params(),
        weights in prop::collection::vec(0.1f64..5.0, 1..8),
        kind in prop::sample::select(LayoutKind::DYNAMIC.to_vec()),
        focus in 0usize..20,
    ) {
        let (order, w) = setup(n, &weights);
        let focused = order[focus % n];
        let p = arrange(kind, &params, &order, &w, Some(focused));
        prop_assert_eq!(p.len(), n);
        let mut seen: Vec<TileId> = p.iter().map(|x| x.tile).collect();
        seen.sort();
        let mut want = order.clone();
        want.sort();
        prop_assert_eq!(seen, want);
        for x in &p {
            let r = x.rect;
            prop_assert!(r.x >= -EPS && r.y >= -EPS && r.right() <= 1.0 + EPS && r.bottom() <= 1.0 + EPS);
            prop_assert!(r.width > 0.0 && r.height > 0.0);
        }
        prop_assert_eq!(&p, &arrange(kind, &params, &order, &w, Some(focused)));
        if kind == LayoutKind::Monocle {
            prop_assert_eq!(p.iter().filter(|x| x.visible).count(), 1);
            let top = p.iter().max_by_key(|x| x.z).unwrap();
            prop_assert_eq!(top.tile, focused);
            return Ok(());
        }
        let total: f64 = p.iter().map(|x| area(&x.rect)).sum();
        prop_assert!((total - 1.0).abs() < EPS, "cover {}", total);
        for (i, a) in p.iter().enumerate() {
            for b in &p[i + 1..] {
                prop_assert!(overlap(&a.rect, &b.rect) < EPS, "overlap");
            }
        }
        let rects: Vec<_> = p.iter().map(|x| (x.tile, x.rect)).collect();
        let tree = freeze(&rects).expect("guillotine");
        for (t, r) in tree.rects() {
            let want = rects.iter().find(|(id, _)| *id == t).unwrap().1;
            prop_assert!((r.x - want.x).abs() < EPS && (r.y - want.y).abs() < EPS);
            prop_assert!((r.width - want.width).abs() < EPS && (r.height - want.height).abs() < EPS);
        }
    }

    /// A canonical tall tree converts to Tile with no visible change, and
    /// switching back restores the exact tree.
    #[test]
    fn canonical_tall_converts_to_tile_and_back(
        stack in 1usize..8,
        grow in -0.25f64..0.25,
        rotate in any::<bool>(),
    ) {
        let mut s = Session::new();
        let (_, ws) = s.create_window("main", None);
        s.create_tab(ws, Tile::new(TileContent::terminal()), None).unwrap();
        let tab = s.focus().tab.unwrap();
        for _ in 0..stack {
            s.split_auto(tab, Tile::new(TileContent::terminal())).unwrap();
        }
        if rotate {
            s.rotate(tab).unwrap();
        }
        let master = s.tab(tab).unwrap().root.tall().master;
        let edge = if rotate { Direction::Down } else { Direction::Right };
        s.resize(master, edge, grow).unwrap();
        let before = s.tab(tab).unwrap().root.clone();
        prop_assert!(s.set_layout(tab, LayoutKind::Tile).unwrap());
        let t = s.tab(tab).unwrap();
        for (id, r) in before.rects() {
            let got = t.placements().into_iter().find(|p| p.tile == id).unwrap().rect;
            prop_assert!((r.x - got.x).abs() < EPS && (r.width - got.width).abs() < EPS, "jumped");
            prop_assert!((r.y - got.y).abs() < EPS && (r.height - got.height).abs() < EPS, "jumped");
        }
        prop_assert!(s.set_layout(tab, LayoutKind::Manual).unwrap());
        prop_assert_eq!(&s.tab(tab).unwrap().root, &before);
        prop_assert_eq!(s.check_invariants(), Ok(()));
    }

    /// Swapping one way and then back restores the order when the two tiles
    /// are each other's neighbours.
    #[test]
    fn swap_there_and_back(
        n in 2usize..10,
        params in params(),
        kind in prop::sample::select(LayoutKind::DYNAMIC.to_vec()),
        pick in 0usize..10,
        d in prop::sample::select(Direction::ALL.to_vec()),
    ) {
        let mut s = Session::new();
        s.prefs.default = kind;
        s.prefs.params = params;
        let (_, ws) = s.create_window("main", None);
        s.create_tab(ws, Tile::new(TileContent::terminal()), None).unwrap();
        let tab = s.focus().tab.unwrap();
        for _ in 1..n {
            s.split_auto(tab, Tile::new(TileContent::terminal())).unwrap();
        }
        let order = s.tab(tab).unwrap().layout.order.clone();
        let tile = order[pick % n];
        s.focus_tile(tile).unwrap();
        let t = s.tab(tab).unwrap();
        let Some(other) = t.neighbour(tile, d) else { return Ok(()); };
        // Mutual: from `other`, looking back, with `tile` as most recent.
        let mut probe = s.clone();
        probe.focus_tile(other).unwrap();
        probe.focus_tile(tile).unwrap();
        let mutual = probe.tab(tab).unwrap().neighbour(other, d.opposite()) == Some(tile);
        s.swap_direction(tile, d).unwrap();
        if mutual {
            s.focus_tile(tile).unwrap();
            let back = s.tab(tab).unwrap().neighbour(tile, d.opposite());
            if back == Some(other) {
                s.swap_direction(tile, d.opposite()).unwrap();
                prop_assert_eq!(&s.tab(tab).unwrap().layout.order, &order);
            }
        }
        prop_assert_eq!(s.check_invariants(), Ok(()));
    }
}

#[test]
fn old_state_loads_as_manual() {
    let json = r#"{"id":"tab_01JBX3Q7Y8Z9A0B1C2D3E4F5G6","root":{"type":"leaf","id":"tile_01JBX3Q7Y8Z9A0B1C2D3E4F5G7"},"tiles":[],"focused":"tile_01JBX3Q7Y8Z9A0B1C2D3E4F5G7"}"#;
    let tab: azulejo_model::Tab = serde_json::from_str(json).unwrap();
    assert_eq!(tab.layout.kind, LayoutKind::Manual);
    assert!(tab.layout.is_plain_manual());
    // And a plain manual tab writes no layout field.
    let out = serde_json::to_string(&tab).unwrap();
    assert!(!out.contains("layout"));
}

#[test]
fn monocle_focus_changes_only_z() {
    let mut s = Session::new();
    s.prefs.default = LayoutKind::Monocle;
    let (_, ws) = s.create_window("main", None);
    s.create_tab(ws, Tile::new(TileContent::terminal()), None)
        .unwrap();
    let tab = s.focus().tab.unwrap();
    for _ in 0..2 {
        s.split_auto(tab, Tile::new(TileContent::terminal()))
            .unwrap();
    }
    let before = s.tab(tab).unwrap().placements();
    let from = s.focused_tile().unwrap();
    let next = s.focus_direction(from, Direction::Right).unwrap();
    let after = s.tab(tab).unwrap().placements();
    for (a, b) in before.iter().zip(&after) {
        assert_eq!(a.tile, b.tile);
        assert_eq!(a.rect, b.rect, "monocle resized a tile on focus");
    }
    assert!(after.iter().any(|p| p.tile == next && p.visible));
    // Resizing does nothing in Monocle.
    s.resize(next, Direction::Left, 0.2).unwrap();
    assert_eq!(s.tab(tab).unwrap().placements(), after);
}

#[test]
fn dynamic_ops_map_as_in_the_adr() {
    let mut s = Session::new();
    s.prefs.default = LayoutKind::Tile;
    let (_, ws) = s.create_window("main", None);
    s.create_tab(ws, Tile::new(TileContent::terminal()), None)
        .unwrap();
    let tab = s.focus().tab.unwrap();
    let a = s.focused_tile().unwrap();
    // A plain split ignores its direction: the new tile goes to the end.
    let b = Tile::new(TileContent::terminal());
    let bid = b.id;
    s.split(a, Direction::Left, b).unwrap();
    let c = Tile::new(TileContent::terminal());
    let cid = c.id;
    s.split(a, Direction::Up, c).unwrap();
    assert_eq!(s.tab(tab).unwrap().layout.order, vec![a, bid, cid]);
    assert_eq!(s.focused_tile(), Some(cid));
    // Promote: to the front.
    assert!(s.promote(cid).unwrap());
    assert_eq!(s.tab(tab).unwrap().layout.order, vec![cid, a, bid]);
    // Move in order wraps.
    assert!(s.move_in_order(bid, 1).unwrap());
    assert_eq!(s.tab(tab).unwrap().layout.order, vec![bid, a, cid]);
    // Masters: two masters stacked on the left.
    assert!(s.change_masters(tab, 1).unwrap());
    assert!(
        !s.change_masters(tab, -5).unwrap() || s.tab(tab).unwrap().layout.params.tile.masters == 0
    );
    s.change_masters(tab, 2).unwrap();
    assert_eq!(s.tab(tab).unwrap().layout.params.tile.masters, 2);
    // Ratio is clamped.
    s.change_master_ratio(tab, 5.0).unwrap();
    assert_eq!(s.tab(tab).unwrap().layout.params.tile.ratio, 0.8);
    // Close: the tile now in that position takes focus.
    s.focus_tile(a).unwrap();
    s.close_tile(a).unwrap();
    assert_eq!(s.focused_tile(), Some(cid));
    // Switching to manual freezes, back to tile restores the order.
    assert!(s.set_layout(tab, LayoutKind::Manual).unwrap());
    assert_eq!(s.tab(tab).unwrap().layout.previous, Some(LayoutKind::Tile));
    assert!(s.toggle_layout(tab, LayoutKind::Manual).unwrap());
    assert_eq!(s.tab(tab).unwrap().layout.kind, LayoutKind::Tile);
    // Cycle walks tile → monocle → three_column → tile.
    s.cycle_layout(tab, 1).unwrap();
    assert_eq!(s.tab(tab).unwrap().layout.kind, LayoutKind::Monocle);
    s.cycle_layout(tab, 1).unwrap();
    assert_eq!(s.tab(tab).unwrap().layout.kind, LayoutKind::ThreeColumn);
    s.cycle_layout(tab, 1).unwrap();
    assert_eq!(s.tab(tab).unwrap().layout.kind, LayoutKind::Tile);
    s.check_invariants().unwrap();
    let _ = Node::leaf(a);
}
