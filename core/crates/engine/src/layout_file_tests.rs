use super::*;
use crate::Engine;
use azulejo_protocol::{Command, Event};

const SCRIPTED: &str = r#"
name = "shop"
cwd = "app"
layout = "tall"

[[tile]]
title = "editor"
command = "nvim ."
focus = true

[[tile]]
command = ["cargo", "watch", "-x", "test"]
env = { RUST_LOG = "debug" }

[[tile]]
command = "npm run dev"
cwd = "web"

[[tile]]
url = "http://localhost:3000"
"#;

fn base() -> PathBuf {
    PathBuf::from("/projects/shop")
}

fn terminal(
    tile: &Tile,
) -> (
    Option<&str>,
    Option<&Vec<String>>,
    &BTreeMap<String, String>,
) {
    match &tile.content {
        TileContent::Terminal {
            cwd, command, env, ..
        } => (cwd.as_deref(), command.as_ref(), env),
        other => panic!("not a terminal: {other:?}"),
    }
}

#[test]
fn parses_the_scripted_layout_use_case() {
    let plan = parse(SCRIPTED, Some(&base())).unwrap();
    assert_eq!(plan.name.as_deref(), Some("shop"));
    assert_eq!(plan.cwd.as_deref(), Some("/projects/shop/app"));
    let [tab] = plan.tabs.as_slice() else {
        panic!("one tab")
    };
    let order = tab.root.tiles();
    assert_eq!(order.len(), 4);
    let t = |i: usize| tab.tile(order[i]).unwrap();

    assert_eq!(t(0).title.as_deref(), Some("editor"));
    assert_eq!(tab.focused, order[0]);
    let (cwd, cmd, _) = terminal(t(0));
    assert_eq!(cwd, Some("/projects/shop/app"), "file cwd is the default");
    assert_eq!(cmd.unwrap(), &vec!["nvim .".to_owned()]);

    let (_, cmd, env) = terminal(t(1));
    assert_eq!(cmd.unwrap().len(), 4);
    assert_eq!(env["RUST_LOG"], "debug");

    let (cwd, _, _) = terminal(t(2));
    assert_eq!(
        cwd,
        Some("/projects/shop/web"),
        "tile cwd is relative to the file"
    );

    assert!(matches!(
        &t(3).content,
        TileContent::Browser { url: Some(u), .. } if u == "http://localhost:3000"
    ));

    // Canonical tall: master | even vertical stack.
    let tall = tab.root.tall();
    assert!(tall.vertical);
    assert_eq!(tall.master, order[0]);
    assert_eq!(tall.stack, order[1..]);
    let mut even = tab.root.clone();
    assert!(!even.equalize(), "the stack is already even");
}

#[test]
fn explicit_trees_and_sizes() {
    let text = r#"
[[tab]]
name = "custom"
[tab.root]
split = "horizontal"
sizes = [60, 40]
[[tab.root.children]]
command = "htop"
[[tab.root.children]]
split = "vertical"
children = [{ path = "README.md" }, { root = "." }, { kind = "browser" }]

[[tab]]
layout = "rows"
sizes = [1, 3]
tile = [{}, {}]
"#;
    let plan = parse(text, Some(&base())).unwrap();
    assert_eq!(plan.tabs.len(), 2);
    let custom = &plan.tabs[0];
    assert_eq!(custom.name.as_deref(), Some("custom"));
    let Node::Split { axis, ratio, b, .. } = &custom.root else {
        panic!()
    };
    assert_eq!(*axis, Axis::Horizontal);
    assert!((ratio - 0.6).abs() < 1e-9);
    assert_eq!(b.leaf_count(), 3);
    let kinds: Vec<_> = custom
        .root
        .tiles()
        .iter()
        .map(|id| custom.tile(*id).unwrap().kind())
        .collect();
    use azulejo_model::TileKind::*;
    assert_eq!(kinds, [Terminal, Preview, Files, Browser]);
    let preview = custom.tile(custom.root.tiles()[1]).unwrap();
    assert!(
        matches!(&preview.content, TileContent::Preview { path: Some(p) } if p == "/projects/shop/README.md")
    );
    let files = custom.tile(custom.root.tiles()[2]).unwrap();
    assert!(
        matches!(&files.content, TileContent::Files { root: Some(p) } if p == "/projects/shop")
    );

    let rows = &plan.tabs[1];
    let Node::Split { axis, ratio, .. } = &rows.root else {
        panic!()
    };
    assert_eq!(*axis, Axis::Vertical);
    assert!((ratio - 0.25).abs() < 1e-9);
}

