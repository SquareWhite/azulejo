use std::collections::BTreeSet;

use azulejo_protocol::*;

fn sample_commands() -> Vec<Command> {
    let window = WindowId::new();
    let workspace = WorkspaceId::new();
    let tab = TabId::new();
    let tile = TileId::new();
    let herdr = HerdrRef {
        pane_id: "w1:p1".into(),
        terminal_id: "term_abc".into(),
        tab_id: "w1:t1".into(),
    };
    vec![
        Command::Ping,
        Command::SessionGet,
        Command::WindowCreate(WindowCreate {
            workspace_name: Some("main".into()),
            cwd: Some("/tmp".into()),
        }),
        Command::WindowClose(WindowTarget { window }),
        Command::WindowFocus(WindowTarget { window }),
        Command::WindowSetFrame(WindowSetFrame {
            window,
            frame: Frame {
                x: 10.0,
                y: 20.0,
                width: 800.0,
                height: 600.0,
            },
        }),
        Command::WorkspaceCreate(WorkspaceCreate {
            window: Some(window),
            name: Some("api".into()),
            cwd: None,
        }),
        Command::WorkspaceClose(WorkspaceTarget {
            workspace: Some(workspace),
        }),
        Command::WorkspaceFocus(WorkspaceTarget { workspace: None }),
        Command::WorkspaceRename(WorkspaceRename {
            workspace: None,
            name: "web".into(),
        }),
        Command::WorkspaceMove(WorkspaceMove {
            workspace: Some(workspace),
            index: 2,
        }),
        Command::WorkspaceBind(WorkspaceBind {
            workspace,
            herdr_workspace: "w3".into(),
        }),
        Command::TabCreate(TabCreate {
            workspace: None,
            name: Some("dev".into()),
            content: Some(TileContent::Browser {
                url: Some("http://localhost:3000".into()),
            }),
        }),
        Command::TabClose(TabTarget { tab: Some(tab) }),
        Command::TabFocus(TabTarget { tab: Some(tab) }),
        Command::TabRename(TabRename {
            tab: None,
            name: None,
        }),
        Command::TabMove(TabMove {
            tab: Some(tab),
            index: 0,
        }),
        Command::TileList(TileList {
            workspace: Some(workspace),
        }),
        Command::TileGet(TileTarget { tile: Some(tile) }),
        Command::TileClose(TileClose {
            tile: Some(tile),
            kill: true,
        }),
        Command::TileFocus(TileTarget { tile: Some(tile) }),
        Command::LayoutGet,
        Command::LayoutSplit(LayoutSplit {
            tile: None,
            direction: Direction::Right,
            content: Some(TileContent::Preview {
                path: Some("plan.md".into()),
            }),
        }),
        Command::LayoutFocus(LayoutFocus {
            direction: Direction::Up,
        }),
        Command::LayoutSwap(LayoutSwap {
            tile: None,
            direction: None,
            with: Some(tile),
        }),
        Command::LayoutMove(LayoutMove {
            tile: Some(tile),
            direction: Direction::Left,
        }),
        Command::LayoutResize(LayoutResize {
            tile: None,
            direction: Direction::Down,
            amount: -0.1,
        }),
        Command::LayoutZoom(LayoutZoom {
            tile: None,
            zoomed: Some(true),
        }),
        Command::LayoutSplitAuto(LayoutSplitAuto {
            tab: Some(tab),
            content: None,
        }),
        Command::LayoutPromote(TileTarget { tile: None }),
        Command::LayoutRotate(TabTarget { tab: None }),
        Command::LayoutEqualize(TabTarget { tab: Some(tab) }),
        Command::TerminalBind(TerminalBind {
            tile,
            herdr: herdr.clone(),
        }),
        Command::TerminalSendText(TerminalSendText {
            tile: None,
            text: "ls\n".into(),
        }),
        Command::TerminalSendKeys(TerminalSendKeys {
            tile: Some(tile),
            keys: vec!["ctrl+c".into(), "enter".into()],
        }),
        Command::TerminalRead(TerminalRead {
            tile: None,
            source: ReadSource::RecentUnwrapped,
            lines: Some(50),
        }),
        Command::TerminalScroll(TerminalScroll {
            tile: Some(tile),
            offset_from_bottom: 120,
        }),
        Command::TerminalSearch(TerminalSearch {
            tile: None,
            query: "error".into(),
            case_sensitive: true,
            lines: Some(2000),
        }),
        Command::WindowList,
        Command::WorkspaceList(WorkspaceList {
            window: Some(window),
        }),
        Command::TabList(TabList { workspace: None }),
        Command::TileRename(TileRename {
            tile: Some(tile),
            title: Some("server".into()),
        }),
        Command::BrowserOpen(BrowserOpen {
            tile: None,
            url: "http://localhost:3000".into(),
        }),
        Command::PreviewOpen(PreviewOpen {
            tile: Some(tile),
            path: "plan.md".into(),
        }),
    ]
}

