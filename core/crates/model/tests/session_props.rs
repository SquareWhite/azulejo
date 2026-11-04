//! Random op sequences against the session. After every op the invariants must
//! hold, and a refused op must leave the session exactly as it was.

use azulejo_model::{Direction, InsertAt, LayoutKind, Session, Tile, TileContent, TileId};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Op {
    CreateWindow,
    CloseWindow(usize),
    FocusWindow(usize),
    CreateWorkspace(usize),
    CloseWorkspace(usize),
    FocusWorkspace(usize),
    CreateTab(usize),
    CloseTab(usize),
    FocusTab(usize),
    Split(usize, Direction),
    CloseTile(usize),
    FocusTile(usize),
    FocusDirection(Direction),
    SwapDirection(usize, Direction),
    SwapTiles(usize, usize),
    Move(usize, Direction),
    Resize(usize, Direction, f64),
    Zoom(usize, Option<bool>),
    CloseMissingTile,
    MoveTab(usize, usize),
    MoveWorkspace(usize, usize),
    SplitAuto(usize),
    Promote(usize),
    Flip(usize),
    Equalize(usize),
    SetLayout(usize, LayoutKind),
    ToggleLayout(usize, LayoutKind),
    CycleLayout(usize, i32),
    Masters(usize, i32),
    MasterRatio(usize, f64),
    MoveOrder(usize, i32),
    RotateTurn(usize, bool),
    RotatePart(usize),
}

fn kind() -> impl Strategy<Value = LayoutKind> {
    prop::sample::select(vec![
        LayoutKind::Manual,
        LayoutKind::Tile,
        LayoutKind::Monocle,
        LayoutKind::ThreeColumn,
    ])
}

fn direction() -> impl Strategy<Value = Direction> {
    prop::sample::select(Direction::ALL.to_vec())
}

fn op() -> impl Strategy<Value = Op> {
    let pick = 0usize..64;
    prop_oneof![
        1 => Just(Op::CreateWindow),
        1 => pick.clone().prop_map(Op::CloseWindow),
        1 => pick.clone().prop_map(Op::FocusWindow),
        2 => pick.clone().prop_map(Op::CreateWorkspace),
        1 => pick.clone().prop_map(Op::CloseWorkspace),
        1 => pick.clone().prop_map(Op::FocusWorkspace),
        3 => pick.clone().prop_map(Op::CreateTab),
        1 => pick.clone().prop_map(Op::CloseTab),
        1 => pick.clone().prop_map(Op::FocusTab),
        8 => (pick.clone(), direction()).prop_map(|(t, d)| Op::Split(t, d)),
        3 => pick.clone().prop_map(Op::CloseTile),
        2 => pick.clone().prop_map(Op::FocusTile),
        3 => direction().prop_map(Op::FocusDirection),
        2 => (pick.clone(), direction()).prop_map(|(t, d)| Op::SwapDirection(t, d)),
        1 => (pick.clone(), pick.clone()).prop_map(|(a, b)| Op::SwapTiles(a, b)),
        3 => (pick.clone(), direction()).prop_map(|(t, d)| Op::Move(t, d)),
        3 => (pick.clone(), direction(), -1.5f64..1.5).prop_map(|(t, d, a)| Op::Resize(t, d, a)),
        2 => (pick, prop::option::of(any::<bool>())).prop_map(|(t, z)| Op::Zoom(t, z)),
        1 => Just(Op::CloseMissingTile),
        1 => (0usize..64, 0usize..8).prop_map(|(t, i)| Op::MoveTab(t, i)),
        1 => (0usize..64, 0usize..8).prop_map(|(w, i)| Op::MoveWorkspace(w, i)),
        6 => (0usize..64).prop_map(Op::SplitAuto),
        2 => (0usize..64).prop_map(Op::Promote),
        2 => (0usize..64).prop_map(Op::Flip),
        2 => (0usize..64).prop_map(Op::Equalize),
        3 => (0usize..64, kind()).prop_map(|(t, k)| Op::SetLayout(t, k)),
        1 => (0usize..64, kind()).prop_map(|(t, k)| Op::ToggleLayout(t, k)),
        1 => (0usize..64, -2i32..3).prop_map(|(t, s)| Op::CycleLayout(t, s)),
        2 => (0usize..64, -3i32..4).prop_map(|(t, d)| Op::Masters(t, d)),
        2 => (0usize..64, -0.7f64..0.7).prop_map(|(t, d)| Op::MasterRatio(t, d)),
        2 => (0usize..64, -3i32..4).prop_map(|(t, s)| Op::MoveOrder(t, s)),
        3 => (0usize..64, any::<bool>()).prop_map(|(t, r)| Op::RotateTurn(t, r)),
        1 => (0usize..64).prop_map(Op::RotatePart),
    ]
}

