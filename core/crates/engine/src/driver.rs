//! The running core: one thread owns the [`Engine`]; everything else talks to
//! it through messages.
//!
//! - Commands from shells and the API arrive through [`Core::execute`].
//! - herdr work runs on a small tokio runtime ([`crate::link`]); results come
//!   back as internal commands, so state only ever changes on the engine's
//!   thread.
//! - Every change is saved, debounced ([`crate::persist`]), and broadcast to
//!   [`Core::subscribe`] listeners.

use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::thread;

use azulejo_herdr::StreamItem;
use azulejo_protocol::{Command, Error, ErrorCode, Event, Reply, Session};
use tokio::sync::mpsc as tmpsc;

use crate::link::{self, Effect, HerdrPort, Internal};
use crate::persist::{self, Loaded, Saver};
use crate::preview::{self, Previews};
use crate::{Engine, Outcome, terminal};

/// Called with a log line. Defaults to stderr.
pub type LogFn = Arc<dyn Fn(String) + Send + Sync>;

/// Builds the command palette's entries from the current config
/// (`commands.list`); see [`Core::set_commands`].
pub type CommandsFn = Arc<dyn Fn() -> Vec<azulejo_protocol::CommandEntry> + Send + Sync>;

pub struct CoreOptions {
    /// Where to load and save the session; `None` keeps it in memory only.
    pub state_path: Option<PathBuf>,
    /// The API socket, passed to terminals as `AZULEJO_SOCKET`.
    pub api_socket: String,
    pub log: LogFn,
}

impl Default for CoreOptions {
    fn default() -> Self {
        Self {
            state_path: Some(persist::default_state_path()),
            api_socket: String::new(),
            log: Arc::new(|line| eprintln!("azulejo: {line}")),
        }
    }
}

enum Msg {
    Command(Command, mpsc::Sender<Result<Outcome, Error>>),
    Internal(Internal),
    Herdr(StreamItem),
    /// A preview tile's file finished rendering.
    Preview(
        azulejo_protocol::TileId,
        PathBuf,
        Box<azulejo_preview::Rendered>,
    ),
    Subscribe(mpsc::Sender<Event>),
    /// An event from outside the engine (a config reload) for every listener.
    Broadcast(Event),
    SetCommands(CommandsFn),
    Shutdown(mpsc::Sender<()>),
}

/// Handle on the running core. Dropping it saves pending state and stops the
/// core; herdr panes keep running.
pub struct Core {
    tx: mpsc::Sender<Msg>,
    thread: Option<thread::JoinHandle<()>>,
    herdr_socket: Option<PathBuf>,
}

impl Core {
    /// A core without herdr: terminal tiles stay unbound.
    pub fn start(opts: CoreOptions) -> Core {
        Self::spawn::<azulejo_herdr::Client>(opts, None)
    }

    /// A core that keeps terminal tiles bound to herdr panes through `port`.
    pub fn start_with_herdr<P: HerdrPort>(opts: CoreOptions, port: P) -> Core {
        Self::spawn(opts, Some(port))
    }

    fn spawn<P: HerdrPort>(opts: CoreOptions, port: Option<P>) -> Core {
        let herdr_socket = port.as_ref().and_then(HerdrPort::socket);
        let (tx, rx) = mpsc::channel();
        let self_tx = tx.clone();
        let thread = thread::Builder::new()
            .name("azulejo-core".into())
            .spawn(move || run(opts, port, &rx, self_tx))
            .expect("spawn core thread");
        Core {
            tx,
            thread: Some(thread),
            herdr_socket,
        }
    }

    /// The herdr session socket terminal tiles live in, when connected to
    /// herdr. Lets the CLI tell `HERDR_PANE_ID`s from different sessions apart.
    pub fn herdr_socket(&self) -> Option<&std::path::Path> {
        self.herdr_socket.as_deref()
    }

    /// Runs a command on the core's thread and waits for the result.
    pub fn execute(&self, command: Command) -> Result<Outcome, Error> {
        let (reply, wait) = mpsc::channel();
        self.tx
            .send(Msg::Command(command, reply))
            .map_err(|_| stopped())?;
        wait.recv().map_err(|_| stopped())?
    }

    /// A snapshot of the whole session.
    pub fn session(&self) -> Result<Session, Error> {
        match self.execute(Command::SessionGet)?.reply {
            Reply::Session { session } => Ok(session),
            other => Err(Error::new(
                ErrorCode::Internal,
                format!("unexpected reply {other:?}"),
            )),
        }
    }

    /// Every event from now on, including those caused by herdr.
    pub fn subscribe(&self) -> mpsc::Receiver<Event> {
        let (tx, rx) = mpsc::channel();
        let _ = self.tx.send(Msg::Subscribe(tx));
        rx
    }