fn sample_events() -> Vec<Event> {
    let window = WindowId::new();
    let workspace = WorkspaceId::new();
    let tab = TabId::new();
    let tile = Tile::new(TileContent::Terminal {
        cwd: Some("~/src".into()),
        command: Some(vec!["claude".into()]),
        herdr: Some(HerdrRef {
            pane_id: "w1:p2".into(),
            terminal_id: "term_x".into(),
            tab_id: "w1:t2".into(),
        }),
        ended: false,
    });
    let info = WorkspaceInfo {
        id: workspace,
        name: "main".into(),
        cwd: None,
        herdr_workspace: Some("w1".into()),
    };
    vec![
        Event::WindowCreated { window },
        Event::WindowClosed { window },
        Event::WindowUpdated {
            window,
            frame: None,
        },
        Event::WorkspaceCreated {
            window,
            workspace: info.clone(),
        },
        Event::WorkspaceClosed { window, workspace },
        Event::WorkspaceUpdated {
            window,
            workspace: info,
        },
        Event::WorkspacesReordered {
            window,
            order: vec![workspace],
        },
        Event::TabCreated {
            workspace,
            tab: TabInfo {
                id: tab,
                name: None,
            },
        },
        Event::TabClosed { workspace, tab },
        Event::TabUpdated {
            workspace,
            tab: TabInfo {
                id: tab,
                name: Some("x".into()),
            },
        },
        Event::TabsReordered {
            workspace,
            order: vec![tab],
        },
        Event::TileCreated {
            tab,
            tile: tile.clone(),
        },
        Event::TileUpdated {
            tab,
            tile: tile.clone(),
        },
        Event::TileClosed {
            tab,
            tile: tile.clone(),
            kill: false,
        },
        Event::LayoutChanged(LayoutState {
            window,
            workspace,
            tab,
            tree: Node::leaf(tile.id),
            focused: tile.id,
            zoomed: Some(tile.id),
        }),
        Event::FocusChanged(Focus {
            window: Some(window),
            workspace: Some(workspace),
            tab: Some(tab),
            tile: Some(tile.id),
        }),
    ]
}

#[test]
fn every_command_round_trips_and_samples_cover_every_method() {
    let mut covered = BTreeSet::new();
    for command in sample_commands() {
        let json = serde_json::to_string(&command).unwrap();
        assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), command);
        assert_eq!(Command::from_json(&json).unwrap(), command);
        let method = command.method();
        assert!(METHODS.contains(&method), "{method} missing from METHODS");
        covered.insert(method);
    }
    let all: BTreeSet<_> = METHODS.iter().copied().collect();
    assert_eq!(covered, all, "add a sample for every method");
}

#[test]
fn every_event_round_trips() {
    let mut names = BTreeSet::new();
    for event in sample_events() {
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
        names.insert(event.name());
    }
    assert_eq!(names.len(), sample_events().len());
}