fn nth<T: Copy>(items: &[T], i: usize) -> Option<T> {
    (!items.is_empty()).then(|| items[i % items.len()])
}

fn apply(s: &mut Session, op: &Op) -> Result<(), azulejo_model::ModelError> {
    let windows: Vec<_> = s.windows.iter().map(|w| w.id).collect();
    let workspaces: Vec<_> = s
        .windows
        .iter()
        .flat_map(|w| w.workspaces.iter().map(|ws| ws.id))
        .collect();
    let tabs: Vec<_> = s
        .windows
        .iter()
        .flat_map(|w| {
            w.workspaces
                .iter()
                .flat_map(|ws| ws.tabs.iter().map(|t| t.id))
        })
        .collect();
    let tiles: Vec<TileId> = s.tiles().map(|(_, t)| t.id).collect();
    let terminal = || Tile::new(TileContent::terminal());

    match *op {
        Op::CreateWindow => {
            s.create_window("ws", None);
        }
        Op::CloseWindow(i) => {
            if let Some(id) = nth(&windows, i) {
                s.close_window(id)?;
            }
        }
        Op::FocusWindow(i) => {
            if let Some(id) = nth(&windows, i) {
                s.focus_window(id)?;
            }
        }
        Op::CreateWorkspace(i) => {
            if let Some(id) = nth(&windows, i) {
                s.create_workspace(id, "ws", None)?;
            }
        }
        Op::CloseWorkspace(i) => {
            if let Some(id) = nth(&workspaces, i) {
                s.close_workspace(id)?;
            }
        }
        Op::FocusWorkspace(i) => {
            if let Some(id) = nth(&workspaces, i) {
                s.focus_workspace(id)?;
            }
        }
        Op::CreateTab(i) => {
            if let Some(id) = nth(&workspaces, i) {
                s.create_tab(id, terminal(), None)?;
            }
        }
        Op::CloseTab(i) => {
            if let Some(id) = nth(&tabs, i) {
                s.close_tab(id)?;
            }
        }
        Op::FocusTab(i) => {
            if let Some(id) = nth(&tabs, i) {
                s.focus_tab(id)?;
            }
        }
        Op::Split(i, d) => {
            if let Some(id) = nth(&tiles, i) {
                s.split(id, d, terminal())?;
            }
        }
        Op::CloseTile(i) => {
            if let Some(id) = nth(&tiles, i) {
                s.close_tile(id)?;
            }
        }
        Op::FocusTile(i) => {
            if let Some(id) = nth(&tiles, i) {
                s.focus_tile(id)?;
            }
        }
        Op::FocusDirection(d) => {
            let from = s.target(None)?;
            s.focus_direction(from, d)?;
        }
        Op::SwapDirection(i, d) => {
            if let Some(id) = nth(&tiles, i) {
                s.swap_direction(id, d)?;
            }
        }
        Op::SwapTiles(i, j) => {
            if let (Some(a), Some(b)) = (nth(&tiles, i), nth(&tiles, j)) {
                s.swap_tiles(a, b)?;
            }
        }
        Op::Move(i, d) => {
            if let Some(id) = nth(&tiles, i) {
                s.move_tile(id, d)?;
            }
        }
        Op::Resize(i, d, amount) => {
            if let Some(id) = nth(&tiles, i) {
                s.resize(id, d, amount)?;
            }
        }
        Op::Zoom(i, z) => {
            if let Some(id) = nth(&tiles, i) {
                s.zoom(id, z)?;
            }
        }
        Op::CloseMissingTile => {
            s.close_tile(TileId::new())?;
        }
        Op::MoveTab(t, i) => {
            if let Some(id) = nth(&tabs, t) {
                s.move_tab(id, i)?;
            }
        }
        Op::MoveWorkspace(w, i) => {
            if let Some(id) = nth(&workspaces, w) {
                s.move_workspace(id, i)?;
            }
        }
        Op::SplitAuto(i) => {
            if let Some(id) = nth(&tabs, i) {
                s.split_auto(id, terminal())?;
            }
        }
        Op::Promote(i) => {
            if let Some(id) = nth(&tiles, i) {
                s.promote(id)?;
            }
        }
        Op::Flip(i) => {
            if let Some(id) = nth(&tabs, i) {
                s.flip(id)?;
            }
        }
        Op::Equalize(i) => {
            if let Some(id) = nth(&tabs, i) {
                s.equalize(id)?;
            }
        }
        Op::SetLayout(i, k) => {
            if let Some(id) = nth(&tabs, i) {
                s.set_layout(id, k)?;
            }
        }
        Op::ToggleLayout(i, k) => {
            if let Some(id) = nth(&tabs, i) {
                s.toggle_layout(id, k)?;
            }
        }
        Op::CycleLayout(i, step) => {
            if let Some(id) = nth(&tabs, i) {
                s.cycle_layout(id, step)?;
            }
        }
        Op::Masters(i, d) => {
            if let Some(id) = nth(&tabs, i) {
                s.change_masters(id, d)?;
            }
        }
        Op::MasterRatio(i, d) => {
            if let Some(id) = nth(&tabs, i) {
                s.change_master_ratio(id, d)?;
            }
        }
        Op::MoveOrder(i, step) => {
            if let Some(id) = nth(&tiles, i) {
                s.move_in_order(id, step)?;
            }
        }
        Op::RotateTurn(i, reverse) => {
            if let Some(id) = nth(&tabs, i) {
                s.rotate_turn(id, reverse)?;
            }
        }
        Op::RotatePart(i) => {
            if let Some(id) = nth(&tabs, i) {
                s.rotate_part(id)?;
            }
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 10_000, ..ProptestConfig::default() })]

    #[test]
    fn random_op_sequences_keep_the_invariants(
        ops in prop::collection::vec(op(), 1..40),
        default in kind(),
        insert in prop::sample::select(vec![InsertAt::End, InsertAt::Master, InsertAt::AfterFocused]),
    ) {
        let mut session = Session::new();
        // New tabs start as any kind, so every op runs against every kind.
        session.prefs.default = default;
        session.prefs.insert = insert;
        let (_, ws) = session.create_window("main", None);
        session.create_tab(ws, Tile::new(TileContent::terminal()), None).unwrap();
        prop_assert_eq!(session.check_invariants(), Ok(()));

        for op in &ops {
            let before = session.clone();
            let tile_count = before.tiles().count();
            match apply(&mut session, op) {
                Ok(()) => {}
                Err(_) => prop_assert_eq!(&session, &before, "refused {:?} changed state", op),
            }
            if let Err(broken) = session.check_invariants() {
                return Err(TestCaseError::fail(format!("after {op:?}: {broken}")));
            }
            // A turn of a manual tab turns its whole tree, whatever its
            // shape, and a turn back gives it back exactly (ratios on the
            // turn's grid); Monocle and Three Column don't move.
            if let Op::RotateTurn(i, reverse) = *op {
                let tabs: Vec<_> = before.windows.iter()
                    .flat_map(|w| w.workspaces.iter().flat_map(|ws| ws.tabs.iter().map(|t| t.id)))
                    .collect();
                if let Some(id) = nth(&tabs, i) {
                    let was = before.tab(id).unwrap();
                    let now = session.tab(id).unwrap();
                    match was.layout.kind {
                        LayoutKind::Manual => {
                            let mut back = session.clone();
                            back.rotate_turn(id, !reverse).unwrap();
                            let mut want = was.root.clone();
                            want.snap_ratios();
                            prop_assert_eq!(&back.tab(id).unwrap().root, &want);
                            prop_assert_eq!(now.focused, was.focused);
                            prop_assert_eq!(now.zoomed, was.zoomed);
                        }
                        LayoutKind::Monocle | LayoutKind::ThreeColumn => {
                            prop_assert_eq!(now, was, "{:?} turned a {:?} tab", op, was.layout.kind);
                        }
                        LayoutKind::Tile => {}
                    }
                }
            }
            // Only split and create-tab add tiles; nothing else may lose or invent one
            // except the closing ops.
            let after = session.tiles().count();
            match op {
                Op::Split(..) | Op::SplitAuto(_) | Op::CreateTab(_) => prop_assert!(after <= tile_count + 1),
                Op::CloseTile(_) => prop_assert!(after + 1 >= tile_count),
                Op::CloseTab(_) | Op::CloseWorkspace(_) | Op::CloseWindow(_) => {
                    prop_assert!(after <= tile_count)
                }
                _ => prop_assert_eq!(after, tile_count, "{:?} changed the tile count", op),
            }
            // A focused tile, when there is one, is a real tile in the focused tab.
            let focus = session.focus();
            if let Some(tile) = focus.tile {
                let path = session.locate(tile).expect("focused tile exists");
                let (_, _, tab) = session.ids_at(path);
                prop_assert_eq!(Some(tab), focus.tab);
            }
        }
    }
}

