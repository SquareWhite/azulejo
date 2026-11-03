//! Layout files against a real herdr, in a throwaway named session: each
//! terminal tile's `command` really runs, with its `cwd` and `env`. Ignored by
//! default; run with `just test-herdr`. Never touches the user's default
//! session.

use std::sync::Arc;
use std::time::{Duration, Instant};

use azulejo_engine::{Core, CoreOptions};
use azulejo_herdr::{
    Backoff, Client, ClientOptions, Herdr, PaneId, ReadSource, Session as HerdrSession,
};
use azulejo_model::{Session, TileContent};
use azulejo_protocol::{Command, LayoutApply};

struct TempHerdr {
    rt: tokio::runtime::Runtime,
    herdr: Herdr,
    name: String,
    client: Client,
}

impl TempHerdr {
    fn start() -> Self {
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("HERDR_") {
                // SAFETY: tests run single-threaded (`--test-threads=1`).
                unsafe { std::env::remove_var(key) };
            }
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let herdr = Herdr::locate().expect("herdr on PATH");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let name = format!("azj-test-layout-{}-{nanos:x}", std::process::id());
        let options = ClientOptions {
            request_timeout: Duration::from_secs(5),
            connect: Backoff {
                initial: Duration::from_millis(50),
                max: Duration::from_millis(500),
                give_up_after: Duration::from_secs(10),
            },
        };
        let (client, _) = rt
            .block_on(herdr.ensure_server(&HerdrSession::named(&name), options))
            .expect("herdr server");
        Self {
            rt,
            herdr,
            name,
            client,
        }
    }

    fn read(&self, pane: &str) -> String {
        self.rt
            .block_on(
                self.client
                    .read(&PaneId(pane.into()), ReadSource::Recent, Some(200)),
            )
            .map(|r| r.text)
            .unwrap_or_default()
    }
}

impl Drop for TempHerdr {
    fn drop(&mut self) {
        let _ = self.rt.block_on(self.client.stop_server());
        std::thread::sleep(Duration::from_millis(500));
        let _ = self.rt.block_on(self.herdr.delete_session(&self.name));
    }
}

fn panes(session: &Session) -> Vec<(Option<String>, Option<String>)> {
    session
        .tiles()
        .filter_map(|(_, t)| match &t.content {
            TileContent::Terminal { herdr, .. } => {
                Some((t.title.clone(), herdr.as_ref().map(|h| h.pane_id.clone())))
            }
            _ => None,
        })
        .collect()
}

#[test]
#[ignore = "needs herdr; run with `just test-herdr`"]
fn layout_file_commands_run_in_their_panes() {
    let herdr = TempHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(
        root.join(".azulejo.toml"),
        r#"
layout = "tall"

[[tile]]
title = "line"
command = "echo AZJ-LINE-OK"

[[tile]]
title = "argv"
command = ["echo", "AZJ ARGV", "it's quoted"]

[[tile]]
title = "env"
cwd = "sub"
env = { AZJ_LAYOUT_PROBE = "probe-ok" }
command = "printenv AZJ_LAYOUT_PROBE; pwd"
"#,
    )
    .unwrap();

    let core = Core::start_with_herdr(
        CoreOptions {
            state_path: None,
            api_socket: "/tmp/azj-test.sock".into(),
            log: Arc::new(|line| eprintln!("core: {line}")),
        },
        herdr.client.clone(),
    );
    core.execute(Command::LayoutApply(LayoutApply {
        path: Some(root.display().to_string()),
        ..LayoutApply::default()
    }))
    .unwrap();

    // Each titled tile gets a pane, then its output shows up. Commands are
    // shell-neutral: the pane runs the user's shell (often fish).
    let sub = root.join("sub").display().to_string();
    let want = [
        ("line", "AZJ-LINE-OK"),
        ("argv", "AZJ ARGV it's quoted"),
        ("env", "probe-ok"),
        ("env", sub.as_str()),
    ];
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut missing: Vec<String> = Vec::new();
    while Instant::now() < deadline {
        let session = core.session().unwrap();
        let bound = panes(&session);
        missing = want
            .iter()
            .filter(|(title, text)| {
                let pane = bound
                    .iter()
                    .find(|(t, _)| t.as_deref() == Some(*title))
                    .and_then(|(_, p)| p.clone());
                // The echoed output, not the typed command line: the line
                // itself doesn't contain the expanded text.
                pane.is_none_or(|p| !herdr.read(&p).lines().any(|l| l.trim() == *text))
            })
            .map(|(title, _)| title.to_string())
            .collect();
        if missing.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if !missing.is_empty() {
        let session = core.session().unwrap();
        for (title, pane) in panes(&session) {
            if let Some(p) = pane {
                eprintln!("--- {title:?} ({p}):\n{}", herdr.read(&p));
            }
        }
        panic!("no expected output from {missing:?}");
    }
    core.shutdown();
}
