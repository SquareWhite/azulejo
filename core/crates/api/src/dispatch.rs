//! How the server reaches the engine.
//!
//! The server never touches the engine itself. It calls a [`Dispatcher`],
//! which in the app is a [`CommandSender`]: each command travels over a channel
//! to whichever thread owns the engine, and the reply comes back on a oneshot.

use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use azulejo_engine::{Core, Engine};
use azulejo_protocol::{Command, Error, ErrorCode, Event, Reply};
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};

/// What [`Dispatcher::dispatch`] returns.
pub type DispatchFuture = Pin<Box<dyn Future<Output = Result<Reply, Error>> + Send + 'static>>;

/// Runs a command somewhere and resolves to its reply.
pub trait Dispatcher: Send + Sync + 'static {
    fn dispatch(&self, command: Command) -> DispatchFuture;

    /// Extra fields for `core.version`, such as the herdr session's socket.
    fn info(&self) -> Map<String, Value> {
        Map::new()
    }

    /// Every event from now on, for `events.subscribe`. The server asks once,
    /// when it starts serving. `None` means this dispatcher has no events and
    /// subscriptions stay silent.
    fn events(&self) -> Option<std_mpsc::Receiver<Event>> {
        None
    }
}

/// A dispatcher plus fixed `core.version` fields.
pub struct WithInfo<D> {
    inner: D,
    info: Map<String, Value>,
}

impl<D: Dispatcher> WithInfo<D> {
    pub fn new(inner: D, info: Map<String, Value>) -> Self {
        Self { inner, info }
    }
}

impl<D: Dispatcher> Dispatcher for WithInfo<D> {
    fn dispatch(&self, command: Command) -> DispatchFuture {
        self.inner.dispatch(command)
    }

    fn info(&self) -> Map<String, Value> {
        let mut info = self.inner.info();
        info.extend(self.info.clone());
        info
    }

    fn events(&self) -> Option<std_mpsc::Receiver<Event>> {
        self.inner.events()
    }
}

/// Fans a stream of event batches out to any number of listeners. Pair it
/// with [`spawn_engine`] (pass [`EventFeed::sink`] as `on_events`) and
/// [`WithEvents`] to serve events from an engine that isn't a [`Core`].
#[derive(Clone, Default)]
pub struct EventFeed {
    listeners: Arc<Mutex<Vec<std_mpsc::Sender<Event>>>>,
}

impl EventFeed {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self) -> std_mpsc::Receiver<Event> {
        let (tx, rx) = std_mpsc::channel();
        self.listeners.lock().expect("feed lock").push(tx);
        rx
    }

    pub fn publish(&self, events: &[Event]) {
        self.listeners
            .lock()
            .expect("feed lock")
            .retain(|l| events.iter().all(|e| l.send(e.clone()).is_ok()));
    }

    /// An `on_events` callback for [`spawn_engine`].
    pub fn sink(&self) -> impl FnMut(Vec<Event>) + Send + 'static {
        let feed = self.clone();
        move |events| feed.publish(&events)
    }
}

/// A dispatcher whose events come from an [`EventFeed`].
pub struct WithEvents<D> {
    inner: D,
    feed: EventFeed,
}

impl<D: Dispatcher> WithEvents<D> {
    pub fn new(inner: D, feed: EventFeed) -> Self {
        Self { inner, feed }
    }
}

impl<D: Dispatcher> Dispatcher for WithEvents<D> {
    fn dispatch(&self, command: Command) -> DispatchFuture {
        self.inner.dispatch(command)
    }

    fn info(&self) -> Map<String, Value> {
        self.inner.info()
    }

    fn events(&self) -> Option<std_mpsc::Receiver<Event>> {
        Some(self.feed.subscribe())
    }
}

/// Runs commands on a running [`Core`]. `Core::execute` blocks until the
/// core's thread answers, so each call runs on Tokio's blocking pool.
#[derive(Clone)]
pub struct CoreDispatcher {
    core: Arc<Core>,
}

impl CoreDispatcher {
    pub fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}

impl Dispatcher for CoreDispatcher {
    fn dispatch(&self, command: Command) -> DispatchFuture {
        let core = self.core.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || core.execute(command))
                .await
                .map_err(|_| core_gone())?
                .map(|outcome| outcome.reply)
        })
    }

    fn events(&self) -> Option<std_mpsc::Receiver<Event>> {
        Some(self.core.subscribe())
    }
}