#[test]
fn diagnostics_say_where_and_what() {
    let cases = [
        ("", "no tiles"),
        ("layout = \"diagonal\"\n[[tile]]\n", "diagonal"),
        ("[[tile]]\ncommnd = \"x\"\n", "commnd"),
        (
            "[[tile]]\nurl = \"x\"\ncommand = \"y\"\n",
            "`command` only applies to terminal tiles",
        ),
        (
            "[[tile]]\nfocus = true\n[[tile]]\nfocus = true\n",
            "more than one tile",
        ),
        (
            "layout = \"columns\"\nsizes = [1]\n[[tile]]\n[[tile]]\n",
            "1 sizes for 2 children",
        ),
        (
            "layout = \"columns\"\nsizes = [1, 100]\n[[tile]]\n[[tile]]\n",
            "too uneven",
        ),
        (
            "layout = \"tall\"\nratio = 0.99\n[[tile]]\n[[tile]]\n",
            "`ratio` must be between",
        ),
        (
            "[root]\nsplit = \"vertical\"\nchildren = [{}]\n",
            "at least two children",
        ),
        (
            "[root]\nsplit = \"vertical\"\ncommand = \"x\"\nchildren = [{}, {}]\n",
            "a split has only",
        ),
        ("[[tile]]\n[[tab]]\n[[tab.tile]]\n", "not both"),
        ("version = 2\n[[tile]]\n", "version 2"),
        ("[[tile]]\nenv = { AZULEJO_TILE_ID = \"x\" }\n", "reserved"),
        (
            "[[tab]]\nname = \"dev\"\n[[tab]]\nname = \"ops\"\n",
            "tab 1 (`dev`): no tiles",
        ),
        ("[[tile]]\ncommand = []\n", "`command` is empty"),
        (
            "[[tile]]\nurl = \"x\"\non_exit = \"keep\"\n",
            "`on_exit` only applies to terminal tiles",
        ),
        ("[[tile]]\non_exit = \"linger\"\n", "linger"),
    ];
    for (text, want) in cases {
        let err = parse(text, None).expect_err(text);
        assert_eq!(err.code, ErrorCode::InvalidParams, "{text}");
        assert!(
            err.message.contains(want),
            "for {text:?}: wanted {want:?} in {:?}",
            err.message
        );
    }
    // toml's own errors carry the position.
    let err = parse("[[tile]]\ncommand = \"unterminated\n", None).unwrap_err();
    assert!(err.message.contains("line 2"), "{}", err.message);
}

#[test]
fn quoting() {
    assert_eq!(shell_quote("cargo"), "cargo");
    assert_eq!(shell_quote("a b"), "'a b'");
    assert_eq!(shell_quote("it's"), r"'it'\''s'");
    assert_eq!(shell_quote(""), "''");
    assert_eq!(
        command_line(&[
            "cargo".into(),
            "watch".into(),
            "-x".into(),
            "test --lib".into()
        ])
        .unwrap(),
        "cargo watch -x 'test --lib'"
    );
    assert_eq!(
        command_line(&["npm run dev".into()]).unwrap(),
        "npm run dev"
    );
    assert_eq!(command_line(&[]), None);
    // A one-word argv gets quoted at parse time, so it isn't read as a line.
    let plan = parse("[[tile]]\ncommand = [\"my prog\"]\n", None).unwrap();
    let (_, cmd, _) = terminal(&plan.tabs[0].tiles[0]);
    assert_eq!(cmd.unwrap(), &vec!["'my prog'".to_owned()]);
}

fn apply_cmd(text: &str, mode: ApplyMode) -> Command {
    Command::LayoutApply(LayoutApply {
        toml: Some(text.into()),
        base_dir: Some("/projects/shop".into()),
        mode,
        ..LayoutApply::default()
    })
}

#[test]
fn apply_new_workspace_creates_tiles_and_focuses_them() {
    let mut engine = Engine::with_session(Engine::new().session().clone());
    let before_ws = engine.session().windows[0].workspaces.len();
    let out = engine
        .execute(apply_cmd(SCRIPTED, ApplyMode::NewWorkspace))
        .unwrap();
    let Reply::Created(created) = out.reply else {
        panic!()
    };
    let s = engine.session();
    assert_eq!(s.windows[0].workspaces.len(), before_ws + 1);
    let ws = s.workspace(created.workspace.unwrap()).unwrap();
    assert_eq!(ws.name, "shop");
    assert_eq!(ws.cwd.as_deref(), Some("/projects/shop/app"));
    assert_eq!(s.focus().workspace, Some(ws.id));
    assert_eq!(s.focus().tab, created.tab);
    assert_eq!(s.focus().tile, created.tile);
    let created_tiles = out
        .events
        .iter()
        .filter(|e| matches!(e, Event::TileCreated { .. }))
        .count();
    assert_eq!(created_tiles, 4, "tile.created drives herdr panes");
    assert_eq!(s.check_invariants(), Ok(()));
}

