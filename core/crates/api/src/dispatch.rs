//! How the server reaches the engine.
//!
//! The server never touches the engine itself. It calls a [`Dispatcher`],
//! which in the app is a [`CommandSender`]: each command travels over a channel
//! to whichever thread owns the engine, and the reply comes back on a oneshot.

use std::future::Future;
use std::pin::Pin;
use std::thread::JoinHandle;

use azulejo_engine::Engine;
use azulejo_protocol::{Command, Error, ErrorCode, Event, Reply};
use tokio::sync::{mpsc, oneshot};

/// What [`Dispatcher::dispatch`] returns.
pub type DispatchFuture = Pin<Box<dyn Future<Output = Result<Reply, Error>> + Send + 'static>>;

/// Runs a command somewhere and resolves to its reply.
pub trait Dispatcher: Send + Sync + 'static {
    fn dispatch(&self, command: Command) -> DispatchFuture;
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
    async fn a_dead_core_is_an_internal_error() {
        let (tx, rx) = command_channel(1);
        drop(rx);
        let err = tx.dispatch(Command::Ping).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal);
    }
}
