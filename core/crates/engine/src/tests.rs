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
        tab_id: "w1:t1".into(),
        machine: None,
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
                page_title: None,
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
                tab_id: "t1".into(),
                machine: None,
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
fn terminal_io_needs_a_core_with_herdr() {
    let mut engine = Engine::new();
    let err = engine
        .execute(Command::TerminalRead(TerminalRead::default()))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable);
    // An unbound terminal has no pane to talk to yet.
    assert_eq!(
        engine.terminal_pane(None).unwrap_err().code,
        ErrorCode::Unavailable
    );
}

#[test]
fn lists_report_structure_and_focus() {
    let mut engine = Engine::new();
    let first_ws = engine.session().focus().workspace.unwrap();
    let Reply::Created(c) = engine
        .execute(Command::WorkspaceCreate(WorkspaceCreate {
            name: Some("web".into()),
            ..WorkspaceCreate::default()
        }))
        .unwrap()
        .reply
    else {
        panic!()
    };
    let web = c.workspace.unwrap();
    engine
        .execute(Command::TabCreate(TabCreate {
            workspace: Some(web),
            name: Some("dev".into()),
            content: None,
        }))
        .unwrap();
    engine
        .execute(Command::TabCreate(TabCreate {
            workspace: Some(web),
            name: None,
            content: Some(TileContent::Browser {
                url: None,
                page_title: None,
            }),
        }))
        .unwrap();

    let Reply::Windows { windows } = engine.execute(Command::WindowList).unwrap().reply else {
        panic!()
    };
    assert_eq!(windows.len(), 1);
    assert!(windows[0].focused);
    assert_eq!(windows[0].workspaces, vec![first_ws, web]);

    let Reply::Workspaces { workspaces } = engine
        .execute(Command::WorkspaceList(WorkspaceList::default()))
        .unwrap()
        .reply
    else {
        panic!()
    };
    let names: Vec<_> = workspaces
        .iter()
        .map(|w| (w.workspace.name.as_str(), w.tabs, w.active))
        .collect();
    assert_eq!(names, vec![("main", 1, false), ("web", 2, true)]);
    assert!(workspaces[1].focused);

    let Reply::Tabs { tabs } = engine
        .execute(Command::TabList(TabList {
            workspace: Some(web),
        }))
        .unwrap()
        .reply
    else {
        panic!()
    };
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0].tab.name.as_deref(), Some("dev"));
    assert!(!tabs[0].active && tabs[1].active && tabs[1].focused);
    assert!(tabs.iter().all(|t| t.workspace == web && t.tiles == 1));

    let Reply::Tabs { tabs } = engine
        .execute(Command::TabList(TabList::default()))
        .unwrap()
        .reply
    else {
        panic!()
    };
    assert_eq!(tabs.len(), 3);

    let missing = engine
        .execute(Command::TabList(TabList {
            workspace: Some(WorkspaceId::new()),
        }))
        .unwrap_err();
    assert_eq!(missing.code, ErrorCode::NotFound);
}

