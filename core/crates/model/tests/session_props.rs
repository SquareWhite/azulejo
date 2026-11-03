//! Random op sequences against the session. After every op the invariants must
//! hold, and a refused op must leave the session exactly as it was.

use azulejo_model::{Direction, Session, Tile, TileContent, TileId};
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
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 10_000, ..ProptestConfig::default() })]

    #[test]
    fn random_op_sequences_keep_the_invariants(ops in prop::collection::vec(op(), 1..40)) {
        let mut session = Session::new();
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
            // Only split and create-tab add tiles; nothing else may lose or invent one
            // except the closing ops.
            let after = session.tiles().count();
            match op {
                Op::Split(..) | Op::CreateTab(_) => prop_assert!(after <= tile_count + 1),
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
