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
        machine: None,
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
        Command::WindowSetSidebar(WindowSetSidebar {
            window,
            visible: false,
        }),
        Command::WorkspaceCreate(WorkspaceCreate {
            window: Some(window),
            name: Some("api".into()),
            cwd: None,
            machine: Some("dev-server".into()),
        }),
        Command::WorkspaceClose(WorkspaceTarget {
            workspace: Some(workspace),
        }),
        Command::WorkspaceFocus(WorkspaceTarget { workspace: None }),
        Command::WorkspaceRename(WorkspaceRename {
            workspace: None,
            name: "web".into(),
        }),
        Command::WorkspaceSetRepo(WorkspaceSetRepo {
            workspace: Some(workspace),
            path: Some("/src/shop".into()),
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
                page_title: None,
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
        Command::TileSeen(TileTarget { tile: None }),
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
        Command::LayoutRotate(LayoutRotate {
            tab: None,
            reverse: Some(true),
            part: false,
        }),
        Command::LayoutSet(LayoutSetKind {
            tab: Some(tab),
            kind: LayoutKind::ThreeColumn,
        }),
        Command::LayoutToggle(LayoutSetKind {
            tab: None,
            kind: LayoutKind::Monocle,
        }),
        Command::LayoutCycle(LayoutCycle {
            tab: None,
            step: -1,
        }),
        Command::LayoutMasters(LayoutMasters {
            tab: None,
            delta: 2,
        }),
        Command::LayoutMasterRatio(LayoutMasterRatio {
            tab: None,
            delta: -0.05,
        }),
        Command::LayoutMoveOrder(LayoutMoveOrder {
            tile: Some(tile),
            step: 1,
        }),
        Command::LayoutEqualize(TabTarget { tab: Some(tab) }),
        Command::TerminalBind(TerminalBind {
            tile,
            herdr: herdr.clone(),
        }),
        Command::TerminalRespawn(TileTarget { tile: Some(tile) }),
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
        Command::EventsSubscribe(EventsSubscribe {
            filter: vec!["tile.".into(), "focus.changed".into()],
            workspace: Some(workspace),
            tile: Some(tile),
        }),
        Command::EventsUnsubscribe(EventsUnsubscribe { subscription: 3 }),
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
        Command::BrowserState(BrowserState {
            tile: Some(tile),
            url: Some("http://localhost:3000/docs".into()),
            page_title: Some("Docs".into()),
        }),
        Command::PreviewOpen(PreviewOpen {
            tile: Some(tile),
            path: "plan.md".into(),
        }),
        Command::PreviewGet(TileTarget { tile: Some(tile) }),
        Command::PreviewSetMode(PreviewSetMode {
            tile: Some(tile),
            mode: Some(PreviewMode::Source),
        }),
        Command::FilesList(FilesDir {
            tile: Some(tile),
            dir: None,
        }),
        Command::FilesExpand(FilesDir {
            tile: None,
            dir: Some("src".into()),
        }),
        Command::FilesCollapse(FilesDir {
            tile: Some(tile),
            dir: Some("/src/shop/src".into()),
        }),
        Command::FilesSetRoot(FilesSetRoot {
            tile: Some(tile),
            root: "~/src/shop".into(),
        }),
        Command::FilesReveal(FilesReveal {
            tile: None,
            path: "src/main.rs".into(),
        }),
        Command::FilesToggleHidden(FilesToggleHidden {
            tile: Some(tile),
            show: Some(true),
        }),
        Command::FilesOpen(FilesOpen {
            tile: Some(tile),
            path: "README.md".into(),
            target: FilesOpenTarget::Terminal,
        }),
        Command::LayoutApply(LayoutApply {
            path: Some("/src/shop/.azulejo.toml".into()),
            mode: ApplyMode::ReplaceTab,
            ..LayoutApply::default()
        }),
        Command::LayoutApply(LayoutApply {
            toml: Some("[[tile]]\n".into()),
            base_dir: Some("/src/shop".into()),
            workspace: Some(workspace),
            mode: ApplyMode::NewTab,
            ..LayoutApply::default()
        }),
        Command::LayoutExport(LayoutExport {
            tab: None,
            workspace: Some(workspace),
            path: Some("/src/shop/.azulejo.toml".into()),
        }),
        Command::CommandsList,
        Command::MachinesList,
    ]
}

