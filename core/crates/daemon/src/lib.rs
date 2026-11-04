//! Boots Azulejo's core: config, saved state, the engine with herdr, and the
//! API server. The macOS shell gets this through the C ABI (`azj_core_new`);
//! `azulejod` runs it headless for development and end-to-end tests.
//!
//! By default Azulejo connects to herdr's default session, the one the user's
//! own herdr TUI and phone use (ADR 0008), and adopts what already runs
//! there. If that server isn't running, Azulejo starts it with a clean
//! environment (ADR 0006); a server the user started keeps whatever
//! environment it was started with. `[herdr] session = "NAME"` picks a named
//! session instead.
//!
//! herdr's saved machines come along too (ADR 0011, [`machines`]): each gets
//! its own link in the core, and terminal tiles on them attach over SSH.

mod machines;
mod watch;

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use azulejo_api::{CoreDispatcher, Server, ServerHandle, WithInfo};
use azulejo_config::{Config, HerdrConfig, Platform};
use azulejo_engine::{Core, CoreOptions, LogFn, persist};
use azulejo_herdr::{
    AttachCommand, Backoff, Client, ClientOptions, Herdr, PaneId, RemoteMachine, ServerOrigin,
    Session as HerdrSession, Ssh, TerminalId,
};
use serde_json::{Map, Value};

pub use machines::Source as MachineSource;
use machines::{MachineSync, Registry};
pub use watch::diagnostic;
use watch::{ConfigWatch, SharedConfig};

/// Which herdr session terminal tiles use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HerdrChoice {
    /// Whatever the config's `[herdr]` section says.
    FromConfig,
    /// No herdr: terminal tiles stay unbound.
    Off,
    /// This session, whatever the config says.
    Session(HerdrSession),
}

/// Which Ghostty config files the core reads for the settings it shares
/// with Ghostty (`unfocused-split-opacity`, `unfocused-split-fill`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhosttyFiles {
    /// Ghostty's own default locations.
    Default,
    /// This file only (a dev profile's); a missing file sets nothing.
    Only(PathBuf),
    /// None: Azulejo's own defaults.
    Off,
}

pub struct Options {
    /// The user's config file; `None` uses the bundled defaults only.
    pub config_path: Option<PathBuf>,
    /// Where the Ghostty settings Azulejo honours come from.
    pub ghostty: GhosttyFiles,
    /// Where the session is saved; `None` keeps it in memory.
    pub state_path: Option<PathBuf>,
    /// The API socket; `None` runs without one.
    pub socket_path: Option<PathBuf>,
    /// Fail to start when the socket can't be bound (another instance is
    /// running). Otherwise the app starts without an API and logs why.
    pub api_required: bool,
    pub herdr: HerdrChoice,
    pub platform: Platform,
    pub log: LogFn,
}

impl Default for Options {
    /// The real thing: the user's config, saved state, the platform socket
    /// and herdr per the config.
    fn default() -> Self {
        Options {
            config_path: azulejo_config::config_path(),
            ghostty: GhosttyFiles::Default,
            state_path: Some(persist::default_state_path()),
            socket_path: Some(azulejo_api::socket_path()),
            api_required: false,
            herdr: HerdrChoice::FromConfig,
            platform: Platform::current(),
            log: Arc::new(|line| eprintln!("azulejo: {line}")),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error("can't serve the API at {path}: {source}")]
    Api {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// The herdr server terminal tiles attach to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrInfo {
    pub herdr: Herdr,
    pub session: HerdrSession,
    pub socket: PathBuf,
}

impl HerdrInfo {
    /// What a terminal tile runs to view `terminal` (ADR 0001), with the clean
    /// environment herdr's children get (ADR 0006).
    pub fn attach(&self, terminal: &str, takeover: bool) -> AttachCommand {
        self.herdr
            .attach(&self.session, &TerminalId(terminal.to_owned()), takeover)
    }
}

/// Where a terminal tile's pane runs: Local, or one of herdr's saved
/// machines (ADR 0011).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalTarget {
    Local(HerdrInfo),
    Remote(RemoteMachine),
}

impl TerminalTarget {
    /// The attach command for a tile's PTY (`ssh -tt …` on a remote machine).
    pub fn attach(&self, terminal: &str, takeover: bool) -> AttachCommand {
        match self {
            TerminalTarget::Local(h) => h.attach(terminal, takeover),
            TerminalTarget::Remote(m) => m.attach(&TerminalId(terminal.to_owned()), takeover),
        }
    }