#[test]
fn rename_and_open_update_tiles() {
    let mut engine = Engine::new();
    let term = engine.session().focused_tile().unwrap();
    let out = engine
        .execute(Command::TileRename(TileRename {
            tile: None,
            title: Some("server".into()),
        }))
        .unwrap();
    assert!(out.events.iter().any(
        |e| matches!(e, Event::TileUpdated { tile, .. } if tile.title.as_deref() == Some("server"))
    ));
    engine
        .execute(Command::TileRename(TileRename {
            tile: Some(term),
            title: Some(String::new()),
        }))
        .unwrap();
    assert_eq!(engine.session().tile(term).unwrap().title, None);

    let err = engine
        .execute(Command::BrowserOpen(BrowserOpen {
            tile: None,
            url: "http://localhost:3000".into(),
        }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidState);
    assert!(
        err.message.contains("terminal tile, not a browser"),
        "{err}"
    );

    let split = |engine: &mut Engine, content| {
        let Reply::Created(c) = engine
            .execute(Command::LayoutSplit(LayoutSplit {
                tile: None,
                direction: Direction::Right,
                content: Some(content),
            }))
            .unwrap()
            .reply
        else {
            panic!()
        };
        c.tile.unwrap()
    };
    let browser = split(
        &mut engine,
        TileContent::Browser {
            url: None,
            page_title: None,
        },
    );
    let out = engine
        .execute(Command::BrowserOpen(BrowserOpen {
            tile: Some(browser),
            url: "http://localhost:3000".into(),
        }))
        .unwrap();
    assert!(out.events.iter().any(|e| matches!(
        e,
        Event::TileUpdated { tile, .. }
            if tile.content == TileContent::Browser { url: Some("http://localhost:3000".into()), page_title: None }
    )));

    // The shell reports navigation: URL and page title change, and no
    // separate load is asked for (that's browser.open's job).
    engine
        .execute(Command::BrowserState(BrowserState {
            tile: Some(browser),
            url: Some("http://localhost:3000/docs".into()),
            page_title: Some("Docs".into()),
        }))
        .unwrap();
    let state = |engine: &Engine| {
        engine
            .session()
            .tile(browser)
            .map(|t| t.content.clone())
            .unwrap()
    };
    assert_eq!(
        state(&engine),
        TileContent::Browser {
            url: Some("http://localhost:3000/docs".into()),
            page_title: Some("Docs".into()),
        }
    );
    // A title-only report keeps the URL; an empty title clears it.
    engine
        .execute(Command::BrowserState(BrowserState {
            tile: Some(browser),
            url: None,
            page_title: Some(String::new()),
        }))
        .unwrap();
    assert_eq!(
        state(&engine),
        TileContent::Browser {
            url: Some("http://localhost:3000/docs".into()),
            page_title: None,
        }
    );

    let preview = split(&mut engine, TileContent::Preview { path: None });
    engine
        .execute(Command::PreviewOpen(PreviewOpen {
            tile: None,
            path: "/tmp/plan.md".into(),
        }))
        .unwrap();
    assert_eq!(
        engine.session().tile(preview).unwrap().content,
        TileContent::Preview {
            path: Some("/tmp/plan.md".into())
        }
    );
    let err = engine
        .execute(Command::PreviewOpen(PreviewOpen {
            tile: Some(browser),
            path: "/tmp/plan.md".into(),
        }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidState);
}

#[test]
fn search_finds_lines_case_insensitively_by_default() {
    let text = "ok\nWARN low disk\nwarn again\n\nfine";
    let hits = crate::terminal::search(text, "warn", false);
    assert_eq!(hits.iter().map(|m| m.line).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(crate::terminal::search(text, "WARN", true).len(), 1);
    assert!(crate::terminal::search(text, "nope", false).is_empty());
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

#[test]
fn moving_focus_onto_a_tile_clears_its_agent_attention() {
    use azulejo_model::{AgentInfo, AgentStatus};
    let mut engine = Engine::new();
    let first = engine.session().focused_tile().unwrap();
    engine.handle(Command::LayoutSplit(LayoutSplit {
        tile: None,
        direction: Direction::Right,
        content: None,
    }));
    engine.session.tile_mut(first).unwrap().agent = Some(AgentInfo {
        name: Some("claude".into()),
        status: AgentStatus::Blocked,
        since_ms: 1,
        attention: true,
    });
    // Unrelated commands leave it lit.
    engine.handle(Command::LayoutEqualize(Default::default()));
    assert!(engine.session().tile(first).unwrap().needs_attention());
    let events = engine.handle(Command::LayoutFocus(azulejo_protocol::LayoutFocus {
        direction: Direction::Left,
    }));
    assert!(!engine.session().tile(first).unwrap().needs_attention());
    assert!(events.iter().any(|e| matches!(e,
        Event::TileAttentionChanged { tile, attention: false, .. } if *tile == first)));
}