    /// Sends `event` to every [`Core::subscribe`] listener, like the core's
    /// own events. For things the engine doesn't own, such as config reloads.
    pub fn broadcast(&self, event: Event) {
        let _ = self.tx.send(Msg::Broadcast(event));
    }

    /// Where `commands.list` gets its entries. Until this is called it
    /// answers `unavailable`.
    pub fn set_commands(&self, commands: CommandsFn) {
        let _ = self.tx.send(Msg::SetCommands(commands));
    }

    /// Saves pending state and stops the core.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let (done, wait) = mpsc::channel();
            if self.tx.send(Msg::Shutdown(done)).is_ok() {
                let _ = wait.recv();
            }
            let _ = thread.join();
        }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        self.stop();
    }
}

fn stopped() -> Error {
    Error::new(ErrorCode::Internal, "the core has stopped")
}

fn load_engine(opts: &CoreOptions) -> Engine {
    let Some(path) = &opts.state_path else {
        return Engine::new();
    };
    match persist::load(path) {
        Ok(Loaded::Session(session)) if !session.windows.is_empty() => {
            Engine::with_session(session)
        }
        Ok(Loaded::Session(_) | Loaded::Missing) => Engine::new(),
        Ok(Loaded::Corrupt { error, backup }) => {
            (opts.log)(format!(
                "state file was unusable ({error}); kept it as {} and started fresh",
                backup.display()
            ));
            Engine::new()
        }
        Err(e) => {
            (opts.log)(format!(
                "couldn't read {} ({e}); starting fresh",
                path.display()
            ));
            Engine::new()
        }
    }
}