    /// The read-only view at the tile's size.
    pub fn observe(&self, pane: &str, cols: u16, rows: u16) -> AttachCommand {
        let pane = PaneId(pane.to_owned());
        match self {
            TerminalTarget::Local(h) => h.herdr.observe(&h.session, &pane, cols, rows),
            TerminalTarget::Remote(m) => m.observe(&pane, cols, rows),
        }
    }

    /// A client for asking herdr why an attach ended; answers quickly or
    /// not at all.
    pub fn diagnose_client(&self) -> Client {
        match self {
            TerminalTarget::Local(h) => Client::new(
                &h.socket,
                ClientOptions {
                    request_timeout: std::time::Duration::from_secs(2),
                    connect: Backoff {
                        initial: std::time::Duration::from_millis(50),
                        max: std::time::Duration::from_millis(200),
                        give_up_after: std::time::Duration::from_millis(500),
                    },
                },
            ),
            TerminalTarget::Remote(m) => Client::bridged(
                m.bridge(),
                ClientOptions {
                    request_timeout: std::time::Duration::from_secs(4),
                    connect: Backoff {
                        give_up_after: std::time::Duration::ZERO,
                        ..Backoff::default()
                    },
                },
            ),
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, TerminalTarget::Remote(_))
    }
}

/// A running core. Dropping it stops the API server, saves pending state and
/// stops the engine; herdr panes keep running.
pub struct App {
    core: Option<Arc<Core>>,
    server: Option<ServerHandle>,
    /// Swapped whole when the file changes (F25b).
    config: SharedConfig,
    watch: Option<ConfigWatch>,
    platform: Platform,
    herdr: Option<HerdrInfo>,
    /// herdr's saved machines terminal tiles can attach to.
    remote: Registry,
    machines: Option<MachineSync>,
    socket: Option<PathBuf>,
    state_path: Option<PathBuf>,
    ghostty_files: GhosttyFiles,
}

impl App {
    pub fn start(opts: Options) -> Result<App, BootError> {
        let log = opts.log.clone();
        let config = match &opts.config_path {
            Some(path) => azulejo_config::reload(path, opts.platform),
            None => Config::defaults(opts.platform),
        };
        for d in &config.diagnostics {
            log(format!("config: {d}"));
        }

        // Bind first: a second instance must fail before it touches herdr or
        // the saved state.
        let server = match &opts.socket_path {
            Some(path) => match Server::bind(path) {
                Ok(server) => Some(server),
                Err(source) if opts.api_required => {
                    return Err(BootError::Api {
                        path: path.clone(),
                        source,
                    });
                }
                Err(e) => {
                    log(format!(
                        "running without the API: can't bind {}: {e}",
                        path.display()
                    ));
                    None
                }
            },
            None => None,
        };
        let socket = server.as_ref().map(|s| s.path().to_owned());

        let session = match &opts.herdr {
            HerdrChoice::Off => None,
            HerdrChoice::Session(session) => Some(session.clone()),
            HerdrChoice::FromConfig => herdr_session(&config.herdr),
        };
        // herdr's saved machines: on with herdr's default session, off with a
        // named one unless the config says so (ADR 0011).
        let machines_on = session.as_ref().is_some_and(|s| {
            config.herdr.enabled && config.herdr.machines.unwrap_or(s.name().is_none())
        });
        let herdr = session.and_then(|session| match connect_herdr(&session) {
            Ok(found) => Some(found),
            Err(e) => {
                log(format!(
                    "running without herdr: terminal tiles stay unbound ({e})"
                ));
                None
            }
        });

        let core_opts = CoreOptions {
            state_path: opts.state_path.clone(),
            api_socket: socket
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            log: log.clone(),
        };
        let (core, herdr_info) = match herdr {
            Some((client, info, ServerOrigin::Running)) => (
                Core::start_with_running_herdr(core_opts, client),
                Some(info),
            ),
            Some((client, info, ServerOrigin::Started)) => {
                (Core::start_with_herdr(core_opts, client), Some(info))
            }
            None => (Core::start(core_opts), None),
        };
        let core = Arc::new(core);
        let config: SharedConfig = Arc::new(RwLock::new(Arc::new(config)));
        watch::push_settings(&core, &config);
        {
            let config = config.clone();
            core.set_commands(Arc::new(move || match config.read() {
                Ok(c) => azulejo_config::catalogue(&c),
                Err(_) => Vec::new(),
            }));
        }
        let watch = opts.config_path.as_deref().and_then(|path| {
            watch::watch(
                path,
                opts.platform,
                config.clone(),
                Arc::downgrade(&core),
                log.clone(),
            )
        });

        let remote: Registry = Default::default();
        let machines = if machines_on {
            let herdr_cfg = config.read().map(|c| c.herdr.clone()).unwrap_or_default();
            let source = match (&herdr_cfg.machines_file, &herdr_info) {
                (Some(file), _) => Some(machines::Source::File(file.clone())),
                (None, Some(h)) => Some(machines::Source::Herdr(h.herdr.clone())),
                (None, None) => Herdr::locate().ok().map(machines::Source::Herdr),
            };
            source.map(|source| {
                let mut ssh = Ssh::default();
                if let Some(program) = &herdr_cfg.ssh {
                    ssh.program = program.clone();
                }
                MachineSync::start(
                    &core,
                    source,
                    ssh,
                    config.clone(),
                    remote.clone(),
                    log.clone(),
                )
            })
        } else {
            None
        };

        let mut app = App {
            core: Some(core.clone()),
            server: None,
            config,
            watch,
            platform: opts.platform,
            herdr: herdr_info,
            remote,
            machines,
            socket,
            state_path: opts.state_path,
            ghostty_files: opts.ghostty,
        };
        if let Some(server) = server {
            let dispatcher = WithInfo::new(CoreDispatcher::new(core), app.info());
            match server.spawn(Arc::new(dispatcher)) {
                Ok(handle) => app.server = Some(handle),
                Err(source) => {
                    let path = app.socket.take().unwrap_or_default();
                    if opts.api_required {
                        return Err(BootError::Api { path, source });
                    }
                    log(format!("running without the API: {source}"));
                }
            }
        }
        Ok(app)
    }