#[test]
fn apply_new_tab_and_replace_tab() {
    let mut engine = Engine::new();
    let ws = engine.session().focus().workspace.unwrap();
    let old_tab = engine.session().focus().tab.unwrap();
    engine
        .execute(apply_cmd("[[tile]]\n[[tile]]\n", ApplyMode::NewTab))
        .unwrap();
    assert_eq!(engine.session().workspace(ws).unwrap().tabs.len(), 2);

    let active = engine.session().focus().tab.unwrap();
    assert_ne!(active, old_tab);
    let out = engine
        .execute(apply_cmd(
            "[[tab]]\n[[tab.tile]]\n[[tab]]\n[[tab.tile]]\n",
            ApplyMode::ReplaceTab,
        ))
        .unwrap();
    let tabs = &engine.session().workspace(ws).unwrap().tabs;
    assert_eq!(tabs.len(), 3, "one replaced by two");
    assert!(tabs.iter().all(|t| t.id != active));
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, Event::TileClosed { kill: false, .. })),
        "the replaced tab's tiles detach, never kill"
    );
    assert_eq!(engine.session().check_invariants(), Ok(()));
}

#[test]
fn apply_errors_leave_the_session_alone() {
    let mut engine = Engine::new();
    let before = engine.session().clone();
    assert!(
        engine
            .execute(apply_cmd("layout = 3", ApplyMode::NewWorkspace))
            .is_err()
    );
    let path_only = Command::LayoutApply(LayoutApply {
        path: Some("/x/.azulejo.toml".into()),
        ..LayoutApply::default()
    });
    assert_eq!(
        engine.execute(path_only).unwrap_err().code,
        ErrorCode::Unavailable,
        "the pure engine can't read files"
    );
    let relative = Command::LayoutApply(LayoutApply {
        toml: Some("[[tile]]".into()),
        base_dir: Some("rel".into()),
        ..LayoutApply::default()
    });
    assert!(engine.execute(relative).is_err());
    assert_eq!(engine.session(), &before);
}

/// Everything about a tab that a layout file carries: the tree's shape and
/// ratios, and each tile's content, in tree order.
fn shape(tab: &Tab) -> (String, Vec<TileContent>, usize) {
    fn walk(n: &Node, out: &mut String) {
        match n {
            Node::Leaf { .. } => out.push('L'),
            Node::Split { axis, ratio, a, b } => {
                out.push_str(&format!("({axis:?} {:.4} ", ratio));
                walk(a, out);
                out.push(' ');
                walk(b, out);
                out.push(')');
            }
        }
    }
    let mut s = String::new();
    walk(&tab.root, &mut s);
    let order = tab.root.tiles();
    let contents = order
        .iter()
        .map(|id| tab.tile(*id).unwrap().content.clone())
        .collect();
    let focus = order.iter().position(|id| *id == tab.focused).unwrap();
    (s, contents, focus)
}

#[test]
fn export_then_apply_round_trips() {
    let texts = [
        SCRIPTED,
        r#"
[[tab]]
name = "a"
[tab.root]
split = "horizontal"
sizes = [20, 30, 50]
[[tab.root.children]]
title = "left"
[[tab.root.children]]
split = "vertical"
sizes = [70, 30]
children = [{ path = "/etc/hosts" }, { url = "https://example.com" }]
[[tab.root.children]]
command = ["echo", "hi there"]
on_exit = "keep"
focus = true

[[tab]]
layout = "tall"
ratio = 0.62
tile = [{ root = "/tmp" }, {}, {}]
"#,
    ];
    for text in texts {
        let mut engine = Engine::new();
        engine
            .execute(apply_cmd(text, ApplyMode::NewWorkspace))
            .unwrap();
        let ws = engine.session().focus().workspace.unwrap();
        let first: Vec<_> = engine
            .session()
            .workspace(ws)
            .unwrap()
            .tabs
            .iter()
            .map(shape)
            .collect();
        let exported = match engine
            .execute(Command::LayoutExport(LayoutExport::default()))
            .unwrap()
            .reply
        {
            Reply::LayoutFile { toml, .. } => toml,
            other => panic!("{other:?}"),
        };
        let mut again = Engine::new();
        again
            .execute(Command::LayoutApply(LayoutApply {
                toml: Some(exported.clone()),
                ..LayoutApply::default()
            }))
            .unwrap_or_else(|e| panic!("{e:?} applying\n{exported}"));
        let ws2 = again.session().focus().workspace.unwrap();
        let second: Vec<_> = again
            .session()
            .workspace(ws2)
            .unwrap()
            .tabs
            .iter()
            .map(shape)
            .collect();
        assert_eq!(first, second, "exported:\n{exported}");
        let names = |e: &Engine, w| {
            e.session()
                .workspace(w)
                .unwrap()
                .tabs
                .iter()
                .map(|t| t.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&engine, ws), names(&again, ws2));
    }
}

