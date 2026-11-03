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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use azulejo_api::{CoreDispatcher, Server, ServerHandle, WithInfo};
use azulejo_config::{Config, HerdrConfig, Platform};
use azulejo_engine::{Core, CoreOptions, LogFn, persist};
use azulejo_herdr::{AttachCommand, ClientOptions, Herdr, Session as HerdrSession, TerminalId};
use serde_json::{Map, Value};

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

pub struct Options {
    /// The user's config file; `None` uses the bundled defaults only.
    pub config_path: Option<PathBuf>,
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

/// A running core. Dropping it stops the API server, saves pending state and
/// stops the engine; herdr panes keep running.
pub struct App {
    core: Option<Arc<Core>>,
    server: Option<ServerHandle>,
    config: Config,
    herdr: Option<HerdrInfo>,
    socket: Option<PathBuf>,
    state_path: Option<PathBuf>,
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
            Some((client, info)) => (Core::start_with_herdr(core_opts, client), Some(info)),
            None => (Core::start(core_opts), None),
        };
        let core = Arc::new(core);
        {
            let config = config.clone();
            core.set_commands(Arc::new(move || azulejo_config::catalogue(&config)));
        }

        let mut app = App {
            core: Some(core.clone()),
            server: None,
            config,
            herdr: herdr_info,
            socket,
            state_path: opts.state_path,
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

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn herdr(&self) -> Option<&HerdrInfo> {
        self.herdr.as_ref()
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
        info.insert("config_path".into(), path(self.config.path.as_deref()));
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

fn connect_herdr(
    session: &HerdrSession,
) -> Result<(azulejo_herdr::Client, HerdrInfo), azulejo_herdr::Error> {
    let herdr = Herdr::locate()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| azulejo_herdr::Error::Binary(format!("tokio runtime: {e}")))?;
    let (client, _) = runtime.block_on(herdr.ensure_server(session, ClientOptions::default()))?;
    let info = HerdrInfo {
        socket: client.socket_path().to_owned(),
        herdr,
        session: session.clone(),
    };
    Ok((client, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory() -> Options {
        Options {
            config_path: None,
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

    #[test]
    fn commands_list_follows_the_keymap() {
        let app = App::start(Options {
            config_path: None,
            ..in_memory()
        })
        .unwrap();
        let reply = app
            .core()
            .execute(azulejo_protocol::Command::CommandsList)
            .unwrap()
            .reply;
        let azulejo_protocol::Reply::Commands { commands } = reply else {
            panic!("{reply:?}")
        };
        let split = commands.iter().find(|e| e.id == "split_auto").unwrap();
        assert!(
            !split.chords.is_empty(),
            "the default keymap binds split_auto"
        );
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
