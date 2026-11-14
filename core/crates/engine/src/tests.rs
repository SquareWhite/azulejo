use azulejo_model::{Direction, HerdrRef, Node, TileContent};
use azulejo_protocol::*;

use super::*;

fn names(events: &[Event]) -> Vec<String> {
    events.iter().map(Event::name).collect()
}

fn split(engine: &mut Engine, direction: Direction) -> Outcome {
    engine
        .execute(Command::LayoutSplit(LayoutSplit {
            tile: None,
            direction,
            content: None,
        }))
        .unwrap()
}

#[test]
fn starts_with_one_terminal_and_answers_ping() {
    let mut engine = Engine::new();
    assert_eq!(engine.session().tiles().count(), 1);
    assert_eq!(engine.session().check_invariants(), Ok(()));
    let outcome = engine.execute(Command::Ping).unwrap();
    assert_eq!(outcome.reply, Reply::Pong { api_version: 1 });
    assert!(outcome.events.is_empty());
}

#[test]
fn split_reports_the_new_tile_layout_and_focus() {
    let mut engine = Engine::new();
    let outcome = split(&mut engine, Direction::Right);
    let Reply::Created(Created {
        tile: Some(new_tile),
        ..
    }) = outcome.reply
    else {
        panic!("expected created, got {:?}", outcome.reply);
    };
    assert_eq!(
        names(&outcome.events),
        ["tile.created", "layout.changed", "focus.changed"]
    );
    let Event::LayoutChanged(state) = &outcome.events[1] else {
        unreachable!()
    };
    assert_eq!(state.focused, new_tile);
    assert_eq!(state.tree.leaf_count(), 2);
}

#[test]
fn the_shells_launch_sequence_builds_four_tiles() {
    let mut engine = Engine::new();
    let events = engine.handle(Command::LayoutGet);
    assert_eq!(names(&events), ["layout.changed"]);
    for direction in [Direction::Right, Direction::Down, Direction::Right] {
        split(&mut engine, direction);
    }
    let Ok(Outcome {
        reply: Reply::Layout(state),
        ..
    }) = engine.execute(Command::LayoutGet)
    else {
        panic!("expected a layout")
    };
    assert_eq!(state.tree.leaf_count(), 4);
    assert!(matches!(state.tree, Node::Split { .. }));
}