#[test]
fn replies_round_trip() {
    let tile = Tile::new(TileContent::Files {
        root: Some(".".into()),
    });
    let replies = vec![
        Reply::Ok,
        Reply::Pong { api_version: 1 },
        Reply::Session {
            session: Session::new(),
        },
        Reply::Created(Created {
            tab: Some(TabId::new()),
            tile: Some(tile.id),
            ..Created::default()
        }),
        Reply::Tiles {
            tiles: vec![TileInfo {
                window: WindowId::new(),
                workspace: WorkspaceId::new(),
                tab: TabId::new(),
                focused: true,
                tile: tile.clone(),
            }],
        },
        Reply::Text {
            text: "$ ls".into(),
        },
        Reply::Layout(LayoutState {
            window: WindowId::new(),
            workspace: WorkspaceId::new(),
            tab: TabId::new(),
            tree: Node::leaf(tile.id),
            focused: tile.id,
            zoomed: None,
        }),
        Reply::Tile(TileInfo {
            window: WindowId::new(),
            workspace: WorkspaceId::new(),
            tab: TabId::new(),
            focused: false,
            tile: tile.clone(),
        }),
        Reply::Windows {
            windows: vec![WindowSummary {
                id: WindowId::new(),
                focused: true,
                active_workspace: WorkspaceId::new(),
                workspaces: vec![WorkspaceId::new()],
                frame: None,
            }],
        },
        Reply::Workspaces {
            workspaces: vec![WorkspaceSummary {
                window: WindowId::new(),
                active: true,
                focused: false,
                tabs: 2,
                workspace: WorkspaceInfo {
                    id: WorkspaceId::new(),
                    name: "web".into(),
                    cwd: None,
                    herdr_workspace: Some("w2".into()),
                },
            }],
        },
        Reply::Tabs {
            tabs: vec![TabSummary {
                window: WindowId::new(),
                workspace: WorkspaceId::new(),
                active: true,
                focused: true,
                tiles: 3,
                focused_tile: tile.id,
                tab: TabInfo {
                    id: TabId::new(),
                    name: Some("dev".into()),
                },
            }],
        },
        Reply::Scroll(ScrollState {
            offset_from_bottom: 10,
            max_offset_from_bottom: 400,
            viewport_rows: 40,
        }),
        Reply::Matches {
            matches: vec![SearchMatch {
                line: 3,
                text: "error: boom".into(),
            }],
            truncated: true,
        },
    ];
    for reply in replies {
        let json = serde_json::to_string(&reply).unwrap();
        assert_eq!(serde_json::from_str::<Reply>(&json).unwrap(), reply);
    }
}

#[test]
fn tile_info_is_flat() {
    let tile = Tile::new(TileContent::Browser {
        url: Some("http://localhost".into()),
    });
    let info = TileInfo {
        window: WindowId::new(),
        workspace: WorkspaceId::new(),
        tab: TabId::new(),
        focused: false,
        tile: tile.clone(),
    };
    let value = serde_json::to_value(&info).unwrap();
    assert_eq!(value["kind"], "browser");
    assert_eq!(value["id"], tile.id.to_string());
    assert_eq!(value["url"], "http://localhost");
}

#[test]
fn params_may_be_omitted_when_all_optional() {
    assert_eq!(
        Command::from_json(r#"{"method":"tile.close"}"#).unwrap(),
        Command::TileClose(TileClose::default())
    );
    assert_eq!(
        Command::from_json(r#"{"method":"layout.get"}"#).unwrap(),
        Command::LayoutGet
    );
    assert_eq!(
        Command::from_json(r#"{"method":"layout.resize","params":{"direction":"left"}}"#).unwrap(),
        Command::LayoutResize(LayoutResize {
            tile: None,
            direction: Direction::Left,
            amount: DEFAULT_RESIZE_STEP,
        })
    );
}

#[test]
fn bad_input_maps_to_typed_errors() {
    let code = |text: &str| Command::from_json(text).unwrap_err().code;
    assert_eq!(code("{nope"), ErrorCode::ParseError);
    assert_eq!(code(r#"{"params":{}}"#), ErrorCode::InvalidRequest);
    assert_eq!(
        code(r#"{"method":"tile.explode"}"#),
        ErrorCode::MethodNotFound
    );
    assert_eq!(
        code(r#"{"method":"layout.split","params":{"direction":"sideways"}}"#),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        code(r#"{"method":"tile.focus","params":{"tile":"tab_01JBX3V8Q3M9W2N5K7R4T6Y8ZA"}}"#),
        ErrorCode::InvalidParams
    );
    assert_eq!(ErrorCode::MethodNotFound.rpc_code(), -32601);
}

#[test]
fn shells_split_json_still_parses() {
    // The JSON the 0.6 shells send.
    assert_eq!(
        Command::from_json(r#"{"method":"layout.split","params":{"direction":"down"}}"#).unwrap(),
        Command::LayoutSplit(LayoutSplit {
            tile: None,
            direction: Direction::Down,
            content: None,
        })
    );
}

#[test]
fn schema_lists_exactly_the_methods() {
    let schema = api_schema();
    let consts: BTreeSet<String> = schema["$defs"]["Command"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            v["properties"]["method"]["const"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let methods: BTreeSet<String> = METHODS.iter().map(|m| m.to_string()).collect();
    assert_eq!(consts, methods);
}

#[test]
fn committed_schema_is_fresh() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../schema/api.v1.json");
    let committed = std::fs::read_to_string(path).unwrap_or_default();
    assert!(
        committed == api_schema_text(),
        "schema/api.v1.json is stale: run `just schema` and commit the result"
    );
}
