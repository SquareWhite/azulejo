//! Turning any split tree a quarter turn (manual tabs' `alt+o`): four turns
//! are the identity, a turn back is the exact inverse, and the tiles' rects
//! turn as a picture does.

use azulejo_model::{Direction, MAX_RATIO, MIN_RATIO, Node, Rect, TileId};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum TreeOp {
    Split(usize, Direction),
    SplitAuto,
    Resize(usize, Direction, f64),
    Move(usize, Direction),
}

fn direction() -> impl Strategy<Value = Direction> {
    prop::sample::select(Direction::ALL.to_vec())
}

/// Trees the way people make them (splits, tall splits, resizes, moves),
/// then with any ratios at all written over some of them.
fn tree() -> impl Strategy<Value = Node> {
    (
        prop::collection::vec(
            prop_oneof![
                4 => (0usize..32, direction()).prop_map(|(t, d)| TreeOp::Split(t, d)),
                2 => Just(TreeOp::SplitAuto),
                2 => (0usize..32, direction(), -0.5f64..0.5)
                    .prop_map(|(t, d, a)| TreeOp::Resize(t, d, a)),
                1 => (0usize..32, direction()).prop_map(|(t, d)| TreeOp::Move(t, d)),
            ],
            0..24,
        ),
        prop::collection::vec(prop::option::of(MIN_RATIO..=MAX_RATIO), 0..24),
    )
        .prop_map(|(ops, ratios)| {
            let mut root = Node::leaf(TileId::new());
            for op in ops {
                let tiles = root.tiles();
                let pick = |i: usize| tiles[i % tiles.len()];
                match op {
                    TreeOp::Split(i, d) => root.split(pick(i), d, TileId::new()).unwrap(),
                    TreeOp::SplitAuto => root.split_auto(TileId::new()).unwrap(),
                    TreeOp::Resize(i, d, a) => {
                        let _ = root.resize(pick(i), d, a);
                    }
                    TreeOp::Move(i, d) => {
                        let _ = root.move_tile(pick(i), d);
                    }
                }
            }
            overwrite_ratios(&mut root, &mut ratios.into_iter());
            root
        })
}

fn overwrite_ratios(node: &mut Node, with: &mut impl Iterator<Item = Option<f64>>) {
    if let Node::Split { ratio, a, b, .. } = node {
        if let Some(Some(r)) = with.next() {
            *ratio = r;
        }
        overwrite_ratios(a, with);
        overwrite_ratios(b, with);
    }
}

fn turned(root: &Node, clockwise: bool, times: usize) -> Node {
    let mut t = root.clone();
    for _ in 0..times {
        if clockwise {
            t.rotate_clockwise();
        } else {
            t.rotate_anticlockwise();
        }
    }
    t
}

/// Where a rect goes when the unit square turns a quarter turn.
fn turn_rect(r: Rect, clockwise: bool) -> Rect {
    if clockwise {
        Rect {
            x: 1.0 - r.y - r.height,
            y: r.x,
            width: r.height,
            height: r.width,
        }
    } else {
        Rect {
            x: r.y,
            y: 1.0 - r.x - r.width,
            width: r.height,
            height: r.width,
        }
    }
}

fn close(a: Rect, b: Rect) -> bool {
    const EPS: f64 = 1e-9;
    (a.x - b.x).abs() < EPS
        && (a.y - b.y).abs() < EPS
        && (a.width - b.width).abs() < EPS
        && (a.height - b.height).abs() < EPS
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 10_000, ..ProptestConfig::default() })]

    /// Four turns either way, and a turn and a turn back, give the tree back
    /// bit for bit: exactly the input when its ratios are on the grid the
    /// turn works on, and within 2⁻⁵⁴ per ratio otherwise.
    #[test]
    fn four_turns_and_a_turn_back_are_the_identity(root in tree()) {
        let mut snapped = root.clone();
        snapped.snap_ratios();
        for (r, s) in root.ratios().iter().zip(snapped.ratios()) {
            prop_assert!((r - s).abs() <= f64::EPSILON / 4.0, "snapping moved {} to {}", r, s);
        }
        let mut again = snapped.clone();
        again.snap_ratios();
        prop_assert_eq!(&again, &snapped);

        prop_assert_eq!(&turned(&root, true, 4), &snapped);
        prop_assert_eq!(&turned(&root, false, 4), &snapped);
        prop_assert_eq!(&turned(&turned(&root, true, 1), false, 1), &snapped);
        prop_assert_eq!(&turned(&turned(&root, false, 1), true, 1), &snapped);
        // Two turns either way meet: upside down is upside down.
        prop_assert_eq!(turned(&root, true, 2), turned(&root, false, 2));
        // On the grid, the input itself comes back.
        prop_assert_eq!(&turned(&snapped, true, 4), &snapped);
        prop_assert_eq!(&turned(&turned(&snapped, true, 3), false, 3), &snapped);
    }

    /// Every tile's rect turns as the picture does, keeping its size with
    /// width and height swapped; the tiles and their count stay; ratios stay
    /// in range.
    #[test]
    fn tile_rects_turn_as_geometry(root in tree(), clockwise in any::<bool>(), times in 1usize..4) {
        let mut want: Vec<(TileId, Rect)> = root.rects();
        for _ in 0..times {
            for (_, r) in &mut want {
                *r = turn_rect(*r, clockwise);
            }
        }
        let after = turned(&root, clockwise, times);
        let got = after.rects();
        prop_assert_eq!(got.len(), want.len());
        for (tile, r) in &want {
            let g = got.iter().find(|(t, _)| t == tile).map(|(_, r)| *r);
            prop_assert!(g.is_some(), "tile {} lost", tile);
            let g = g.unwrap();
            prop_assert!(close(g, *r), "tile {}: got {:?}, want {:?}", tile, g, r);
        }
        let mut before_tiles = root.tiles();
        let mut after_tiles = after.tiles();
        before_tiles.sort();
        after_tiles.sort();
        prop_assert_eq!(before_tiles, after_tiles);
        // (A long tall stack's 1/n can start below the range; the turn
        // never takes a ratio out of it.)
        let in_range = |r: &f64| (MIN_RATIO..=MAX_RATIO).contains(r);
        if root.ratios().iter().all(in_range) {
            for r in after.ratios() {
                prop_assert!(in_range(&r), "ratio {} out of range", r);
            }
        }
    }
}

#[test]
fn a_lone_tile_has_nothing_to_turn() {
    let lone = Node::leaf(TileId::new());
    let mut t = lone.clone();
    assert!(!t.rotate_clockwise());
    assert!(!t.rotate_anticlockwise());
    assert_eq!(t, lone);
}