fn run<P: HerdrPort>(
    opts: CoreOptions,
    port: Option<P>,
    rx: &mpsc::Receiver<Msg>,
    self_tx: mpsc::Sender<Msg>,
) {
    let mut engine = load_engine(&opts);
    let saver = opts.state_path.clone().map(|path| {
        let log = opts.log.clone();
        Saver::new(path.clone(), move |e| {
            log(format!("saving {} failed: {e}", path.display()));
        })
    });
    if let Some(saver) = &saver {
        saver.schedule(engine.session().clone());
    }

    // Preview tiles: watch their files and render off this thread.
    let mut previews = {
        let out = self_tx.clone();
        Previews::new(
            Arc::new(move |tile, path, rendered| {
                let _ = out.send(Msg::Preview(tile, path, Box::new(rendered)));
            }),
            opts.log.clone(),
        )
    };
    previews.sync(engine.session());

    // herdr: a worker for effects and a forwarder for herdr's events.
    let mut herdr = port.map(|port| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("azulejo-herdr")
            .enable_all()
            .build()
            .expect("tokio runtime");
        let port = Arc::new(port);
        let (effects_tx, effects_rx) = tmpsc::unbounded_channel::<Effect>();
        {
            let out = self_tx.clone();
            let log = opts.log.clone();
            let port = port.clone();
            let socket = opts.api_socket.clone();
            runtime.spawn(link::run_worker(
                port,
                effects_rx,
                socket,
                move |internal| {
                    let _ = out.send(Msg::Internal(internal));
                },
                move |line| log(line),
            ));
        }
        {
            let out = self_tx.clone();
            let mut events = runtime.block_on(async { port.events() });
            runtime.spawn(async move {
                while let Some(item) = events.recv().await {
                    if out.send(Msg::Herdr(item)).is_err() {
                        break;
                    }
                }
            });
        }
        // Adopt what herdr already runs first (ADR 0008); restoring waits for
        // that, so a fresh session's placeholder terminal doesn't create a
        // herdr workspace the user's own ones make unnecessary.
        let _ = effects_tx.send(Effect::Survey);
        Herdr {
            runtime,
            effects_tx,
            port,
            restored: false,
            survey: SurveyState::InFlight,
        }
    });
    drop(self_tx);

    let mut listeners: Vec<mpsc::Sender<Event>> = Vec::new();
    let mut commands: Option<CommandsFn> = None;
    let publish = |engine: &Engine,
                   events: &[Event],
                   herdr: &Option<Herdr<P>>,
                   listeners: &mut Vec<mpsc::Sender<Event>>,
                   previews: &mut Previews| {
        if events.is_empty() {
            return;
        }
        previews.sync(engine.session());
        if let Some(saver) = &saver {
            saver.schedule(engine.session().clone());
        }
        listeners.retain(|l| events.iter().all(|e| l.send(e.clone()).is_ok()));
        if let Some(h) = herdr {
            for effect in link::effects(events, engine.session()) {
                let _ = h.effects_tx.send(effect);
            }
        }
    };

    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Command(command, reply) if terminal::is_io(&command) => {
                // Resolve the pane here; talk to herdr off this thread.
                let pane = engine.terminal_pane(terminal::target(&command));
                match (pane, &herdr) {
                    (Err(e), _) => {
                        let _ = reply.send(Err(e));
                    }
                    (Ok(_), None) => {
                        let _ = reply.send(Err(Error::new(
                            ErrorCode::Unavailable,
                            "this core isn't connected to herdr",
                        )));
                    }
                    (Ok(pane), Some(h)) => {
                        let port = h.port.clone();
                        h.runtime.spawn(async move {
                            let result = terminal::run(&*port, pane, command).await;
                            let _ = reply.send(result.map(|reply| Outcome {
                                reply,
                                events: Vec::new(),
                            }));
                        });
                    }
                }
            }
            Msg::Command(Command::PreviewGet(target), reply) => {
                let result = engine.preview_path(target.tile).and_then(|(id, path)| {
                    path.map(|p| (id, PathBuf::from(p))).ok_or_else(|| {
                        Error::new(
                            ErrorCode::InvalidState,
                            format!("{id} has no file yet; use preview.open"),
                        )
                    })
                });
                match result {
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                    Ok((id, path)) => match previews.latest(id, &path) {
                        Some(content) => {
                            let _ = reply.send(Ok(Outcome {
                                reply: Reply::Preview(content.clone()),
                                events: Vec::new(),
                            }));
                        }
                        // Not rendered yet: render now, off this thread.
                        None => {
                            let revision = previews.revision();
                            std::thread::spawn(move || {
                                let rendered = azulejo_preview::render(&path);
                                let content = preview::content(id, &path, rendered, revision);
                                let _ = reply.send(Ok(Outcome {
                                    reply: Reply::Preview(content),
                                    events: Vec::new(),
                                }));
                            });
                        }
                    },
                }
            }
            Msg::Command(Command::CommandsList, reply) => {
                let result = match &commands {
                    Some(list) => Ok(Outcome {
                        reply: Reply::Commands { commands: list() },
                        events: Vec::new(),
                    }),
                    None => Err(Error::new(
                        ErrorCode::Unavailable,
                        "this core has no config to list commands from",
                    )),
                };
                let _ = reply.send(result);
            }
            // Layout files: read or write on this thread (small files), then
            // let the engine do the rest.
            Msg::Command(Command::LayoutApply(p), reply) if p.path.is_some() => {
                let result = layout_file_io::read(p).and_then(|p| {
                    let outcome = engine.execute(Command::LayoutApply(p))?;
                    publish(
                        &engine,
                        &outcome.events,
                        &herdr,
                        &mut listeners,
                        &mut previews,
                    );
                    Ok(outcome)
                });
                let _ = reply.send(result);
            }
            Msg::Command(Command::LayoutExport(p), reply) if p.path.is_some() => {
                let result = engine
                    .execute(Command::LayoutExport(p))
                    .and_then(layout_file_io::write);
                let _ = reply.send(result);
            }
            Msg::Command(Command::TerminalRespawn(_), reply) if herdr.is_none() => {
                let _ = reply.send(Err(Error::new(
                    ErrorCode::Unavailable,
                    "this core isn't connected to herdr",
                )));
            }
            Msg::Command(command, reply) => {
                let respawn = match &command {
                    Command::TerminalRespawn(p) => engine.session().target(p.tile).ok(),
                    _ => None,
                };
                let result = engine.execute(command);
                if let (Ok(_), Some(tile), Some(h)) = (&result, respawn, &herdr)
                    && let Some(effect) = link::respawn_effect(engine.session(), tile)
                {
                    let _ = h.effects_tx.send(effect);
                }
                if let Ok(outcome) = &result {
                    publish(
                        &engine,
                        &outcome.events,
                        &herdr,
                        &mut listeners,
                        &mut previews,
                    );
                }
                let _ = reply.send(result);
            }
            Msg::Internal(internal) => {
                let surveyed = matches!(internal, Internal::Surveyed(_) | Internal::SurveyFailed);
                let events = engine.apply_internal(internal);
                publish(&engine, &events, &herdr, &mut listeners, &mut previews);
                if surveyed && let Some(h) = &mut herdr {
                    h.survey_done(&engine);
                }
            }
            Msg::Herdr(item) => {
                let status = match &item {
                    StreamItem::Connected | StreamItem::Resync => Some(Event::HerdrStatus {
                        connected: true,
                        message: None,
                    }),
                    StreamItem::Disconnected(why) => Some(Event::HerdrStatus {
                        connected: false,
                        message: Some(why.clone()),
                    }),
                    StreamItem::Event(_) => None,
                };
                if let Some(event) = status {
                    listeners.retain(|l| l.send(event.clone()).is_ok());
                }
                let reaction = link::reconcile(&item, engine.session());
                for internal in reaction.internals {
                    let events = engine.apply_internal(internal);
                    publish(&engine, &events, &herdr, &mut listeners, &mut previews);
                }
                if let Some(h) = &mut herdr {
                    for effect in reaction.effects {
                        let _ = h.effects_tx.send(effect);
                    }
                    if reaction.survey {
                        h.request_survey();
                    }
                }
            }
            Msg::Preview(tile, path, rendered) => {
                if let Some(event) = previews.accept(tile, &path, *rendered) {
                    listeners.retain(|l| l.send(event.clone()).is_ok());
                }
            }
            Msg::Subscribe(listener) => listeners.push(listener),
            Msg::Broadcast(event) => listeners.retain(|l| l.send(event.clone()).is_ok()),
            Msg::SetCommands(list) => commands = Some(list),
            Msg::Shutdown(done) => {
                if let Some(saver) = &saver {
                    saver.flush();
                }
                let _ = done.send(());
                break;
            }
        }
    }
    drop(previews);
    // Stop herdr work before the saver goes; panes keep running in herdr.
    if let Some(h) = herdr.take() {
        h.runtime.shutdown_background();
    }
}

