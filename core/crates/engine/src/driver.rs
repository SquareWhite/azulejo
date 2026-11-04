//! The running core: one thread owns the [`Engine`]; everything else talks to
//! it through messages.
//!
//! - Commands from shells and the API arrive through [`Core::execute`].
//! - herdr work runs on a small tokio runtime ([`crate::link`]); results come
//!   back as internal commands, so state only ever changes on the engine's
//!   thread. Each herdr machine (Local, and herdr's saved machines added with
//!   [`Core::add_machine`], ADR 0011) has its own link: worker, event stream
//!   and agent subscription, so one machine dropping never holds up another.
//! - Every change is saved, debounced ([`crate::persist`]), and broadcast to
//!   [`Core::subscribe`] listeners.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::thread;

use azulejo_herdr::StreamItem;
use azulejo_protocol::{
    Command, Error, ErrorCode, Event, LOCAL_MACHINE, MachineInfo, MachineState, Reply, Session,
};
use tokio::sync::mpsc as tmpsc;

use crate::files::{self as file_tiles, Files, Handled};
use crate::link::{self, Effect, HerdrPort, Internal, Machine};
use crate::persist::{self, Loaded, Saver};
use crate::preview::{self, Previews};
use crate::repos::{self, Repos};
use crate::{Engine, Outcome, meta, terminal};

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
    Herdr(Machine, StreamItem),
    /// A machine's agent-status subscription went live: read every pane's
    /// agent status for what changed before herdr accepted it.
    AgentsLive(Machine),
    /// Connect a herdr machine (ADR 0011). The port is the core's `P`.
    AddMachine(MachineInfo, Box<dyn std::any::Any + Send>),
    /// Show a machine without connecting it (`off`, or an error before
    /// there's anything to connect).
    ShowMachine(MachineInfo),
    /// A machine left herdr's list (or is hidden now).
    RemoveMachine(String),
    /// A preview tile's file finished rendering.
    Preview(
        azulejo_protocol::TileId,
        PathBuf,
        Box<azulejo_preview::Rendered>,
    ),
    /// File tile work finished off this thread.
    Files(file_tiles::Delivery),
    /// Workspace repository work finished off this thread (F26).
    Repos(repos::Delivery),
    Subscribe(mpsc::Sender<Event>),
    /// An event from outside the engine (a config reload) for every listener.
    Broadcast(Event),
    SetCommands(CommandsFn),
    SetOnExit(azulejo_model::OnExit),
    SetLayoutPrefs(azulejo_model::LayoutPrefs),
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
        Self::spawn::<azulejo_herdr::Client>(opts, None, MachineState::Connecting)
    }

    /// A core that keeps terminal tiles bound to herdr panes through `port`.
    /// Local reads `connecting` until the core has checked the panes its
    /// restored tiles were bound to: shells hold those tiles' attaches until
    /// then, since a herdr that (re)started gave every pane a new terminal.
    pub fn start_with_herdr<P: HerdrPort>(opts: CoreOptions, port: P) -> Core {
        Self::spawn(opts, Some(port), MachineState::Connecting)
    }

    /// Like [`Core::start_with_herdr`], for a herdr server that was already
    /// running (the caller just reached it). Its panes are most likely the
    /// ones the saved session names, so Local reads `connected` at once and
    /// restored tiles attach without waiting for the check; a pane that did
    /// change is rebound when the check lands.
    pub fn start_with_running_herdr<P: HerdrPort>(opts: CoreOptions, port: P) -> Core {
        Self::spawn(opts, Some(port), MachineState::Connected)
    }

    fn spawn<P: HerdrPort>(opts: CoreOptions, port: Option<P>, local: MachineState) -> Core {
        let herdr_socket = port.as_ref().and_then(HerdrPort::socket);
        let (tx, rx) = mpsc::channel();
        let self_tx = tx.clone();
        let thread = thread::Builder::new()
            .name("azulejo-core".into())
            .spawn(move || run(opts, port, local, &rx, self_tx))
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

    /// Connects a herdr saved machine (ADR 0011): its workspaces are adopted
    /// like Local's, and workspaces created on it put their terminals there.
    /// `port` must be the same type the core was started with. Adding a
    /// machine that's already there replaces its link.
    pub fn add_machine<P: HerdrPort>(&self, id: String, label: String, port: P) {
        let info = MachineInfo {
            id,
            label,
            state: MachineState::Connecting,
            message: None,
        };
        let _ = self.tx.send(Msg::AddMachine(info, Box::new(port)));
    }

    /// Lists a machine without connecting to it, e.g. `off` because it's
    /// disabled in herdr. Replaces a running link.
    pub fn show_machine(&self, info: MachineInfo) {
        let _ = self.tx.send(Msg::ShowMachine(info));
    }

    /// Disconnects a machine and drops it from `machines.list`. Its
    /// workspaces stay, unconnected, until it comes back.
    pub fn remove_machine(&self, id: String) {
        let _ = self.tx.send(Msg::RemoveMachine(id));
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

    /// What terminal tiles do when their process exits, unless a tile says
    /// otherwise (`[terminal] on_exit`). Close until this is called.
    pub fn set_on_exit(&self, on_exit: azulejo_model::OnExit) {
        let _ = self.tx.send(Msg::SetOnExit(on_exit));
    }

    /// The `[layout]` preferences: what new tabs start as, where new tiles go
    /// in dynamic tabs, the layout cycle (ADR 0010). Manual tabs and the
    /// model's defaults until this is called.
    pub fn set_layout_prefs(&self, prefs: azulejo_model::LayoutPrefs) {
        let _ = self.tx.send(Msg::SetLayoutPrefs(prefs));
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
    local: MachineState,
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

    // Files tiles: list folders off this thread, watch the open ones.
    let mut files = {
        let out = self_tx.clone();
        Files::new(
            Arc::new(move |delivery| {
                let _ = out.send(Msg::Files(delivery));
            }),
            opts.log.clone(),
        )
    };
    files.sync(engine.session());

    // Workspaces' git branches (F26): find, watch and read off this thread.
    let mut repos = {
        let out = self_tx.clone();
        Repos::new(
            Arc::new(move |delivery| {
                let _ = out.send(Msg::Repos(delivery));
            }),
            opts.log.clone(),
        )
    };
    repos.sync(engine.session());

    // herdr: one link per machine, all on one small runtime.
    let mut herdr = Links::<P>::new(self_tx.clone(), opts.log.clone(), opts.api_socket.clone());
    if let Some(port) = port {
        herdr.add(
            None,
            MachineInfo {
                id: LOCAL_MACHINE.into(),
                label: "Local".into(),
                state: local,
                message: None,
            },
            port,
            &engine,
        );
    }
    drop(self_tx);

    let mut listeners: Vec<mpsc::Sender<Event>> = Vec::new();
    let mut commands: Option<CommandsFn> = None;
    let mut on_exit = azulejo_model::OnExit::default();
    let publish = |engine: &Engine,
                   events: &[Event],
                   herdr: &Links<P>,
                   listeners: &mut Vec<mpsc::Sender<Event>>,
                   previews: &mut Previews,
                   files: &mut Files,
                   repos: &mut Repos| {
        if events.is_empty() {
            return;
        }
        previews.sync(engine.session());
        files.sync(engine.session());
        repos.sync(engine.session());
        if let Some(saver) = &saver {
            saver.schedule(engine.session().clone());
        }
        listeners.retain(|l| events.iter().all(|e| l.send(e.clone()).is_ok()));
        for (machine, effect) in link::effects(events, engine.session()) {
            herdr.send(&machine, effect);
        }
        herdr.follow_agents(engine.session());
    };

    while let Ok(msg) = rx.recv() {
        // A files panel isn't in the session: its paths open beside the
        // focused tile.
        let msg = match msg {
            Msg::Command(Command::FilesOpen(mut p), reply) if files.is_panel(p.tile) => {
                p.tile = None;
                Msg::Command(Command::FilesOpen(p), reply)
            }
            other => other,
        };
        match msg {
            Msg::Command(command, reply) if terminal::is_io(&command) => {
                // Resolve the pane here; talk to herdr off this thread.
                let pane = engine.terminal_pane(terminal::target(&command));
                let port = pane
                    .as_ref()
                    .ok()
                    .and_then(|p| herdr.port(&p.machine))
                    .zip(herdr.runtime.as_ref());
                match (pane, port) {
                    (Err(e), _) => {
                        let _ = reply.send(Err(e));
                    }
                    (Ok(pane), None) => {
                        let _ = reply.send(Err(not_connected(&herdr, &pane.machine)));
                    }
                    (Ok(pane), Some((port, runtime))) => {
                        runtime.spawn(async move {
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
                            let mode = engine
                                .session()
                                .tile(id)
                                .map(|t| t.preview_mode)
                                .unwrap_or_default();
                            std::thread::spawn(move || {
                                let rendered = preview::render(&path, mode);
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
            Msg::Command(command, reply) if file_tiles::is_runtime(&command) => {
                match files.handle(&engine, command) {
                    Handled::Done(result) => {
                        let result = *result;
                        if let Ok(outcome) = &result {
                            for event in &outcome.events {
                                listeners.retain(|l| l.send(event.clone()).is_ok());
                            }
                        }
                        let _ = reply.send(result);
                    }
                    Handled::Later(job) => {
                        let spawned = std::thread::Builder::new()
                            .name("azulejo-files-list".into())
                            .spawn({
                                let reply = reply.clone();
                                move || {
                                    let _ = reply.send(job());
                                }
                            });
                        if spawned.is_err() {
                            let _ = reply.send(Err(Error::new(
                                ErrorCode::Internal,
                                "couldn't start a thread to list the folder",
                            )));
                        }
                    }
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
                        &mut files,
                        &mut repos,
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
            Msg::Command(Command::TerminalRespawn(_), reply) if herdr.links.is_empty() => {
                let _ = reply.send(Err(Error::new(
                    ErrorCode::Unavailable,
                    "this core isn't connected to herdr",
                )));
            }
            Msg::Command(Command::MachinesList, reply) => {
                let _ = reply.send(Ok(Outcome {
                    reply: Reply::Machines {
                        machines: herdr.list(),
                    },
                    events: Vec::new(),
                }));
            }
            Msg::Command(Command::WorkspaceCreate(mut p), reply) if p.machine.is_some() => {
                let result = herdr.resolve(p.machine.as_deref()).and_then(|machine| {
                    p.machine = machine;
                    let outcome = engine.execute(Command::WorkspaceCreate(p))?;
                    publish(
                        &engine,
                        &outcome.events,
                        &herdr,
                        &mut listeners,
                        &mut previews,
                        &mut files,
                        &mut repos,
                    );
                    Ok(outcome)
                });
                let _ = reply.send(result);
            }
            Msg::Command(command, reply) => {
                let respawn = match &command {
                    Command::TerminalRespawn(p) => engine.session().target(p.tile).ok(),
                    _ => None,
                };
                let result = engine.execute(command);
                if let (Ok(_), Some(tile)) = (&result, respawn)
                    && let Some((machine, effect)) = link::respawn_effect(engine.session(), tile)
                {
                    herdr.send(&machine, effect);
                }
                if let Ok(outcome) = &result {
                    publish(
                        &engine,
                        &outcome.events,
                        &herdr,
                        &mut listeners,
                        &mut previews,
                        &mut files,
                        &mut repos,
                    );
                }
                let _ = reply.send(result);
            }
            Msg::Internal(Internal::Announce(machine, generation)) => {
                if !herdr.is_current(&machine, generation) {
                    continue;
                }
                // Only news when the machine wasn't connected (a resync after
                // lost events changes nothing, nor does a herdr that was
                // already running at start).
                if let Some(event) = herdr.track(&machine, &StreamItem::Connected) {
                    if machine.is_none() {
                        let status = Event::HerdrStatus {
                            connected: true,
                            message: None,
                        };
                        listeners.retain(|l| l.send(status.clone()).is_ok());
                    }
                    listeners.retain(|l| l.send(event.clone()).is_ok());
                }
            }
            Msg::Internal(internal) => {
                let surveyed = match &internal {
                    Internal::Surveyed(m, _, _) | Internal::SurveyFailed(m) => Some(m.clone()),
                    _ => None,
                };
                if let Internal::BindTerminal { tile, .. } | Internal::MarkEnded { tile } =
                    &internal
                {
                    herdr.created(*tile);
                }
                let events = engine.apply_internal(internal);
                publish(
                    &engine,
                    &events,
                    &herdr,
                    &mut listeners,
                    &mut previews,
                    &mut files,
                    &mut repos,
                );
                if let Some(machine) = surveyed {
                    herdr.survey_done(&machine, &engine);
                }
            }
            Msg::AgentsLive(machine) => {
                if let Some(link) = herdr.links.get(&machine) {
                    let _ = link.effects_tx.send(Effect::AgentStatuses);
                }
            }
            Msg::Herdr(machine, item) => {
                if !herdr.links.contains_key(&machine) {
                    // A removed machine's stream, still draining.
                    continue;
                }
                let connected = matches!(item, StreamItem::Connected | StreamItem::Resync);
                if !matches!(item, StreamItem::Event(_)) {
                    herdr.next_generation(&machine);
                }
                // Going down is news at once, but only once: the stream says
                // so after every failed retry. Coming back is announced after
                // the bound tiles are checked (`Internal::Announce`).
                // `herdr.status` stays Local's; every machine gets
                // `machine.status` (ADR 0011).
                if let StreamItem::Disconnected(why) = &item
                    && let Some(event) = herdr.track(&machine, &item)
                {
                    if machine.is_none() {
                        let status = Event::HerdrStatus {
                            connected: false,
                            message: Some(why.clone()),
                        };
                        listeners.retain(|l| l.send(status.clone()).is_ok());
                    }
                    listeners.retain(|l| l.send(event.clone()).is_ok());
                }
                if let StreamItem::Event(azulejo_herdr::HerdrEvent::PaneUpdated(_)) = &item {
                    herdr.poke_processes(&machine);
                }
                let reaction = link::reconcile(&machine, &item, engine.session(), on_exit);
                for internal in reaction.internals {
                    let events = engine.apply_internal(internal);
                    publish(
                        &engine,
                        &events,
                        &herdr,
                        &mut listeners,
                        &mut previews,
                        &mut files,
                        &mut repos,
                    );
                }
                if connected {
                    let (checks, rest): (Vec<Effect>, Vec<Effect>) = reaction
                        .effects
                        .into_iter()
                        .partition(|e| matches!(e, Effect::Reresolve { .. }));
                    for effect in rest {
                        herdr.send(&machine, effect);
                    }
                    herdr.check_bound(&machine, checks, link::Check::Connected);
                } else {
                    for effect in reaction.effects {
                        herdr.send(&machine, effect);
                    }
                }
                if reaction.survey {
                    herdr.request_survey(&machine);
                }
            }
            Msg::AddMachine(info, port) => match port.downcast::<P>() {
                Ok(port) => {
                    let id = Some(info.id.clone());
                    let event = herdr.add(id, info, *port, &engine);
                    listeners.retain(|l| l.send(event.clone()).is_ok());
                }
                Err(_) => (opts.log)(format!(
                    "machine {}: its port isn't the core's herdr port type",
                    info.label
                )),
            },
            Msg::ShowMachine(info) => {
                let event = herdr.show(info);
                listeners.retain(|l| l.send(event.clone()).is_ok());
            }
            Msg::RemoveMachine(id) => {
                if let Some(event) = herdr.remove(&id) {
                    listeners.retain(|l| l.send(event.clone()).is_ok());
                }
            }
            Msg::Preview(tile, path, rendered) => {
                if let Some(event) = previews.accept(tile, &path, *rendered) {
                    listeners.retain(|l| l.send(event.clone()).is_ok());
                }
            }
            Msg::Files(delivery) => {
                for event in files.accept(delivery) {
                    listeners.retain(|l| l.send(event.clone()).is_ok());
                }
            }
            Msg::Repos(delivery) => {
                if let Some(internal) = repos.accept(delivery) {
                    let events = engine.apply_internal(internal);
                    publish(
                        &engine,
                        &events,
                        &herdr,
                        &mut listeners,
                        &mut previews,
                        &mut files,
                        &mut repos,
                    );
                }
            }
            Msg::Subscribe(listener) => listeners.push(listener),
            Msg::Broadcast(event) => listeners.retain(|l| l.send(event.clone()).is_ok()),
            Msg::SetCommands(list) => commands = Some(list),
            Msg::SetOnExit(policy) => on_exit = policy,
            Msg::SetLayoutPrefs(prefs) => {
                let events = engine.set_layout_prefs(prefs);
                if !events.is_empty() {
                    if let Some(saver) = &saver {
                        saver.schedule(engine.session().clone());
                    }
                    for event in events {
                        listeners.retain(|l| l.send(event.clone()).is_ok());
                    }
                }
            }
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
    drop(files);
    drop(repos);
    // Stop herdr work before the saver goes; panes keep running in herdr.
    herdr.shutdown();
}

fn not_connected<P: HerdrPort>(herdr: &Links<P>, machine: &Machine) -> Error {
    match machine {
        None if herdr.links.contains_key(&None) => {
            Error::new(ErrorCode::Unavailable, "Local herdr isn't connected")
        }
        None => Error::new(ErrorCode::Unavailable, "this core isn't connected to herdr"),
        Some(id) => Error::new(
            ErrorCode::Unavailable,
            format!(
                "machine {} isn't connected",
                herdr.label(id).unwrap_or(id.as_str())
            ),
        ),
    }
}

/// The herdr side of a running core: one link per machine (ADR 0011).
struct Links<P> {
    /// Created with the first link.
    runtime: Option<tokio::runtime::Runtime>,
    links: HashMap<Machine, Link<P>>,
    /// Every machine listed, connected or not.
    info: HashMap<Machine, MachineInfo>,
    out: mpsc::Sender<Msg>,
    log: LogFn,
    api_socket: String,
}

/// One machine's worker, event stream and agent subscription.
struct Link<P> {
    effects_tx: tmpsc::UnboundedSender<Effect>,
    port: Arc<P>,
    /// Tiles with a `CreateTerminal` on its way: a split right after launch
    /// would otherwise get a second pane from the restore pass.
    creating: std::cell::RefCell<std::collections::HashSet<azulejo_protocol::TileId>>,
    /// The panes whose agent status to follow; see [`follow_agents`].
    agents: tokio::sync::watch::Sender<Vec<String>>,
    /// Asks the process poller to look now (see [`meta::follow_processes`]).
    processes: Arc<tokio::sync::Notify>,
    /// Set once the first survey is in and restore work has been queued.
    restored: bool,
    /// Counts the event stream's connects and drops; an
    /// [`Internal::Announce`] from an older one is stale.
    generation: u64,
    /// The post-connect check of bound tiles ([`link::check_bound`]).
    check: Option<tokio::task::JoinHandle<()>>,
    survey: SurveyState,
    tasks: Vec<tokio::task::JoinHandle<()>>,
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

/// A machine ID as the session stores it: `None` for Local.
fn machine_key(id: &str) -> Machine {
    (id != LOCAL_MACHINE).then(|| id.to_owned())
}

impl<P: HerdrPort> Links<P> {
    fn new(out: mpsc::Sender<Msg>, log: LogFn, api_socket: String) -> Self {
        Self {
            runtime: None,
            links: HashMap::new(),
            info: HashMap::new(),
            out,
            log,
            api_socket,
        }
    }

    fn runtime(&mut self) -> &tokio::runtime::Runtime {
        self.runtime.get_or_insert_with(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("azulejo-herdr")
                .enable_all()
                .build()
                .expect("tokio runtime")
        })
    }

    /// Connects `machine` through `port`: a worker for effects, a forwarder
    /// for herdr's events, agent status for its bound panes, and a first
    /// survey (adoption, then restore; ADR 0008).
    fn add(&mut self, machine: Machine, info: MachineInfo, port: P, engine: &Engine) -> Event {
        self.drop_link(&machine);
        let out = self.out.clone();
        let processes_out = self.out.clone();
        let log = self.log.clone();
        let api_socket = self.api_socket.clone();
        let label = info.label.clone();
        let runtime = self.runtime();
        let port = Arc::new(port);
        let (effects_tx, effects_rx) = tmpsc::unbounded_channel::<Effect>();
        let mut tasks = Vec::new();
        {
            let out = out.clone();
            let prefix = if machine.is_some() {
                format!("{label}: ")
            } else {
                String::new()
            };
            tasks.push(runtime.spawn(link::run_worker(
                machine.clone(),
                port.clone(),
                effects_rx,
                api_socket,
                move |internal| {
                    let _ = out.send(Msg::Internal(internal));
                },
                move |line| log(format!("{prefix}{line}")),
            )));
        }
        {
            let out = out.clone();
            let machine = machine.clone();
            let mut events = runtime.block_on(async { port.events() });
            tasks.push(runtime.spawn(async move {
                while let Some(item) = events.recv().await {
                    if out.send(Msg::Herdr(machine.clone(), item)).is_err() {
                        break;
                    }
                }
            }));
        }
        // Agent status needs a subscription per pane; follow the bound ones.
        let (agents, want) =
            tokio::sync::watch::channel(link::agent_panes(engine.session(), &machine));
        {
            let machine = machine.clone();
            let live_out = out.clone();
            let live_machine = machine.clone();
            tasks.push(runtime.spawn(follow_agents(
                port.clone(),
                want,
                move |item| {
                    let _ = out.send(Msg::Herdr(machine.clone(), item));
                },
                move || {
                    let _ = live_out.send(Msg::AgentsLive(live_machine.clone()));
                },
            )));
        }
        // What runs in each bound pane, for tile headers (F28): polled, and
        // at once when herdr reports a pane changed.
        let processes = Arc::new(tokio::sync::Notify::new());
        {
            let out = processes_out;
            tasks.push(runtime.spawn(meta::follow_processes(
                machine.clone(),
                port.clone(),
                agents.subscribe(),
                processes.clone(),
                move |internal| {
                    let _ = out.send(Msg::Internal(internal));
                },
            )));
        }
        let _ = effects_tx.send(Effect::Survey);
        let checks: Vec<Effect> = link::restore_effects(engine.session(), &machine)
            .into_iter()
            .filter(|e| matches!(e, Effect::Reresolve { .. }))
            .collect();
        self.links.insert(
            machine.clone(),
            Link {
                effects_tx,
                port,
                agents,
                processes,
                creating: Default::default(),
                restored: false,
                generation: 0,
                check: None,
                survey: SurveyState::InFlight,
                tasks,
            },
        );
        self.info.insert(machine.clone(), info.clone());
        // Restored tiles wait for this before they attach: check them now,
        // without waiting for the event stream.
        self.check_bound(&machine, checks, link::Check::Added);
        Event::MachineStatus { machine: info }
    }

    /// Lists a machine without a link.
    fn show(&mut self, info: MachineInfo) -> Event {
        let machine = machine_key(&info.id);
        self.drop_link(&machine);
        self.info.insert(machine, info.clone());
        Event::MachineStatus { machine: info }
    }

    fn remove(&mut self, id: &str) -> Option<Event> {
        let machine = machine_key(id);
        self.drop_link(&machine);
        self.info.remove(&machine).map(|_| Event::MachineRemoved {
            machine: id.to_owned(),
        })
    }

    fn drop_link(&mut self, machine: &Machine) {
        if let Some(link) = self.links.remove(machine) {
            for task in link.tasks.into_iter().chain(link.check) {
                task.abort();
            }
        }
    }

    /// The event stream connected or dropped: whatever was being announced
    /// for the last connection is stale now.
    fn next_generation(&mut self, machine: &Machine) {
        if let Some(link) = self.links.get_mut(machine) {
            link.generation += 1;
        }
    }

    fn is_current(&self, machine: &Machine, generation: u64) -> bool {
        self.links
            .get(machine)
            .is_some_and(|l| l.generation == generation)
    }

    /// Checks `machine`'s bound tiles beside its worker, then announces the
    /// machine connected ([`link::check_bound`]). Replaces a check still
    /// running for an earlier connection.
    fn check_bound(&mut self, machine: &Machine, checks: Vec<Effect>, when: link::Check) {
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        let Some(link) = self.links.get_mut(machine) else {
            return;
        };
        if let Some(old) = link.check.take() {
            old.abort();
        }
        let out = self.out.clone();
        let log = self.log.clone();
        let prefix = self
            .info
            .get(machine)
            .filter(|_| machine.is_some())
            .map(|m| format!("{}: ", m.label))
            .unwrap_or_default();
        link.check = Some(runtime.spawn(link::check_bound(
            machine.clone(),
            link.port.clone(),
            checks,
            (when, link.generation),
            move |internal| {
                let _ = out.send(Msg::Internal(internal));
            },
            move |line| log(format!("{prefix}{line}")),
        )));
    }

    /// Asks `machine`'s process poller to look now (a pane changed).
    fn poke_processes(&self, machine: &Machine) {
        if let Some(link) = self.links.get(machine) {
            link.processes.notify_one();
        }
    }

    fn port(&self, machine: &Machine) -> Option<Arc<P>> {
        self.links.get(machine).map(|l| l.port.clone())
    }

    /// Queues herdr work on `machine`, once per tile for pane creation. A
    /// machine that isn't connected gets it when it connects (restore).
    fn send(&self, machine: &Machine, effect: Effect) {
        let Some(link) = self.links.get(machine) else {
            return;
        };
        if let Effect::CreateTerminal { tile, .. } = &effect
            && !link.creating.borrow_mut().insert(*tile)
        {
            return;
        }
        let _ = link.effects_tx.send(effect);
    }

    /// A tile's pane creation finished, one way or the other.
    fn created(&self, tile: azulejo_protocol::TileId) {
        for link in self.links.values() {
            link.creating.borrow_mut().remove(&tile);
        }
    }

    fn request_survey(&mut self, machine: &Machine) {
        let Some(link) = self.links.get_mut(machine) else {
            return;
        };
        link.survey = match link.survey {
            SurveyState::Idle => {
                let _ = link.effects_tx.send(Effect::Survey);
                SurveyState::InFlight
            }
            SurveyState::InFlight | SurveyState::Again => SurveyState::Again,
        };
    }

    fn survey_done(&mut self, machine: &Machine, engine: &Engine) {
        let Some(link) = self.links.get_mut(machine) else {
            return;
        };
        let restore = !link.restored;
        link.restored = true;
        let again = link.survey == SurveyState::Again;
        link.survey = SurveyState::Idle;
        if restore {
            for effect in link::restore_effects(engine.session(), machine) {
                self.send(machine, effect);
            }
        }
        if again {
            self.request_survey(machine);
        }
    }

    /// Points every machine's agent subscription at its bound panes.
    fn follow_agents(&self, session: &azulejo_model::Session) {
        for (machine, link) in &self.links {
            let want = link::agent_panes(session, machine);
            link.agents.send_if_modified(|panes| {
                let changed = *panes != want;
                if changed {
                    *panes = want;
                }
                changed
            });
        }
    }

    /// Follows a machine's connection state from its event stream; returns
    /// `machine.status` when it changed.
    fn track(&mut self, machine: &Machine, item: &StreamItem) -> Option<Event> {
        let (state, message) = match item {
            StreamItem::Connected | StreamItem::Resync => (MachineState::Connected, None),
            StreamItem::Disconnected(why) if azulejo_herdr::needs_attention(why) => {
                (MachineState::Error, Some(why.clone()))
            }
            StreamItem::Disconnected(why) => (MachineState::Reconnecting, Some(why.clone())),
            StreamItem::Event(_) => return None,
        };
        let info = self.info.get_mut(machine)?;
        if info.state == state && info.message == message {
            return None;
        }
        info.state = state;
        info.message = message;
        Some(Event::MachineStatus {
            machine: info.clone(),
        })
    }

    /// Local first, then by label.
    fn list(&self) -> Vec<MachineInfo> {
        let mut all: Vec<MachineInfo> = self.info.values().cloned().collect();
        all.sort_by(|a, b| {
            (!a.is_local(), a.label.to_lowercase(), &a.id).cmp(&(
                !b.is_local(),
                b.label.to_lowercase(),
                &b.id,
            ))
        });
        all
    }

    /// A machine named in `workspace.create`: `local`, an ID or a label.
    fn resolve(&self, name: Option<&str>) -> Result<Machine, Error> {
        let Some(name) = name.filter(|n| *n != LOCAL_MACHINE) else {
            return Ok(None);
        };
        self.info
            .values()
            .filter(|m| !m.is_local())
            .find(|m| m.id == name)
            .or_else(|| {
                self.info
                    .values()
                    .find(|m| !m.is_local() && m.label == name)
            })
            .map(|m| Some(m.id.clone()))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidParams,
                    format!("no herdr machine `{name}`; see machines.list"),
                )
            })
    }

    fn label(&self, id: &str) -> Option<&str> {
        self.info.get(&machine_key(id)).map(|m| m.label.as_str())
    }

    fn shutdown(mut self) {
        let machines: Vec<Machine> = self.links.keys().cloned().collect();
        for machine in machines {
            self.drop_link(&machine);
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// Keeps one agent-status subscription open for the panes in `want`. When the
/// set changes, the new subscription runs alongside the old one until herdr
/// confirms it. Only agent status events go to `out`; lifecycle and resync
/// come from the main stream.
///
/// herdr streams only what happens after it accepts a subscription, and an
/// event still in flight on the old subscription is lost when it's dropped.
/// So `live` runs whenever a subscription goes live (first open, reconnect,
/// resync, or taking over from the old one): the caller then reads every
/// pane's status from herdr, which has the ones from before.
async fn follow_agents<P: HerdrPort>(
    port: Arc<P>,
    mut want: tokio::sync::watch::Receiver<Vec<String>>,
    out: impl Fn(StreamItem) + Send + 'static,
    live: impl Fn() + Send + 'static,
) {
    async fn next(rx: &mut Option<tmpsc::Receiver<StreamItem>>) -> Option<StreamItem> {
        match rx {
            Some(rx) => rx.recv().await,
            None => std::future::pending().await,
        }
    }
    let open = |panes: Vec<String>| (!panes.is_empty()).then(|| port.agent_events(panes));
    let forward = |item: StreamItem| {
        if let StreamItem::Event(azulejo_herdr::HerdrEvent::AgentStatusChanged { .. }) = item {
            out(item);
        }
    };
    let mut current = open(want.borrow_and_update().clone());
    let mut pending: Option<tmpsc::Receiver<StreamItem>> = None;
    loop {
        tokio::select! {
            changed = want.changed() => {
                if changed.is_err() {
                    return;
                }
                let panes = want.borrow_and_update().clone();
                if current.is_none() || panes.is_empty() {
                    current = open(panes);
                    pending = None;
                } else {
                    pending = open(panes);
                }
            }
            item = next(&mut current) => match item {
                Some(StreamItem::Connected | StreamItem::Resync) => live(),
                Some(item) => forward(item),
                None => current = pending.take(),
            },
            item = next(&mut pending) => match item {
                Some(StreamItem::Connected) => {
                    current = pending.take();
                    live();
                }
                Some(item) => forward(item),
                None => pending = None,
            },
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
