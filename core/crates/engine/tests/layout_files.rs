//! `Core` around layout files, the command catalogue and broadcasts: the
//! parts the pure engine leaves to the running core (file I/O, config).

use std::sync::Arc;
use std::time::Duration;

use azulejo_engine::{Core, CoreOptions};
use azulejo_protocol::{
    ApplyMode, Command, CommandEntry, ConfigReloaded, ErrorCode, Event, LayoutApply, LayoutExport,
    Reply,
};

fn core() -> Core {
    Core::start(CoreOptions {
        state_path: None,
        api_socket: String::new(),
        log: Arc::new(|line| eprintln!("core: {line}")),
    })
}

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn applies_a_file_or_a_directory_and_exports_back_to_disk() {
    let core = core();
    let dir = tempdir();
    // macOS temp dirs sit behind a /var → /private/var link; use the real path
    // so the exported file's relative paths can be checked.
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("web")).unwrap();
    std::fs::write(
        root.join(".azulejo.toml"),
        "layout = \"tall\"\n[[tile]]\ncommand = \"nvim .\"\n[[tile]]\ncwd = \"web\"\n[[tile]]\npath = \"README.md\"\n",
    )
    .unwrap();

    // A directory means its `.azulejo.toml`.
    let reply = core
        .execute(Command::LayoutApply(LayoutApply {
            path: Some(root.display().to_string()),
            ..LayoutApply::default()
        }))
        .unwrap()
        .reply;
    let Reply::Created(created) = reply else {
        panic!("{reply:?}")
    };
    let session = core.session().unwrap();
    let ws = session.workspace(created.workspace.unwrap()).unwrap();
    assert_eq!(
        ws.name,
        root.file_name().unwrap().to_string_lossy(),
        "named after the directory"
    );
    assert_eq!(ws.cwd.as_deref(), Some(root.to_str().unwrap()));

    let out = root.join("exported.toml");
    let reply = core
        .execute(Command::LayoutExport(LayoutExport {
            workspace: Some(ws.id),
            path: Some(out.display().to_string()),
            ..LayoutExport::default()
        }))
        .unwrap()
        .reply;
    let Reply::LayoutFile { toml, path } = reply else {
        panic!()
    };
    assert_eq!(path.as_deref(), Some(out.to_str().unwrap()));
    let written = std::fs::read_to_string(&out).unwrap();
    assert_eq!(written, toml);
    assert!(written.contains("cwd = \"web\""), "{written}");
    assert!(written.contains("path = \"README.md\""), "{written}");

    // And the written file opens again.
    core.execute(Command::LayoutApply(LayoutApply {
        path: Some(out.display().to_string()),
        mode: ApplyMode::NewTab,
        ..LayoutApply::default()
    }))
    .unwrap();
    core.shutdown();
}

#[test]
fn file_errors_are_reported() {
    let core = core();
    let err = |command| core.execute(command).unwrap_err();
    let e = err(Command::LayoutApply(LayoutApply {
        path: Some("relative/.azulejo.toml".into()),
        ..LayoutApply::default()
    }));
    assert_eq!(e.code, ErrorCode::InvalidParams);
    assert!(e.message.contains("absolute"), "{}", e.message);

    let e = err(Command::LayoutApply(LayoutApply {
        path: Some("/no/such/dir/.azulejo.toml".into()),
        ..LayoutApply::default()
    }));
    assert!(e.message.contains("can't read"), "{}", e.message);

    let e = err(Command::LayoutApply(LayoutApply {
        path: Some("/tmp/x.toml".into()),
        toml: Some("[[tile]]".into()),
        ..LayoutApply::default()
    }));
    assert!(e.message.contains("exactly one"), "{}", e.message);

    let e = err(Command::LayoutExport(LayoutExport {
        path: Some("/no/such/dir/out.toml".into()),
        ..LayoutExport::default()
    }));
    assert!(e.message.contains("can't write"), "{}", e.message);
    core.shutdown();
}

#[test]
fn commands_list_comes_from_whoever_holds_the_config() {
    let core = core();
    assert_eq!(
        core.execute(Command::CommandsList).unwrap_err().code,
        ErrorCode::Unavailable
    );
    let entry = CommandEntry {
        id: "split_auto".into(),
        title: "New tile in stack (tall)".into(),
        category: "layout".into(),
        method: Some("layout.split_auto".into()),
        action: Some("split_auto".into()),
        index: None,
        chords: vec!["ctrl+cmd+t".into()],
        needs_tile: false,
        prompt: None,
        planned: false,
        context: None,
    };
    let list = vec![entry.clone()];
    core.set_commands(Arc::new(move || list.clone()));
    let Reply::Commands { commands } = core.execute(Command::CommandsList).unwrap().reply else {
        panic!()
    };
    assert_eq!(commands, vec![entry]);
    core.shutdown();
}

#[test]
fn broadcasts_reach_subscribers() {
    let core = core();
    let events = core.subscribe();
    let reloaded = ConfigReloaded {
        applied: true,
        path: None,
        diagnostics: Vec::new(),
    };
    core.broadcast(Event::ConfigReloaded(reloaded.clone()));
    let got = events.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(got, Event::ConfigReloaded(reloaded));
    core.shutdown();
}