/// The herdr side of a running core.
struct Herdr<P> {
    runtime: tokio::runtime::Runtime,
    effects_tx: tmpsc::UnboundedSender<Effect>,
    port: Arc<P>,
    /// Set once the first survey is in and restore work has been queued.
    restored: bool,
    survey: SurveyState,
}

/// Surveys are coalesced: at most one queued, plus one more if something
/// changed while it ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurveyState {
    Idle,
    InFlight,
    /// Asked for again while one was running.
    Again,
}

impl<P> Herdr<P> {
    fn request_survey(&mut self) {
        self.survey = match self.survey {
            SurveyState::Idle => {
                let _ = self.effects_tx.send(Effect::Survey);
                SurveyState::InFlight
            }
            SurveyState::InFlight | SurveyState::Again => SurveyState::Again,
        };
    }

    fn survey_done(&mut self, engine: &Engine) {
        if !self.restored {
            self.restored = true;
            for effect in link::restore_effects(engine.session()) {
                let _ = self.effects_tx.send(effect);
            }
        }
        let again = self.survey == SurveyState::Again;
        self.survey = SurveyState::Idle;
        if again {
            self.request_survey();
        }
    }
}

/// File I/O for `layout.apply` and `layout.export` with a `path`.
mod layout_file_io {
    use std::path::Path;

    use azulejo_protocol::{Error, ErrorCode, LayoutApply, Reply};

    use crate::Outcome;

    /// Turns `path` into `toml` + `base_dir` (the file's directory).
    pub fn read(mut p: LayoutApply) -> Result<LayoutApply, Error> {
        let path = p.path.take().unwrap_or_default();
        let path = Path::new(&path);
        if path.is_relative() {
            return Err(Error::new(
                ErrorCode::InvalidParams,
                format!("`path` must be absolute, not {}", path.display()),
            ));
        }
        if p.toml.is_some() {
            return Err(Error::new(
                ErrorCode::InvalidParams,
                "give exactly one of `path` and `toml`",
            ));
        }
        // A directory means its `.azulejo.toml`.
        let file = if path.is_dir() {
            path.join(".azulejo.toml")
        } else {
            path.to_path_buf()
        };
        let text = std::fs::read_to_string(&file).map_err(|e| {
            Error::new(
                ErrorCode::InvalidParams,
                format!("can't read {}: {e}", file.display()),
            )
        })?;
        p.toml = Some(text);
        if p.base_dir.is_none() {
            p.base_dir = file.parent().map(|d| d.display().to_string());
        }
        Ok(p)
    }

    /// Writes an exported layout where it was asked to go.
    pub fn write(outcome: Outcome) -> Result<Outcome, Error> {
        if let Reply::LayoutFile {
            toml,
            path: Some(path),
        } = &outcome.reply
        {
            let path = Path::new(path);
            if path.is_relative() {
                return Err(Error::new(
                    ErrorCode::InvalidParams,
                    format!("`path` must be absolute, not {}", path.display()),
                ));
            }
            std::fs::write(path, toml).map_err(|e| {
                Error::new(
                    ErrorCode::InvalidParams,
                    format!("can't write {}: {e}", path.display()),
                )
            })?;
        }
        Ok(outcome)
    }
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