#[derive(Debug, Clone)]
enum TallOp {
    SplitAuto,
    Promote(usize),
    Rotate,
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2_000, ..ProptestConfig::default() })]

    /// Only tall ops: the stack always stays one evenly divided chain.
    #[test]
    fn tall_ops_keep_an_even_stack(ops in prop::collection::vec(
        prop_oneof![
            3 => Just(TallOp::SplitAuto),
            1 => (0usize..16).prop_map(TallOp::Promote),
            1 => Just(TallOp::Rotate),
        ],
        1..20,
    )) {
        let mut root = azulejo_model::Node::leaf(TileId::new());
        for op in &ops {
            match *op {
                TallOp::SplitAuto => root.split_auto(TileId::new()).unwrap(),
                TallOp::Promote(i) => {
                    let tiles = root.tiles();
                    root.promote(tiles[i % tiles.len()]).unwrap();
                }
                TallOp::Rotate => {
                    root.rotate();
                }
            }
            let tall = root.tall();
            prop_assert_eq!(tall.stack.len() + 1, root.leaf_count());
            let rects = root.rects();
            let size = |id: &TileId| {
                let r = rects.iter().find(|(t, _)| t == id).unwrap().1;
                if tall.vertical { r.height } else { r.width }
            };
            if let Some(first) = tall.stack.first() {
                let want = size(first);
                for t in &tall.stack {
                    prop_assert!((size(t) - want).abs() < 1e-9, "uneven stack after {:?}", op);
                }
                prop_assert!((want * tall.stack.len() as f64 - 1.0).abs() < 1e-9);
            }
            prop_assert!(!root.clone().equalize(), "equalize had work left after {:?}", op);
        }
    }
}