fn sample_preview(tile: TileId) -> PreviewContent {
    PreviewContent {
        tile,
        path: "/src/plan.md".into(),
        kind: PreviewKind::Markdown,
        html: Some("<!doctype html><main id=\"azj-content\"></main>".into()),
        language: None,
        truncated: false,
        mode: PreviewMode::Source,
        revision: 3,
    }
}

fn sample_events() -> Vec<Event> {
    let window = WindowId::new();
    let workspace = WorkspaceId::new();
    let tab = TabId::new();
    let tile = Tile::new(TileContent::Terminal {
        cwd: Some("~/src".into()),
        command: Some(vec!["claude".into()]),
        env: [("RUST_LOG".to_owned(), "debug".to_owned())].into(),
        herdr: Some(HerdrRef {
            pane_id: "w1:p2".into(),
            terminal_id: "term_x".into(),
            tab_id: "w1:t2".into(),
            machine: None,
        }),
        ended: false,
        on_exit: Some(azulejo_protocol::OnExit::Keep),
    });
    let info = WorkspaceInfo {
        id: workspace,
        name: "main".into(),
        cwd: None,
        herdr_workspace: Some("w1".into()),
        blocked: 1,
        attention: 2,
        repo: Some("/src/shop".into()),
        git: Some(GitInfo {
            repo: "/src/shop".into(),
            branch: Some("main".into()),
            detached: false,
            dirty: true,
        }),
    };
    vec![
        Event::WindowCreated { window },
        Event::WindowClosed { window },
        Event::WindowUpdated {
            window,
            frame: None,
            sidebar_hidden: true,
        },
        Event::WorkspaceCreated {
            window,
            workspace: info.clone(),
        },
        Event::WorkspaceClosed { window, workspace },
        Event::WorkspaceGit {
            workspace,
            git: info.git.clone(),
        },
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
                kind: LayoutKind::Tile,
            },
        },
        Event::TabClosed { workspace, tab },
        Event::TabUpdated {
            workspace,
            tab: TabInfo {
                id: tab,
                name: Some("x".into()),
                kind: LayoutKind::Tile,
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
            kind: LayoutKind::Tile,
            params: LayoutParams::default(),
            order: vec![tile.id],
            rects: vec![Placement {
                tile: tile.id,
                rect: Rect::UNIT,
                z: 0,
                visible: true,
            }],
        }),
        Event::FocusChanged(Focus {
            window: Some(window),
            workspace: Some(workspace),
            tab: Some(tab),
            tile: Some(tile.id),
        }),
        Event::PreviewUpdated(sample_preview(tile.id)),
        Event::FilesChanged(FilesChanged {
            tile: tile.id,
            dirs: vec!["/src/shop".into(), "/src/shop/src".into()],
        }),
        Event::FilesRevealed(FilesRevealed {
            tile: tile.id,
            path: "/src/shop/src/main.rs".into(),
            dirs: vec!["/src/shop".into(), "/src/shop/src".into()],
        }),
        Event::FilesGit(FilesGit {
            tile: tile.id,
            work_tree: "/src/shop".into(),
            paths: [
                ("/src/shop/src".to_owned(), FileGitState::Modified),
                ("/src/shop/src/main.rs".to_owned(), FileGitState::Modified),
                ("/src/shop/notes".to_owned(), FileGitState::Untracked),
            ]
            .into(),
        }),
        Event::HerdrStatus {
            connected: false,
            message: Some("connection refused".into()),
        },
        Event::MachineStatus {
            machine: MachineInfo {
                id: "0123456789abcdef0123456789abcdef".into(),
                label: "dev-server".into(),
                state: MachineState::Reconnecting,
                message: Some("ssh: connect to host dev-server port 22: Connection refused".into()),
            },
        },
        Event::MachineRemoved {
            machine: "0123456789abcdef0123456789abcdef".into(),
        },
        Event::TileAttentionChanged {
            tab,
            tile: tile.id,
            attention: true,
        },
        Event::AgentStateChanged(AgentStateChange {
            tile: tile.id,
            state: AgentState::Blocked,
            previous: Some(AgentState::Working),
            agent: Some("claude".into()),
        }),
        Event::ConfigReloaded(ConfigReloaded {
            applied: false,
            path: Some("/home/me/.config/azulejo/config.toml".into()),
            diagnostics: vec![ConfigDiagnostic {
                severity: "error".into(),
                message: "not valid TOML".into(),
                file: Some("/home/me/.config/azulejo/config.toml".into()),
                line: Some(3),
                column: Some(7),
            }],
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
        Reply::Files(FilesListing {
            tile: tile.id,
            root: "/src/shop".into(),
            dir: "/src/shop".into(),
            show_hidden: true,
            entries: vec![
                FileEntry {
                    name: "src".into(),
                    path: "/src/shop/src".into(),
                    dir: true,
                    symlink: false,
                    hidden: false,
                    ignored: false,
                    git: Some(FileGitState::Modified),
                },
                FileEntry {
                    name: ".env".into(),
                    path: "/src/shop/.env".into(),
                    dir: false,
                    symlink: false,
                    hidden: true,
                    ignored: true,
                    git: None,
                },
            ],
            truncated: false,
        }),
        Reply::Layout(LayoutState {
            window: WindowId::new(),
            workspace: WorkspaceId::new(),
            tab: TabId::new(),
            tree: Node::leaf(tile.id),
            focused: tile.id,
            zoomed: None,
            kind: LayoutKind::Manual,
            params: LayoutParams::default(),
            order: Vec::new(),
            rects: Vec::new(),
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
                    blocked: 0,
                    attention: 0,
                    repo: None,
                    git: None,
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
                    kind: LayoutKind::Tile,
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
        Reply::Subscribed { subscription: 1 },
        Reply::Preview(sample_preview(tile.id)),
        Reply::Preview(PreviewContent {
            kind: PreviewKind::Pdf,
            html: None,
            ..sample_preview(tile.id)
        }),
        Reply::LayoutFile {
            toml: "[[tile]]\n".into(),
            path: None,
        },
        Reply::Commands {
            commands: vec![CommandEntry {
                id: "split_right".into(),
                title: "Split right".into(),
                category: "layout".into(),
                method: Some("layout.split".into()),
                action: Some("split_right".into()),
                index: None,
                chords: vec!["ctrl+cmd+v".into()],
                needs_tile: true,
                prompt: None,
                planned: false,
            }],
        },
        Reply::Machines {
            machines: vec![MachineInfo {
                id: LOCAL_MACHINE.into(),
                label: "Local".into(),
                state: MachineState::Connected,
                message: None,
            }],
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
        page_title: None,
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

#[test]
fn notifications_round_trip_with_the_event_inline() {
    let tile = TileId::new();
    let n = Notification::Event(EventNotification {
        subscription: 4,
        event: Event::AgentStateChanged(AgentStateChange {
            tile,
            state: AgentState::Done,
            previous: None,
            agent: None,
        }),
    });
    let json = serde_json::to_value(&n).unwrap();
    assert_eq!(json["method"], "events.event");
    assert_eq!(json["params"]["subscription"], 4);
    assert_eq!(json["params"]["event"], "agent.state_changed");
    assert_eq!(json["params"]["data"]["state"], "done");
    assert_eq!(serde_json::from_value::<Notification>(json).unwrap(), n);

    let lagged = Notification::Lagged(LaggedNotification {
        subscription: 4,
        dropped: 9,
    });
    let json = serde_json::to_value(&lagged).unwrap();
    assert_eq!(json["method"], "events.lagged");
    assert_eq!(json["params"]["dropped"], 9);
    assert_eq!(
        serde_json::from_value::<Notification>(json).unwrap(),
        lagged
    );
}

#[test]
fn unknown_agent_states_parse_as_unknown() {
    assert_eq!(
        serde_json::from_str::<AgentState>("\"thinking\"").unwrap(),
        AgentState::Unknown
    );
}