    pub fn core(&self) -> &Core {
        self.core.as_ref().expect("core runs until shutdown")
    }

    /// The current config. Hold the `Arc` briefly: a reload swaps in a new one.
    pub fn config(&self) -> Arc<Config> {
        match self.config.read() {
            Ok(c) => c.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// The Ghostty settings Azulejo shares with Ghostty, read now (so a
    /// config reload picks up edits to Ghostty's file too).
    pub fn ghostty(&self) -> azulejo_config::ghostty::GhosttyConfig {
        use azulejo_config::ghostty;
        match &self.ghostty_files {
            GhosttyFiles::Default => ghostty::load(self.platform),
            GhosttyFiles::Only(path) => ghostty::load_from(std::slice::from_ref(path)),
            GhosttyFiles::Off => ghostty::GhosttyConfig::default(),
        }
    }

    /// Reads the config file again now (the `reload_config` action), as if
    /// it had been saved, and tells listeners. `None` without a config file.
    pub fn reload_config(&self) -> Option<azulejo_protocol::ConfigReloaded> {
        let path = self.config().path.clone()?;
        let log: LogFn = Arc::new(|line| eprintln!("azulejo: {line}"));
        let event = watch::reload(&path, self.platform, &self.config, &log)?;
        if let Some(core) = &self.core {
            watch::push_settings(core, &self.config);
            core.broadcast(azulejo_protocol::Event::ConfigReloaded(event.clone()));
        }
        Some(event)
    }

    /// Sets `[appearance] theme` in the config file (a theme id or
    /// `"auto"`), touching nothing else in it, and reloads. The file and
    /// its folder are created when missing. `Err` says why not: no config
    /// file, an unknown theme, a file the edit can't express, or I/O.
    pub fn set_theme(&self, theme: &str) -> Result<azulejo_protocol::ConfigReloaded, String> {
        let config = self.config();
        let path = config
            .path
            .clone()
            .ok_or("there's no config file to save the theme in")?;
        if theme != azulejo_config::theme::AUTO && config.themes.get(theme).is_none() {
            return Err(format!("unknown theme `{theme}`"));
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("can't read {}: {e}", path.display())),
        };
        let edited = azulejo_config::theme::set_theme_in(&text, theme).ok_or_else(|| {
            format!(
                "{} sets the theme in a way Azulejo can't edit; set `[appearance] theme = \"{theme}\"` there",
                path.display()
            )
        })?;
        if edited != text {
            let dir = path.parent().ok_or("the config path has no folder")?;
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("can't create {}: {e}", dir.display()))?;
            // Write beside it and rename over, as editors do: the watcher
            // never reads half a file.
            let tmp = dir.join(format!(".config.toml.azulejo-{}", std::process::id()));
            std::fs::write(&tmp, &edited)
                .and_then(|()| std::fs::rename(&tmp, &path))
                .map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    format!("can't save {}: {e}", path.display())
                })?;
        }
        self.reload_config()
            .ok_or_else(|| "the config didn't reload".to_owned())
    }

    pub fn herdr(&self) -> Option<&HerdrInfo> {
        self.herdr.as_ref()
    }

    /// Where a terminal tile on `machine` (a `HerdrRef.machine`: `None` or
    /// `"local"` for Local, else herdr's machine ID) attaches. `None` when
    /// that machine isn't known or connected here.
    pub fn terminal_target(&self, machine: Option<&str>) -> Option<TerminalTarget> {
        match machine.filter(|m| *m != azulejo_protocol::LOCAL_MACHINE) {
            None => self.herdr.clone().map(TerminalTarget::Local),
            Some(id) => self
                .remote
                .lock()
                .ok()
                .and_then(|r| r.get(id).cloned())
                .map(TerminalTarget::Remote),
        }
    }

    /// The API socket, when the API is being served.
    pub fn socket(&self) -> Option<&Path> {
        self.socket.as_deref()
    }

    /// What `core.version` reports beyond the version itself: where the API,
    /// state and herdr session live.
    pub fn info(&self) -> Map<String, Value> {
        let path = |p: Option<&Path>| p.map_or(Value::Null, |p| p.display().to_string().into());
        let mut info = Map::new();
        info.insert("socket".into(), path(self.socket.as_deref()));
        info.insert("state_path".into(), path(self.state_path.as_deref()));
        info.insert("config_path".into(), path(self.config().path.as_deref()));
        match &self.herdr {
            Some(h) => {
                let session = h.session.name().unwrap_or(HerdrConfig::HERDR_DEFAULT);
                info.insert("herdr_session".into(), session.into());
                info.insert("herdr_socket".into(), path(Some(&h.socket)));
                info.insert("herdr_bin".into(), path(Some(h.herdr.path())));
            }
            None => {
                info.insert("herdr_session".into(), Value::Null);
                info.insert("herdr_socket".into(), Value::Null);
                info.insert("herdr_bin".into(), Value::Null);
            }
        }
        info
    }

    /// Stops the API server (removing its socket), then saves and stops the
    /// core.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.machines.take();
        self.watch.take();
        if let Some(server) = self.server.take() {
            server.shutdown();
        }
        if let Some(core) = self.core.take() {
            // The server's dispatcher held the only other reference, and it's
            // gone now. If something still holds one, the core stops when that
            // goes.
            if let Ok(core) = Arc::try_unwrap(core) {
                core.shutdown();
            }
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop();
    }
}