/// The leaf sibling of `tile` and the axis of their shared split, if `tile`'s
/// parent split holds another leaf directly.
fn leaf_sibling(node: &azulejo_model::Node, tile: TileId) -> Option<(azulejo_model::Axis, TileId)> {
    use azulejo_model::Node;
    let Node::Split { axis, a, b, .. } = node else {
        return None;
    };
    match (a.as_ref(), b.as_ref()) {
        (Node::Leaf { id: x }, Node::Leaf { id: y }) if *x == tile => Some((*axis, *y)),
        (Node::Leaf { id: x }, Node::Leaf { id: y }) if *y == tile => Some((*axis, *x)),
        _ => leaf_sibling(a, tile).or_else(|| leaf_sibling(b, tile)),
    }
}

#[derive(Debug, Clone)]
enum TreeOp {
    Split(usize, Direction),
    SplitAuto,
    Resize(usize, Direction, f64),
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 10_000, ..ProptestConfig::default() })]

    /// Moving a tile into an aligned neighbour and back restores the tree
    /// exactly; always true for a leaf sibling in a split along the move.
    #[test]
    fn move_there_and_back_restores_aligned_moves(
        ops in prop::collection::vec(
            prop_oneof![
                3 => (0usize..16, direction()).prop_map(|(t, d)| TreeOp::Split(t, d)),
                2 => Just(TreeOp::SplitAuto),
                2 => (0usize..16, direction(), -0.4f64..0.4).prop_map(|(t, d, a)| TreeOp::Resize(t, d, a)),
            ],
            1..16,
        ),
        pick in 0usize..16,
        d in direction(),
    ) {
        use azulejo_model::Node;
        let mut root = Node::leaf(TileId::new());
        for op in &ops {
            let tiles = root.tiles();
            match *op {
                TreeOp::Split(i, dir) => root.split(tiles[i % tiles.len()], dir, TileId::new()).unwrap(),
                TreeOp::SplitAuto => root.split_auto(TileId::new()).unwrap(),
                TreeOp::Resize(i, dir, amount) => {
                    let _ = root.resize(tiles[i % tiles.len()], dir, amount);
                }
            }
        }
        let tiles = root.tiles();
        let tile = tiles[pick % tiles.len()];
        let Some(neighbour) = root.neighbour(tile, d) else {
            return Ok(());
        };
        let sibling = matches!(
            leaf_sibling(&root, tile),
            Some((axis, s)) if s == neighbour && axis == d.axis()
        );
        let before = root.clone();
        let mut moved = root.clone();
        moved.move_tile(tile, d).unwrap();
        let back_target = moved.neighbour(tile, d.opposite());
        // A move that kept the shape (a slot swap) must undo exactly.
        let swapped = {
            let mut s = before.clone();
            s.swap(tile, neighbour).unwrap();
            s == moved
        };
        if sibling {
            prop_assert!(swapped, "sibling move restructured the tree");
        }
        if swapped {
            prop_assert_eq!(back_target, Some(neighbour));
            moved.move_tile(tile, d.opposite()).unwrap();
            prop_assert_eq!(&moved, &before);
        }
    }

    /// Resize only errors when the tile has no divider at all on that axis.
    #[test]
    fn resize_errors_only_without_a_divider_on_the_axis(
        ops in prop::collection::vec((0usize..16, direction()), 0..12),
        pick in 0usize..16,
        d in direction(),
        amount in -0.5f64..0.5,
    ) {
        use azulejo_model::Node;
        let mut root = Node::leaf(TileId::new());
        for (i, dir) in &ops {
            let tiles = root.tiles();
            root.split(tiles[i % tiles.len()], *dir, TileId::new()).unwrap();
        }
        let tiles = root.tiles();
        let tile = tiles[pick % tiles.len()];
        let rect = root.rects().into_iter().find(|(t, _)| *t == tile).unwrap().1;
        let spans_axis = match d.axis() {
            azulejo_model::Axis::Horizontal => rect.width >= 1.0 - 1e-9,
            azulejo_model::Axis::Vertical => rect.height >= 1.0 - 1e-9,
        };
        let before = root.clone();
        match root.resize(tile, d, amount) {
            Ok(()) => prop_assert!(!spans_axis),
            Err(_) => {
                prop_assert!(spans_axis, "refused although a divider exists");
                prop_assert_eq!(&root, &before);
            }
        }
    }
}
