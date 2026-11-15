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
fn tab_move_takes_any_index_and_keeps_the_active_tab() {
    // What a drag in the top bar sends: the dragged tab and where it lands.
    let mut engine = Engine::new();
    for _ in 0..3 {
        engine
            .execute(Command::TabCreate(TabCreate::default()))
            .unwrap();
    }
    let ws = engine.session().focus().workspace.unwrap();
    let order = |e: &Engine| -> Vec<TabId> {
        e.session().windows[0].workspaces[0]
            .tabs
            .iter()
            .map(|t| t.id)
            .collect()
    };
    let start = order(&engine);
    let [a, b, c, d] = start[..] else {
        panic!("four tabs, got {start:?}")
    };
    let active = engine.session().focus().tab.unwrap();
    assert_eq!(active, d);
    let mut drag = |tab: TabId, index: usize| {
        let outcome = engine
            .execute(Command::TabMove(TabMove {
                tab: Some(tab),
                index,
            }))
            .unwrap();
        let [Event::TabsReordered { workspace, order }] = &outcome.events[..] else {
            panic!("expected tab.reordered, got {:?}", outcome.events);
        };
        assert_eq!(*workspace, ws);
        order.clone()
    };
    // A background tab, first to third; the active one, last to first; one
    // past the end goes last.
    assert_eq!(drag(a, 2), [b, c, a, d]);
    assert_eq!(drag(d, 0), [d, b, c, a]);
    assert_eq!(drag(b, 9), [d, c, a, b]);
    assert_eq!(order(&engine), [d, c, a, b]);
    assert_eq!(engine.session().focus().tab, Some(active));
    assert_eq!(engine.session().check_invariants(), Ok(()));
    // Where it already is: nothing changes, nothing to report.
    let same = engine
        .execute(Command::TabMove(TabMove {
            tab: Some(c),
            index: 1,
        }))
        .unwrap();
    assert!(same.events.is_empty(), "{:?}", same.events);
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

/// `workspace.list` as (name, number) pairs, in the order it lists them.
fn listed(engine: &mut Engine) -> Vec<(String, usize)> {
    let Reply::Workspaces { workspaces } = engine
        .execute(Command::WorkspaceList(WorkspaceList::default()))
        .unwrap()
        .reply
    else {
        panic!()
    };
    workspaces
        .into_iter()
        .map(|w| (w.workspace.name, w.number))
        .collect()
}

#[test]
fn workspace_move_reorders_the_window_and_the_numbers_follow() {
    let mut engine = Engine::new();
    engine
        .execute(Command::WorkspaceRename(WorkspaceRename {
            workspace: None,
            name: "a".into(),
        }))
        .unwrap();
    let mut ids = vec![engine.session().focus().workspace.unwrap()];
    for name in ["b", "c", "d"] {
        engine
            .execute(Command::WorkspaceCreate(WorkspaceCreate {
                name: Some(name.into()),
                ..Default::default()
            }))
            .unwrap();
        ids.push(engine.session().focus().workspace.unwrap());
    }
    let numbered = |names: &[&str]| -> Vec<(String, usize)> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.to_string(), i + 1))
            .collect()
    };
    assert_eq!(listed(&mut engine), numbered(&["a", "b", "c", "d"]));

    // Down: "a" to the third place (index 2, counted once it has left).
    let moved = engine
        .execute(Command::WorkspaceMove(WorkspaceMove {
            workspace: Some(ids[0]),
            index: 2,
        }))
        .unwrap();
    assert_eq!(moved.reply, Reply::Ok);
    assert_eq!(names(&moved.events), ["workspace.reordered"]);
    let Event::WorkspacesReordered { order, .. } = &moved.events[0] else {
        panic!()
    };
    assert_eq!(order, &[ids[1], ids[2], ids[0], ids[3]]);
    assert_eq!(listed(&mut engine), numbered(&["b", "c", "a", "d"]));

    // Up: "d" first. Focus stays where it was (on "d", created last).
    let focused = engine.session().focus().workspace;
    engine
        .execute(Command::WorkspaceMove(WorkspaceMove {
            workspace: Some(ids[3]),
            index: 0,
        }))
        .unwrap();
    assert_eq!(listed(&mut engine), numbered(&["d", "b", "c", "a"]));
    assert_eq!(engine.session().focus().workspace, focused);

    // Past the end means last; the focused workspace when none is named.
    engine
        .execute(Command::WorkspaceMove(WorkspaceMove {
            workspace: None,
            index: 99,
        }))
        .unwrap();
    assert_eq!(listed(&mut engine), numbered(&["b", "c", "a", "d"]));

    // Its own place: nothing changes, nothing is reported.
    let same = engine
        .execute(Command::WorkspaceMove(WorkspaceMove {
            workspace: Some(ids[1]),
            index: 0,
        }))
        .unwrap();
    assert!(same.events.is_empty(), "{:?}", names(&same.events));

    // An unknown workspace is an error and changes nothing.
    let err = engine
        .execute(Command::WorkspaceMove(WorkspaceMove {
            workspace: Some(WorkspaceId::new()),
            index: 0,
        }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert_eq!(listed(&mut engine), numbered(&["b", "c", "a", "d"]));
    assert_eq!(engine.session().check_invariants(), Ok(()));
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
    // Another kind of tile gets a new preview beside it.
    let outcome = engine
        .execute(Command::PreviewOpen(PreviewOpen {
            tile: Some(browser),
            path: "/tmp/plan.md".into(),
        }))
        .unwrap();
    assert!(matches!(outcome.reply, Reply::Created(Created { tile: Some(t), .. }) if t != preview));
}

#[test]
fn preview_open_from_a_terminal_opens_a_preview_to_its_right_and_keeps_focus() {
    let mut engine = Engine::new();
    let terminal = engine.session().focused_tile().unwrap();
    let outcome = engine
        .execute(Command::PreviewOpen(PreviewOpen {
            tile: Some(terminal),
            path: "/work/plan.md".into(),
        }))
        .unwrap();
    let Reply::Created(Created {
        tile: Some(preview),
        ..
    }) = outcome.reply
    else {
        panic!("expected created, got {:?}", outcome.reply);
    };
    let s = engine.session();
    assert_eq!(
        s.tile(preview).unwrap().content,
        TileContent::Preview {
            path: Some("/work/plan.md".into())
        }
    );
    assert_eq!(s.focused_tile(), Some(terminal));
    let tab = s.tab(s.focus().tab.unwrap()).unwrap();
    assert_eq!(tab.root.tiles(), vec![terminal, preview]);
    assert!(!names(&outcome.events).contains(&"focus.changed".to_owned()));

    // Focus elsewhere stays there too: naming another tile doesn't pull
    // focus to it.
    split(&mut engine, Direction::Down);
    let focused = engine.session().focused_tile().unwrap();
    engine
        .execute(Command::PreviewOpen(PreviewOpen {
            tile: Some(terminal),
            path: "/work/other.md".into(),
        }))
        .unwrap();
    assert_eq!(engine.session().focused_tile(), Some(focused));
    assert_eq!(engine.session().check_invariants(), Ok(()));
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
fn line_rows_follow_soft_wraps() {
    use crate::terminal::line_rows;
    // Typed lines against the rows on screen: line 1 wraps (herdr trims the
    // space at the wrap), line 2 is blank.
    let lines = ["$ ls", "one two three", "", "end"];
    let rows = ["$ ls", "one two", " three", "", "end"];
    assert_eq!(
        line_rows(&lines, &rows),
        vec![Some(0), Some(1), Some(3), Some(4)]
    );
    // Rows that stop lining up (output arrived between the reads).
    let moved = ["$ ls", "one two", "new output"];
    assert_eq!(line_rows(&lines, &moved), vec![Some(0), None, None, None]);
    assert_eq!(
        line_rows(&lines, &["$ ls"]),
        vec![Some(0), None, None, None]
    );
}

#[test]
fn first_row_finds_the_view_in_a_cut_read() {
    use crate::terminal::first_row;
    let view = |offset| azulejo_protocol::ScrollState {
        offset_from_bottom: offset,
        max_offset_from_bottom: 10,
        viewport_rows: 2,
    };
    // Buffer rows 0..12; herdr gave rows 5..=10 (row 11 is blank, trimmed).
    let rows = ["e", "f", "x", "x", "y", "$"];
    // Live view: rows 10 and 11, shown as "$" alone.
    assert_eq!(first_row(&rows, &["$"], &view(0)), Some(5));
    // Scrolled up 3: rows 7 and 8, "x x"; nearest the guess wins.
    assert_eq!(first_row(&rows, &["x", "x"], &view(3)), Some(5));
    // Scrolled above the read: no anchor.
    assert_eq!(first_row(&rows, &["a", "b"], &view(9)), None);
}

#[test]
fn centre_puts_a_row_mid_view() {
    use crate::terminal::centre;
    let view = azulejo_protocol::ScrollState {
        offset_from_bottom: 0,
        max_offset_from_bottom: 128,
        viewport_rows: 40,
    };
    // Rows 0..168; row 122 shows mid-view with its top at 102.
    assert_eq!(centre(122, &view), 26);
    // Near the top: as far up as it goes; near the bottom: the live view.
    assert_eq!(centre(3, &view), 128);
    assert_eq!(centre(167, &view), 0);
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
        .execute(Command::LayoutRotate(LayoutRotate::default()))
        .unwrap();
    assert!(!engine.session().tab(tab).unwrap().root.tall().vertical);

    // Already even: no layout event.
    let outcome = engine
        .execute(Command::LayoutEqualize(TabTarget::default()))
        .unwrap();
    assert!(outcome.events.is_empty());
}

#[test]
fn rotate_turns_any_manual_tree_four_ways_and_monocle_stays_silent() {
    // A manual 2×2 grid, the shape herdr's own splits make and adoption keeps.
    let mut engine = Engine::new();
    let a = engine.session().focused_tile().unwrap();
    split(&mut engine, Direction::Right);
    let b = engine.session().focused_tile().unwrap();
    engine
        .execute(Command::LayoutFocus(LayoutFocus {
            direction: Direction::Left,
        }))
        .unwrap();
    split(&mut engine, Direction::Down);
    engine
        .execute(Command::LayoutFocus(LayoutFocus {
            direction: Direction::Right,
        }))
        .unwrap();
    split(&mut engine, Direction::Down);
    assert_eq!(tab_of(&engine).kind(), LayoutKind::Manual);
    let original = tab_of(&engine).root.clone();
    let focused = engine.session().focused_tile();
    let rect = |engine: &Engine, t| {
        tab_of(engine)
            .root
            .rects()
            .into_iter()
            .find(|(id, _)| *id == t)
            .unwrap()
            .1
    };

    let mut seen = Vec::new();
    for turn in 0..4 {
        let events = run(&mut engine, Command::LayoutRotate(LayoutRotate::default()));
        assert!(
            names(&events).contains(&"layout.changed".to_string()),
            "turn {turn} changed nothing"
        );
        seen.push((rect(&engine, a), rect(&engine, b)));
        assert_eq!(engine.session().focused_tile(), focused, "focus stays");
    }
    // a starts top left: top right, bottom right, bottom left, home.
    let corner = |r: azulejo_model::Rect| (r.x > 0.25, r.y > 0.25);
    assert_eq!(
        seen.iter().map(|(ra, _)| corner(*ra)).collect::<Vec<_>>(),
        [(true, false), (true, true), (false, true), (false, false)]
    );
    assert_eq!(
        tab_of(&engine).root,
        original,
        "four turns are the identity"
    );

    // Anticlockwise undoes clockwise.
    run(&mut engine, Command::LayoutRotate(LayoutRotate::default()));
    run(
        &mut engine,
        Command::LayoutRotate(LayoutRotate {
            reverse: true,
            ..LayoutRotate::default()
        }),
    );
    assert_eq!(tab_of(&engine).root, original);

    // flip is still the old two-way tall flip (the grid becomes master
    // left | stack, then master on top).
    let mut flipped = original.clone();
    flipped.rotate();
    run(
        &mut engine,
        Command::LayoutRotate(LayoutRotate {
            flip: true,
            ..LayoutRotate::default()
        }),
    );
    assert_eq!(tab_of(&engine).root, flipped);
    assert!(tab_of(&engine).root.tall().vertical);

    // Monocle has nothing to turn: Ok, and no event at all.
    run(
        &mut engine,
        Command::LayoutSet(LayoutSetKind {
            tab: None,
            kind: LayoutKind::Monocle,
        }),
    );
    let before = tab_of(&engine).clone();
    let outcome = engine
        .execute(Command::LayoutRotate(LayoutRotate::default()))
        .unwrap();
    assert_eq!(outcome.reply, Reply::Ok);
    assert!(outcome.events.is_empty(), "{:?}", names(&outcome.events));
    assert_eq!(tab_of(&engine), &before);
}

#[test]
fn focus_alone_keeps_a_done_mark_and_tile_seen_clears_it() {
    use azulejo_model::{AgentInfo, AgentStatus};
    let mut engine = Engine::new();
    let first = engine.session().focused_tile().unwrap();
    engine.handle(Command::LayoutSplit(LayoutSplit {
        tile: None,
        direction: Direction::Right,
        content: None,
    }));
    let finished = |status| AgentInfo {
        name: Some("claude".into()),
        status,
        since_ms: 1,
        attention: true,
    };
    engine.session.tile_mut(first).unwrap().agent = Some(finished(AgentStatus::Done));
    // Moving focus onto it (the shell decides whether anyone's looking), or
    // focusing it by id, leaves it lit.
    engine.handle(Command::LayoutFocus(azulejo_protocol::LayoutFocus {
        direction: Direction::Left,
    }));
    assert_eq!(engine.session().focused_tile(), Some(first));
    engine.handle(Command::TileFocus(TileTarget { tile: Some(first) }));
    assert!(engine.session().tile(first).unwrap().needs_attention());
    // `tile.seen` reads it.
    let events = engine.handle(Command::TileSeen(TileTarget { tile: None }));
    assert!(!engine.session().tile(first).unwrap().needs_attention());
    assert!(events.iter().any(|e| matches!(e,
        Event::TileAttentionChanged { tile, attention: false, .. } if *tile == first)));
    // A blocked agent isn't answered by looking.
    engine.session.tile_mut(first).unwrap().agent = Some(finished(AgentStatus::Blocked));
    let events = engine.handle(Command::TileSeen(TileTarget { tile: Some(first) }));
    assert!(engine.session().tile(first).unwrap().needs_attention());
    assert!(events.is_empty(), "{:?}", names(&events));
}

// ----- dynamic layouts (ADR 0010) -----

/// An engine whose first tab is a dynamic Tile tab, as the app starts.
fn tile_engine() -> Engine {
    let mut engine = Engine::new();
    let events = engine.set_layout_prefs(azulejo_model::LayoutPrefs {
        default: LayoutKind::Tile,
        ..azulejo_model::LayoutPrefs::default()
    });
    assert!(names(&events).contains(&"layout.changed".to_string()));
    assert!(names(&events).contains(&"tab.updated".to_string()));
    engine
}

fn tab_of(engine: &Engine) -> &azulejo_model::Tab {
    let id = engine.session().focus().tab.unwrap();
    engine.session().tab(id).unwrap()
}

fn layout_event(events: &[Event]) -> &LayoutState {
    events
        .iter()
        .find_map(|e| match e {
            Event::LayoutChanged(s) => Some(s),
            _ => None,
        })
        .expect("layout.changed")
}

fn run(engine: &mut Engine, command: Command) -> Vec<Event> {
    engine.execute(command).unwrap().events
}

#[test]
fn the_first_tab_takes_the_configured_default_once() {
    let mut engine = tile_engine();
    assert_eq!(tab_of(&engine).kind(), LayoutKind::Tile);
    // Later prefs don't touch existing tabs.
    engine.set_layout_prefs(azulejo_model::LayoutPrefs::default());
    assert_eq!(tab_of(&engine).kind(), LayoutKind::Tile);
    // But new tabs follow them.
    run(&mut engine, Command::TabCreate(TabCreate::default()));
    assert_eq!(tab_of(&engine).kind(), LayoutKind::Manual);
}

#[test]
fn dynamic_split_ignores_direction_and_reports_rects() {
    let mut engine = tile_engine();
    let a = engine.session().focused_tile().unwrap();
    let events = split(&mut engine, Direction::Left).events;
    let state = layout_event(&events);
    assert_eq!(state.kind, LayoutKind::Tile);
    assert_eq!(state.order.len(), 2);
    assert_eq!(state.order[0], a, "appended, not put on the left");
    assert_eq!(state.rects.len(), 2);
    let master = state.rects.iter().find(|p| p.tile == a).unwrap();
    assert_eq!(master.rect.x, 0.0);
    assert!((master.rect.width - 0.5).abs() < 1e-9);
    split(&mut engine, Direction::Up);
    assert_eq!(tab_of(&engine).layout.order.len(), 3);
    engine.session().check_invariants().unwrap();
}

#[test]
fn every_layout_command_maps_in_a_dynamic_tab() {
    let mut engine = tile_engine();
    let tab = engine.session().focus().tab.unwrap();
    for _ in 0..3 {
        run(
            &mut engine,
            Command::LayoutSplitAuto(LayoutSplitAuto::default()),
        );
    }
    let order = tab_of(&engine).layout.order.clone();
    assert_eq!(order.len(), 4);
    let (m, s1, s2, s3) = (order[0], order[1], order[2], order[3]);

    // Swap and move reorder.
    run(
        &mut engine,
        Command::LayoutSwap(LayoutSwap {
            tile: Some(s1),
            direction: Some(Direction::Down),
            with: None,
        }),
    );
    assert_eq!(tab_of(&engine).layout.order, vec![m, s2, s1, s3]);
    run(
        &mut engine,
        Command::LayoutMove(LayoutMove {
            tile: Some(s1),
            direction: Direction::Up,
        }),
    );
    assert_eq!(tab_of(&engine).layout.order, vec![m, s1, s2, s3]);

    // Resize: the master's right edge → ratio; a stack edge → weights.
    run(
        &mut engine,
        Command::LayoutResize(LayoutResize {
            tile: Some(m),
            direction: Direction::Right,
            amount: 0.1,
        }),
    );
    assert!((tab_of(&engine).layout.params.tile.ratio - 0.6).abs() < 1e-9);
    run(
        &mut engine,
        Command::LayoutResize(LayoutResize {
            tile: Some(s1),
            direction: Direction::Down,
            amount: 0.1,
        }),
    );
    assert!(!tab_of(&engine).layout.weights.is_empty());
    // Equalize resets the weights only.
    run(&mut engine, Command::LayoutEqualize(TabTarget::default()));
    assert!(tab_of(&engine).layout.weights.is_empty());
    assert!((tab_of(&engine).layout.params.tile.ratio - 0.6).abs() < 1e-9);

    // Promote: to the front.
    run(
        &mut engine,
        Command::LayoutPromote(TileTarget { tile: Some(s3) }),
    );
    assert_eq!(tab_of(&engine).layout.order[0], s3);

    // Move in order, wrapping.
    run(
        &mut engine,
        Command::LayoutMoveOrder(LayoutMoveOrder {
            tile: Some(s3),
            step: -1,
        }),
    );
    assert_eq!(*tab_of(&engine).layout.order.last().unwrap(), s3);

    // Masters and ratio.
    run(
        &mut engine,
        Command::LayoutMasters(LayoutMasters {
            tab: None,
            delta: 1,
        }),
    );
    assert_eq!(tab_of(&engine).layout.params.tile.masters, 2);
    run(
        &mut engine,
        Command::LayoutMasterRatio(LayoutMasterRatio {
            tab: None,
            delta: -0.3,
        }),
    );
    assert!((tab_of(&engine).layout.params.tile.ratio - 0.3).abs() < 1e-9);

    // Rotate: quarter turns clockwise (Bismuth), back, flip, part.
    use azulejo_model::Orientation;
    let orientation = |e: &Engine| tab_of(e).layout.params.tile.orientation;
    run(&mut engine, Command::LayoutRotate(LayoutRotate::default()));
    assert_eq!(orientation(&engine), Orientation::Top);
    run(&mut engine, Command::LayoutRotate(LayoutRotate::default()));
    assert_eq!(orientation(&engine), Orientation::Right);
    run(&mut engine, Command::LayoutRotate(LayoutRotate::default()));
    assert_eq!(orientation(&engine), Orientation::Bottom);
    run(&mut engine, Command::LayoutRotate(LayoutRotate::default()));
    assert_eq!(orientation(&engine), Orientation::Left);
    let back = LayoutRotate {
        reverse: true,
        ..LayoutRotate::default()
    };
    run(&mut engine, Command::LayoutRotate(back.clone()));
    assert_eq!(orientation(&engine), Orientation::Bottom);
    run(&mut engine, Command::LayoutRotate(back));
    assert_eq!(orientation(&engine), Orientation::Right);
    let flip = LayoutRotate {
        flip: true,
        ..LayoutRotate::default()
    };
    run(&mut engine, Command::LayoutRotate(flip.clone()));
    assert_eq!(orientation(&engine), Orientation::Bottom);
    run(&mut engine, Command::LayoutRotate(flip));
    assert_eq!(orientation(&engine), Orientation::Right);
    assert!(
        engine
            .execute(Command::LayoutRotate(LayoutRotate {
                flip: true,
                part: true,
                ..LayoutRotate::default()
            }))
            .is_err()
    );
    run(
        &mut engine,
        Command::LayoutRotate(LayoutRotate {
            part: true,
            ..LayoutRotate::default()
        }),
    );
    assert!(tab_of(&engine).layout.params.tile.rotate_part);

    // Kinds: set, toggle back, cycle.
    let events = run(
        &mut engine,
        Command::LayoutSet(LayoutSetKind {
            tab: Some(tab),
            kind: LayoutKind::ThreeColumn,
        }),
    );
    assert!(names(&events).contains(&"tab.updated".to_string()));
    assert_eq!(layout_event(&events).kind, LayoutKind::ThreeColumn);
    run(
        &mut engine,
        Command::LayoutToggle(LayoutSetKind {
            tab: None,
            kind: LayoutKind::ThreeColumn,
        }),
    );
    assert_eq!(tab_of(&engine).kind(), LayoutKind::Tile);
    run(
        &mut engine,
        Command::LayoutCycle(LayoutCycle { tab: None, step: 1 }),
    );
    assert_eq!(tab_of(&engine).kind(), LayoutKind::Monocle);

    // Monocle: focus only restacks, nothing resizes.
    let before = tab_of(&engine).placements();
    let events = run(
        &mut engine,
        Command::LayoutFocus(LayoutFocus {
            direction: Direction::Right,
        }),
    );
    let state = layout_event(&events);
    for (a, b) in before.iter().zip(&state.rects) {
        assert_eq!(a.rect, b.rect);
    }
    assert_eq!(state.rects.iter().filter(|p| p.visible).count(), 1);

    // Freeze to manual and back.
    run(
        &mut engine,
        Command::LayoutSet(LayoutSetKind {
            tab: None,
            kind: LayoutKind::Manual,
        }),
    );
    assert_eq!(tab_of(&engine).kind(), LayoutKind::Manual);
    assert!(tab_of(&engine).layout.order.is_empty());
    engine.session().check_invariants().unwrap();
}

#[test]
fn layout_get_reports_manual_tabs_with_tree_rects() {
    let mut engine = Engine::new();
    split(&mut engine, Direction::Right);
    let Reply::Layout(state) = engine.execute(Command::LayoutGet).unwrap().reply else {
        panic!()
    };
    assert_eq!(state.kind, LayoutKind::Manual);
    assert!(state.order.is_empty());
    assert_eq!(state.rects.len(), 2);
    assert!(state.rects.iter().all(|p| p.visible));
}

fn set_sidebar(engine: &mut Engine, visible: bool) -> Vec<Event> {
    let window = engine.session().windows[0].id;
    engine
        .execute(Command::WindowSetSidebar(WindowSetSidebar {
            window,
            visible,
        }))
        .unwrap()
        .events
}

fn sidebar_update(events: &[Event]) -> (bool, SidebarState) {
    match events {
        [
            Event::WindowUpdated {
                sidebar_hidden,
                sidebar,
                ..
            },
        ] => (*sidebar_hidden, *sidebar),
        other => panic!("expected one window.updated, got {other:?}"),
    }
}

#[test]
fn a_collapsed_sidebar_is_a_rail_by_default() {
    let mut engine = Engine::new();
    assert_eq!(engine.session().windows[0].sidebar, SidebarState::Full);
    let events = set_sidebar(&mut engine, false);
    assert_eq!(sidebar_update(&events), (true, SidebarState::Rail));
    let events = set_sidebar(&mut engine, true);
    assert_eq!(sidebar_update(&events), (false, SidebarState::Full));
    // Collapsing twice changes nothing the second time.
    set_sidebar(&mut engine, false);
    assert!(set_sidebar(&mut engine, false).is_empty());
}

#[test]
fn the_collapsed_setting_moves_collapsed_sidebars_over() {
    let mut engine = Engine::new();
    set_sidebar(&mut engine, false);
    let events = engine.set_sidebar_collapsed(SidebarState::Hidden);
    assert_eq!(sidebar_update(&events), (true, SidebarState::Hidden));
    // Open, then collapsed again: hidden now.
    set_sidebar(&mut engine, true);
    let events = set_sidebar(&mut engine, false);
    assert_eq!(sidebar_update(&events), (true, SidebarState::Hidden));
    // Back to a rail; an open sidebar stays open.
    let events = engine.set_sidebar_collapsed(SidebarState::Rail);
    assert_eq!(sidebar_update(&events), (true, SidebarState::Rail));
    set_sidebar(&mut engine, true);
    assert!(
        engine
            .set_sidebar_collapsed(SidebarState::Hidden)
            .is_empty()
    );
    assert_eq!(engine.session().windows[0].sidebar, SidebarState::Full);
    // `Full` isn't a collapsed form: it counts as a rail.
    engine.set_sidebar_collapsed(SidebarState::Full);
    let events = set_sidebar(&mut engine, false);
    assert_eq!(sidebar_update(&events), (true, SidebarState::Rail));
}

#[test]
fn a_sidebar_saved_before_the_rail_comes_back_collapsed() {
    // State written before the rail: `sidebar_hidden` alone.
    let mut saved = serde_json::to_value(Engine::new().session()).unwrap();
    saved["windows"][0]["sidebar_hidden"] = true.into();
    assert!(saved["windows"][0].get("sidebar").is_none());
    let session: Session = serde_json::from_value(saved).unwrap();
    let mut engine = Engine::with_session(session);
    assert_eq!(engine.session().windows[0].sidebar, SidebarState::Rail);
    // The config says hidden: it follows.
    let events = engine.set_sidebar_collapsed(SidebarState::Hidden);
    assert_eq!(sidebar_update(&events), (true, SidebarState::Hidden));
    // It saves the form next to the old flag.
    let saved = serde_json::to_value(engine.session()).unwrap();
    assert_eq!(saved["windows"][0]["sidebar_hidden"], true);
    assert_eq!(saved["windows"][0]["sidebar"], "hidden");
    // `window.list` says how it shows.
    let Reply::Windows { windows } = engine.execute(Command::WindowList).unwrap().reply else {
        panic!("expected windows");
    };
    assert_eq!(windows[0].sidebar, SidebarState::Hidden);
}

#[test]
fn set_ratio_drags_a_divider_in_manual_and_dynamic_tabs() {
    // Manual: a | (b / c); the root's divider and the nested one.
    let mut engine = Engine::new();
    split(&mut engine, Direction::Right);
    split(&mut engine, Direction::Down);
    let set = |path: &str, ratio: Option<f64>| {
        Command::LayoutSetRatio(LayoutSetRatio {
            tab: None,
            path: path.into(),
            ratio,
        })
    };
    let events = run(&mut engine, set("", Some(0.3)));
    assert!(names(&events).contains(&"layout.changed".to_string()));
    run(&mut engine, set("b", Some(0.25)));
    assert_eq!(tab_of(&engine).root.ratios(), vec![0.3, 0.25]);
    // Evened out: a | (b / c) is two in a row, so the root goes to a half.
    run(&mut engine, set("", None));
    assert_eq!(tab_of(&engine).root.ratios(), vec![0.5, 0.25]);
    // A bad path is the caller's mistake and changes nothing.
    let before = tab_of(&engine).root.clone();
    let err = engine.execute(set("ab", Some(0.4))).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParams);
    let err = engine.execute(set("x", Some(0.4))).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParams);
    assert_eq!(tab_of(&engine).root, before);

    // Tile: the master edge is the master ratio; a stack edge only resizes
    // the two tiles beside it.
    let mut engine = tile_engine();
    for _ in 0..3 {
        run(
            &mut engine,
            Command::LayoutSplitAuto(LayoutSplitAuto::default()),
        );
    }
    run(&mut engine, set("", Some(0.65)));
    assert!((tab_of(&engine).layout.params.tile.ratio - 0.65).abs() < 1e-9);
    let last = *tab_of(&engine).layout.order.last().unwrap();
    let height = |e: &Engine, t| {
        tab_of(e)
            .root
            .rects()
            .into_iter()
            .find(|(id, _)| *id == t)
            .unwrap()
            .1
            .height
    };
    let before = height(&engine, last);
    run(&mut engine, set("b", Some(0.5)));
    assert_eq!(tab_of(&engine).layout.kind, LayoutKind::Tile);
    assert!((height(&engine, last) - before).abs() < 1e-9);

    // Monocle has no dividers.
    run(
        &mut engine,
        Command::LayoutSet(LayoutSetKind {
            tab: None,
            kind: LayoutKind::Monocle,
        }),
    );
    assert!(engine.execute(set("", Some(0.4))).is_err());
}