#[test]
fn refused_commands_change_nothing() {
    let mut engine = Engine::new();
    let before = engine.session().clone();
    let err = engine
        .execute(Command::LayoutFocus(LayoutFocus {
            direction: Direction::Left,
        }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidState);
    let err = engine
        .execute(Command::TileFocus(TileTarget {
            tile: Some(TileId::new()),
        }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    let err = engine
        .execute(Command::LayoutSwap(LayoutSwap::default()))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParams);
    assert_eq!(engine.session(), &before);
    assert!(
        engine
            .handle(Command::LayoutResize(LayoutResize {
                tile: None,
                direction: Direction::Right,
                amount: 0.1,
            }))
            .is_empty()
    );
}

#[test]
fn closing_a_bound_terminal_carries_its_pane_and_kill_flag() {
    let mut engine = Engine::new();
    let tile = engine.session().focused_tile().unwrap();
    let herdr = HerdrRef {
        pane_id: "w1:p1".into(),
        terminal_id: "term_1".into(),
    };
    let bound = engine
        .execute(Command::TerminalBind(TerminalBind {
            tile,
            herdr: herdr.clone(),
        }))
        .unwrap();
    assert_eq!(names(&bound.events), ["tile.updated"]);

    let closed = engine
        .execute(Command::TileClose(TileClose {
            tile: None,
            kill: true,
        }))
        .unwrap();
    // It was the tab's only tile, so the tab closes too.
    assert_eq!(
        names(&closed.events),
        ["tile.closed", "tab.closed", "focus.changed"]
    );
    let Event::TileClosed { tile: t, kill, .. } = &closed.events[0] else {
        unreachable!()
    };
    assert!(*kill);
    assert!(matches!(&t.content, TileContent::Terminal { herdr: Some(h), .. } if *h == herdr));
    assert_eq!(engine.session().focused_tile(), None);
}

#[test]
fn bind_rejects_non_terminals() {
    let mut engine = Engine::new();
    let created = engine
        .execute(Command::TabCreate(TabCreate {
            workspace: None,
            name: Some("web".into()),
            content: Some(TileContent::Browser {
                url: Some("http://localhost:3000".into()),
            }),
        }))
        .unwrap();
    assert_eq!(
        names(&created.events),
        [
            "tab.created",
            "tile.created",
            "layout.changed",
            "focus.changed"
        ]
    );
    let browser = engine.session().focused_tile().unwrap();
    let err = engine
        .execute(Command::TerminalBind(TerminalBind {
            tile: browser,
            herdr: HerdrRef {
                pane_id: "p".into(),
                terminal_id: "t".into(),
            },
        }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidState);
}

#[test]
fn workspaces_and_tabs_report_structure_changes() {
    let mut engine = Engine::new();
    let ws = engine
        .execute(Command::WorkspaceCreate(WorkspaceCreate::default()))
        .unwrap();
    assert_eq!(names(&ws.events), ["workspace.created", "focus.changed"]);
    engine
        .execute(Command::TabCreate(TabCreate::default()))
        .unwrap();
    engine
        .execute(Command::TabCreate(TabCreate::default()))
        .unwrap();
    let moved = engine
        .execute(Command::TabMove(TabMove {
            tab: None,
            index: 0,
        }))
        .unwrap();
    assert_eq!(names(&moved.events), ["tab.reordered"]);
    let renamed = engine
        .execute(Command::WorkspaceRename(WorkspaceRename {
            workspace: None,
            name: "api".into(),
        }))
        .unwrap();
    assert_eq!(names(&renamed.events), ["workspace.updated"]);
    let closed = engine
        .execute(Command::WorkspaceClose(WorkspaceTarget::default()))
        .unwrap();
    assert_eq!(
        names(&closed.events),
        [
            "tile.closed",
            "tile.closed",
            "tab.closed",
            "tab.closed",
            "workspace.closed",
            "focus.changed"
        ]
    );
}

#[test]
fn tile_list_and_get_report_location_and_focus() {
    let mut engine = Engine::new();
    split(&mut engine, Direction::Down);
    let Reply::Tiles { tiles } = engine
        .execute(Command::TileList(TileList::default()))
        .unwrap()
        .reply
    else {
        panic!()
    };
    assert_eq!(tiles.len(), 2);
    assert_eq!(tiles.iter().filter(|t| t.focused).count(), 1);
    let Reply::Tile(info) = engine
        .execute(Command::TileGet(TileTarget::default()))
        .unwrap()
        .reply
    else {
        panic!()
    };
    assert!(info.focused);
    assert_eq!(info.tile.kind(), azulejo_model::TileKind::Terminal);
}

#[test]
fn terminal_io_waits_for_the_herdr_adapter() {
    let mut engine = Engine::new();
    let err = engine
        .execute(Command::TerminalRead(TerminalRead::default()))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Unsupported);
}

#[test]
fn tall_commands_append_promote_rotate_and_equalize() {
    let mut engine = Engine::new();
    let master = engine.session().focused_tile().unwrap();
    let mut stack = Vec::new();
    for _ in 0..3 {
        let outcome = engine
            .execute(Command::LayoutSplitAuto(LayoutSplitAuto::default()))
            .unwrap();
        let Reply::Created(Created { tile: Some(t), .. }) = outcome.reply else {
            panic!("expected created");
        };
        assert!(names(&outcome.events).contains(&"layout.changed".to_string()));
        stack.push(t);
    }
    let tab = engine.session().focus().tab.unwrap();
    let root = || engine.session().tab(tab).unwrap().root.clone();
    assert_eq!(root().tall().master, master);
    assert_eq!(root().tall().stack, stack);

    engine
        .execute(Command::LayoutPromote(TileTarget::default()))
        .unwrap();
    assert_eq!(
        engine.session().tab(tab).unwrap().root.tall().master,
        stack[2]
    );
    assert_eq!(engine.session().focused_tile(), Some(stack[2]));

    engine
        .execute(Command::LayoutRotate(TabTarget::default()))
        .unwrap();
    assert!(!engine.session().tab(tab).unwrap().root.tall().vertical);

    // Already even: no layout event.
    let outcome = engine
        .execute(Command::LayoutEqualize(TabTarget::default()))
        .unwrap();
    assert!(outcome.events.is_empty());
}