fn herdr_session(config: &HerdrConfig) -> Option<HerdrSession> {
    config.enabled.then(|| match &config.session {
        Some(name) => HerdrSession::named(name),
        None => HerdrSession::default_session(),
    })
}

/// The core's client for the local herdr. Trying a Unix socket that isn't
/// there yet costs microseconds, so the event stream retries at least every
/// 250 ms while herdr is down: after a herdr restart, tiles reattach within
/// a quarter second of the server coming up, not up to a second later.
fn local_client_options() -> ClientOptions {
    ClientOptions {
        connect: Backoff {
            max: std::time::Duration::from_millis(250),
            ..Backoff::default()
        },
        ..ClientOptions::default()
    }
}

fn connect_herdr(
    session: &HerdrSession,
) -> Result<(azulejo_herdr::Client, HerdrInfo, ServerOrigin), azulejo_herdr::Error> {
    let herdr = Herdr::locate()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| azulejo_herdr::Error::Binary(format!("tokio runtime: {e}")))?;
    let (client, _, origin) =
        runtime.block_on(herdr.connect_or_start(session, local_client_options()))?;
    let info = HerdrInfo {
        socket: client
            .socket_path()
            .expect("a local herdr session has a socket")
            .to_owned(),
        herdr,
        session: session.clone(),
    };
    Ok((client, info, origin))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory() -> Options {
        Options {
            config_path: None,
            ghostty: GhosttyFiles::Off,
            state_path: None,
            socket_path: None,
            api_required: true,
            herdr: HerdrChoice::Off,
            platform: Platform::current(),
            log: Arc::new(|line| eprintln!("test: {line}")),
        }
    }

    #[test]
    fn starts_without_herdr_or_api() {
        let app = App::start(in_memory()).unwrap();
        assert!(app.herdr().is_none());
        assert!(app.socket().is_none());
        assert!(!app.core().session().unwrap().windows.is_empty());
        assert_eq!(app.info()["herdr_session"], Value::Null);
        app.shutdown();
    }

    #[test]
    fn ghostty_settings_come_from_the_chosen_file() {
        let app = App::start(in_memory()).unwrap();
        assert_eq!(
            app.ghostty().unfocused_split_opacity,
            None,
            "off: nothing set"
        );
        app.shutdown();

        let dir = std::env::temp_dir().join(format!("azj-daemon-gh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config");
        let app = App::start(Options {
            ghostty: GhosttyFiles::Only(file.clone()),
            ..in_memory()
        })
        .unwrap();
        assert_eq!(app.ghostty().unfocused_split_opacity, None, "missing file");
        // Read on each ask, so an edit shows on the next config reload.
        std::fs::write(&file, "unfocused-split-opacity = 0.6\n").unwrap();
        assert_eq!(app.ghostty().unfocused_split_opacity, Some(0.6));
        app.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_second_instance_can_not_take_the_socket() {
        let dir = std::env::temp_dir().join(format!("azj-daemon-{}", std::process::id()));
        let socket = dir.join("azulejo.sock");
        let first = App::start(Options {
            socket_path: Some(socket.clone()),
            ..in_memory()
        })
        .unwrap();
        assert_eq!(first.socket(), Some(socket.as_path()));
        let second = App::start(Options {
            socket_path: Some(socket.clone()),
            ..in_memory()
        });
        assert!(matches!(second, Err(BootError::Api { .. })));
        // Without `api_required` it starts anyway, API-less.
        let third = App::start(Options {
            socket_path: Some(socket.clone()),
            api_required: false,
            ..in_memory()
        })
        .unwrap();
        assert!(third.socket().is_none());
        drop(third);
        first.shutdown();
        assert!(!socket.exists(), "shutdown removes the socket");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn split_auto_chords(app: &App) -> Vec<String> {
        let reply = app
            .core()
            .execute(azulejo_protocol::Command::CommandsList)
            .unwrap()
            .reply;
        let azulejo_protocol::Reply::Commands { commands } = reply else {
            panic!("{reply:?}")
        };
        commands
            .into_iter()
            .find(|e| e.id == "split_auto")
            .unwrap()
            .chords
    }

    fn next_reload(
        events: &std::sync::mpsc::Receiver<azulejo_protocol::Event>,
    ) -> azulejo_protocol::ConfigReloaded {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            match events.recv_timeout(left) {
                Ok(azulejo_protocol::Event::ConfigReloaded(r)) => return r,
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        panic!("no config.reloaded within 10 s")
    }

    #[test]
    fn config_reloads_on_save_and_keeps_the_last_good_one() {
        let dir = std::env::temp_dir().join(format!(
            "azj-daemon-watch-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[keys]\nsplit_auto = \"ctrl+alt+n\"\n").unwrap();
        let app = App::start(Options {
            config_path: Some(path.clone()),
            platform: Platform::Macos,
            ..in_memory()
        })
        .unwrap();
        let events = app.core().subscribe();
        assert_eq!(split_auto_chords(&app), ["ctrl+alt+n"]);
        // FSEvents can replay the watch's own start; let it pass.
        std::thread::sleep(std::time::Duration::from_millis(300));

        // A good edit applies.
        std::fs::write(&path, "[keys]\nsplit_auto = \"ctrl+alt+m\"\n").unwrap();
        let r = next_reload(&events);
        assert!(r.applied, "{r:?}");
        assert!(r.diagnostics.is_empty(), "{r:?}");
        assert_eq!(split_auto_chords(&app), ["ctrl+alt+m"]);

        // An editor-style save (write elsewhere, rename over) with a broken
        // file keeps the previous config and says where the problem is.
        let tmp = dir.join(".config.toml.swp");
        std::fs::write(&tmp, "[keys\nsplit_auto = 1\n").unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        let r = next_reload(&events);
        assert!(!r.applied, "{r:?}");
        let d = &r.diagnostics[0];
        assert_eq!(d.severity, "error");
        assert_eq!(d.line, Some(1), "{d:?}");
        assert_eq!(
            split_auto_chords(&app),
            ["ctrl+alt+m"],
            "previous config stays"
        );

        // Fixing it applies again; the manual reload says the same.
        std::fs::write(&path, "[keys]\nsplit_auto = \"ctrl+alt+k\"\n").unwrap();
        assert!(next_reload(&events).applied);
        assert_eq!(split_auto_chords(&app), ["ctrl+alt+k"]);
        let manual = app.reload_config().unwrap();
        assert!(manual.applied);
        assert!(
            next_reload(&events).applied,
            "manual reloads are broadcast too"
        );

        // Changing the herdr session can't apply live; say so.
        std::fs::write(&path, "[herdr]\nsession = \"elsewhere\"\n").unwrap();
        let r = next_reload(&events);
        assert!(r.applied);
        assert!(
            r.diagnostics.iter().any(|d| d.message.contains("restart")),
            "{r:?}"
        );

        app.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn themes_reload_on_save_and_set_theme_edits_the_file() {
        let dir = std::env::temp_dir().join(format!(
            "azj-daemon-themes-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "# mine\n[layout]\ndefault = \"tile\"\n").unwrap();
        let app = App::start(Options {
            config_path: Some(path.clone()),
            platform: Platform::Macos,
            ..in_memory()
        })
        .unwrap();
        let events = app.core().subscribe();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(app.config().appearance.theme, "atom-one-dark");

        // A theme file appearing (the folder too) reloads.
        std::fs::create_dir_all(dir.join("themes")).unwrap();
        std::fs::write(
            dir.join("themes/dusk.toml"),
            "inherits = \"atom-one-dark\"\n[color]\nwindow = \"#101010\"\n",
        )
        .unwrap();
        let r = next_reload(&events);
        assert!(r.applied, "{r:?}");
        assert!(app.config().themes.get("dusk").is_some());

        // Editing it reloads, and its problems reach the banner.
        std::fs::write(
            dir.join("themes/dusk.toml"),
            "inherits = \"atom-one-dark\"\n[color]\nwindow = \"dark\"\n",
        )
        .unwrap();
        let r = next_reload(&events);
        assert!(r.applied, "theme warnings don't block a reload: {r:?}");
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.message.contains("color.window"))
            .expect("the bad colour is reported");
        assert!(
            d.file.as_deref().unwrap().ends_with("themes/dusk.toml"),
            "{d:?}"
        );
        assert_eq!(d.line, Some(3));

        // set_theme writes the one line and applies it now.
        let r = app.set_theme("dusk").unwrap();
        assert!(r.applied);
        assert_eq!(app.config().appearance.theme, "dusk");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "# mine\n[layout]\ndefault = \"tile\"\n\n[appearance]\ntheme = \"dusk\"\n"
        );
        app.set_theme("auto").unwrap();
        assert!(app.config().appearance.is_auto());
        assert!(
            app.set_theme("solarized")
                .unwrap_err()
                .contains("unknown theme")
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("theme = \"auto\"")
        );

        app.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn commands_list_follows_the_keymap() {
        let app = App::start(in_memory()).unwrap();
        let chords = split_auto_chords(&app);
        assert!(!chords.is_empty(), "the default keymap binds split_auto");
        app.shutdown();
    }

    #[test]
    fn config_picks_the_herdr_session() {
        assert_eq!(
            herdr_session(&HerdrConfig::default()),
            Some(HerdrSession::default_session()),
            "the user's own herdr session by default (ADR 0008)"
        );
        let named = HerdrConfig {
            enabled: true,
            session: Some("azulejo".into()),
            ..HerdrConfig::default()
        };
        assert_eq!(herdr_session(&named), Some(HerdrSession::named("azulejo")));
        let off = HerdrConfig {
            enabled: false,
            ..HerdrConfig::default()
        };
        assert_eq!(herdr_session(&off), None);
        let default = HerdrConfig {
            enabled: true,
            session: None,
            ..HerdrConfig::default()
        };
        assert_eq!(
            herdr_session(&default),
            Some(HerdrSession::default_session())
        );
    }

    #[test]
    fn attach_targets_the_session() {
        let info = HerdrInfo {
            herdr: Herdr::with_path("/bin/herdr"),
            session: HerdrSession::named("azulejo"),
            socket: "/tmp/h.sock".into(),
        };
        assert_eq!(
            info.attach("term_1", true).argv(),
            [
                "/bin/herdr",
                "--session",
                "azulejo",
                "terminal",
                "attach",
                "term_1",
                "--takeover"
            ]
        );
    }
}