/// A command waiting for the engine, plus the way back to its caller.
#[derive(Debug)]
pub struct Request {
    command: Command,
    reply: oneshot::Sender<Result<Reply, Error>>,
}

impl Request {
    pub fn command(&self) -> &Command {
        &self.command
    }

    /// Answers the caller. A caller that has gone away is ignored.
    pub fn respond(self, result: Result<Reply, Error>) {
        let _ = self.reply.send(result);
    }

    /// Runs the command on `engine`, answers the caller and returns the
    /// events for the shell.
    pub fn run(self, engine: &mut Engine) -> Vec<Event> {
        match engine.execute(self.command) {
            Ok(outcome) => {
                let _ = self.reply.send(Ok(outcome.reply));
                outcome.events
            }
            Err(e) => {
                let _ = self.reply.send(Err(e));
                Vec::new()
            }
        }
    }
}

/// The server's end of the command channel. Cheap to clone.
#[derive(Debug, Clone)]
pub struct CommandSender(mpsc::Sender<Request>);

/// The engine owner's end of the command channel. Use `blocking_recv` on a
/// plain thread, `recv` in async code, or `try_recv` to drain it from a run
/// loop.
pub type CommandReceiver = mpsc::Receiver<Request>;

/// A command channel holding at most `capacity` waiting commands.
pub fn command_channel(capacity: usize) -> (CommandSender, CommandReceiver) {
    let (tx, rx) = mpsc::channel(capacity.max(1));
    (CommandSender(tx), rx)
}

fn core_gone() -> Error {
    Error::new(ErrorCode::Internal, "the Azulejo core has shut down")
}

impl Dispatcher for CommandSender {
    fn dispatch(&self, command: Command) -> DispatchFuture {
        let tx = self.0.clone();
        Box::pin(async move {
            let (reply, rx) = oneshot::channel();
            tx.send(Request { command, reply })
                .await
                .map_err(|_| core_gone())?;
            rx.await.map_err(|_| core_gone())?
        })
    }
}

/// Runs `engine` on its own thread, feeding it commands from the returned
/// sender and passing each command's events to `on_events`. The thread stops
/// once every sender is dropped. Handy for tests and headless use; the app
/// drives the engine from its own loop instead.
pub fn spawn_engine<F>(mut engine: Engine, mut on_events: F) -> (CommandSender, JoinHandle<()>)
where
    F: FnMut(Vec<Event>) + Send + 'static,
{
    let (tx, mut rx) = command_channel(64);
    let handle = std::thread::Builder::new()
        .name("azulejo-engine".into())
        .spawn(move || {
            while let Some(request) = rx.blocking_recv() {
                let events = request.run(&mut engine);
                if !events.is_empty() {
                    on_events(events);
                }
            }
        })
        .expect("spawn engine thread");
    (tx, handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn commands_reach_the_engine_and_events_come_out() {
        let (events_tx, events_rx) = std::sync::mpsc::channel();
        let (tx, handle) = spawn_engine(Engine::new(), move |events| {
            events_tx.send(events).unwrap();
        });
        let reply = tx.dispatch(Command::Ping).await.unwrap();
        assert!(matches!(reply, Reply::Pong { .. }));

        let split =
            Command::from_json(r#"{"method":"layout.split","params":{"direction":"right"}}"#)
                .unwrap();
        let reply = tx.dispatch(split).await.unwrap();
        assert!(matches!(reply, Reply::Created(_)));
        let events = events_rx.recv().unwrap();
        assert!(!events.is_empty());

        drop(tx);
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn a_running_core_answers() {
        let core = Arc::new(Core::start(azulejo_engine::CoreOptions {
            state_path: None,
            ..Default::default()
        }));
        let dispatcher = WithInfo::new(
            CoreDispatcher::new(core),
            Map::from_iter([("herdr_session".to_owned(), Value::from("azulejo"))]),
        );
        let reply = dispatcher.dispatch(Command::Ping).await.unwrap();
        assert!(matches!(reply, Reply::Pong { .. }));
        assert_eq!(dispatcher.info()["herdr_session"], "azulejo");
    }

    #[tokio::test]
    async fn a_dead_core_is_an_internal_error() {
        let (tx, rx) = command_channel(1);
        drop(rx);
        let err = tx.dispatch(Command::Ping).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal);
    }
}