#[test]
fn export_of_one_tab_and_relative_paths() {
    let mut engine = Engine::new();
    engine
        .execute(apply_cmd(SCRIPTED, ApplyMode::NewWorkspace))
        .unwrap();
    let tab = engine.session().focus().tab.unwrap();
    let toml = match engine
        .execute(Command::LayoutExport(LayoutExport {
            tab: Some(tab),
            path: Some("/projects/shop/.azulejo.toml".into()),
            ..LayoutExport::default()
        }))
        .unwrap()
        .reply
    {
        Reply::LayoutFile { toml, path } => {
            assert_eq!(path.as_deref(), Some("/projects/shop/.azulejo.toml"));
            toml
        }
        other => panic!("{other:?}"),
    };
    assert!(toml.contains("layout = \"tall\""), "{toml}");
    assert!(
        toml.contains("cwd = \"web\""),
        "relative to the file: {toml}"
    );
    assert!(!toml.contains("/projects/shop/web"), "{toml}");
    assert!(!toml.contains("herdr"), "{toml}");
}

#[test]
fn adopted_style_trees_export_generically() {
    // A tree no shorthand covers still exports, as `root`.
    let mut engine = Engine::new();
    let s = engine.session();
    let tile = s.focus().tile.unwrap();
    engine
        .execute(Command::LayoutSplit(azulejo_protocol::LayoutSplit {
            tile: Some(tile),
            direction: azulejo_model::Direction::Down,
            content: None,
        }))
        .unwrap();
    engine
        .execute(Command::LayoutSplit(azulejo_protocol::LayoutSplit {
            tile: Some(tile),
            direction: azulejo_model::Direction::Right,
            content: None,
        }))
        .unwrap();
    let Reply::LayoutFile { toml, .. } = engine
        .execute(Command::LayoutExport(LayoutExport::default()))
        .unwrap()
        .reply
    else {
        panic!()
    };
    assert!(toml.contains("[root]"), "{toml}");
    let plan = parse(&toml, None).unwrap();
    assert_eq!(plan.tabs[0].tiles.len(), 3);
}

#[test]
fn the_documented_example_parses() {
    let text = include_str!("../../../../docs/examples/azulejo.toml");
    let plan = parse(text, Some(&base())).unwrap();
    let tab = &plan.tabs[0];
    assert_eq!(tab.tiles.len(), 4);
    let kinds: Vec<_> = tab
        .root
        .tiles()
        .iter()
        .map(|id| tab.tile(*id).unwrap().kind())
        .collect();
    use azulejo_model::TileKind::*;
    assert_eq!(kinds, [Terminal, Terminal, Terminal, Browser]);
}

#[test]
fn on_exit_per_tile_parses_and_round_trips() {
    let text = "[[tile]]\ncommand = \"npm run dev\"\non_exit = \"keep\"\n[[tile]]\n";
    let plan = parse(text, Some(&base())).unwrap();
    let tab = &plan.tabs[0];
    let on_exit = |i: usize| match &tab.tile(tab.root.tiles()[i]).unwrap().content {
        TileContent::Terminal { on_exit, .. } => *on_exit,
        other => panic!("not a terminal: {other:?}"),
    };
    assert_eq!(on_exit(0), Some(OnExit::Keep));
    assert_eq!(on_exit(1), None, "unset: the config decides");

    let mut engine = Engine::new();
    engine
        .execute(apply_cmd(text, ApplyMode::NewWorkspace))
        .expect("apply");
    let exported = match engine
        .execute(Command::LayoutExport(LayoutExport::default()))
        .unwrap()
        .reply
    {
        Reply::LayoutFile { toml, .. } => toml,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        exported.matches("on_exit = \"keep\"").count(),
        1,
        "{exported}"
    );
}
