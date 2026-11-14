//! A self-healing `events.subscribe` stream.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::client::{Backoff, Client};
use crate::event::{HerdrEvent, Line, Subscription, parse_line};
use crate::wire::{self, method};

/// What an [`EventStream`] yields.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    /// The first subscription is live.
    Connected,
    /// The stream reconnected, or herdr dropped events for a slow reader.
    /// Events may have been missed: rebuild state from
    /// [`Client::snapshot`](crate::Client::snapshot).
    Resync,
    /// The connection dropped; the stream is retrying in the background.
    Disconnected(String),
    Event(HerdrEvent),
}

/// Events from one `events.subscribe` connection. Reconnects with backoff for
/// as long as it's alive, and yields [`StreamItem::Resync`] after every
/// reconnect. Dropping it closes the connection.
///
/// Also implements [`tokio_stream::Stream`].
#[derive(Debug)]
pub struct EventStream {
    rx: mpsc::Receiver<StreamItem>,
    task: JoinHandle<()>,
}

impl EventStream {
    pub(crate) fn spawn(client: Client, subscriptions: Vec<Subscription>) -> Self {
        let (tx, rx) = mpsc::channel(1024);
        let task = tokio::spawn(run(client, subscriptions, tx));
        Self { rx, task }
    }

    /// Next item, or `None` once the stream has been shut down.
    pub async fn recv(&mut self) -> Option<StreamItem> {
        self.rx.recv().await
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl tokio_stream::Stream for EventStream {
    type Item = StreamItem;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<StreamItem>> {
        self.rx.poll_recv(cx)
    }
}

async fn run(client: Client, subscriptions: Vec<Subscription>, tx: mpsc::Sender<StreamItem>) {
    let backoff = client.options().connect;
    let subs: Vec<Value> = subscriptions.iter().map(Subscription::to_wire).collect();
    let mut connected_before = false;
    let mut delay = backoff.initial;
    let mut sub_seq = 0u64;

    while !tx.is_closed() {
        sub_seq += 1;
        let reason = match session(&client, &subs, sub_seq, &tx, &mut connected_before).await {
            Ok(reason) => {
                // The stream was up, so start the next retry from scratch.
                delay = backoff.initial;
                reason
            }
            Err(reason) => reason,
        };
        if tx.send(StreamItem::Disconnected(reason)).await.is_err() {
            return;
        }
        tokio::time::sleep(delay).await;
        delay = Backoff::next(&backoff, delay);
    }
}

/// One connection's lifetime. `Ok` if the subscription got going before it
/// ended, `Err` if it never did. Both carry the reason it ended.
async fn session(
    client: &Client,
    subs: &[Value],
    seq: u64,
    tx: &mpsc::Sender<StreamItem>,
    connected_before: &mut bool,
) -> Result<String, String> {
    let once = Backoff {
        give_up_after: Duration::ZERO,
        ..client.options().connect
    };
    let conn = client
        .connect_with_backoff(once)
        .await
        .map_err(|e| e.to_string())?;
    let mut write = conn.write;
    let read = crate::transport::GuardedRead {
        inner: conn.read,
        guard: conn.guard,
    };
    let line = wire::encode(
        &format!("azj-sub-{seq}"),
        method::EVENTS_SUBSCRIBE,
        serde_json::json!({ "subscriptions": subs }),
    )
    .map_err(|e| e.to_string())?;
    write
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    let mut lines = BufReader::new(read).lines();
    let mut started = false;
    loop {
        let next = lines.next_line().await;
        let text = match next {
            Ok(Some(text)) => text,
            Ok(None) | Err(_) if lines.get_ref().get_ref().guard.is_some() => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                let why = lines.get_ref().get_ref().bridge_error().expect("bridge");
                return finish(started, why.to_string());
            }
            Ok(None) => return finish(started, "herdr closed the event stream".into()),
            Err(e) => return finish(started, e.to_string()),
        };
        let item = match parse_line(&text) {
            Ok(Line::Started) => {
                started = true;
                let item = if *connected_before {
                    StreamItem::Resync
                } else {
                    StreamItem::Connected
                };
                *connected_before = true;
                item
            }
            Ok(Line::Event(event)) => StreamItem::Event(event),
            Ok(Line::Lost) => StreamItem::Resync,
            Ok(Line::Error { code, message }) => {
                return finish(started, format!("herdr error {code}: {message}"));
            }
            Err(e) => return finish(started, format!("bad event line: {e}")),
        };
        if tx.send(item).await.is_err() {
            return Ok("stream dropped".into());
        }
    }
}

fn finish(started: bool, reason: String) -> Result<String, String> {
    if started { Ok(reason) } else { Err(reason) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ClientOptions;
    use std::path::PathBuf;
    use tokio::net::UnixListener;

    fn temp_socket(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("azj-hs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("s.sock")
    }

    const TAB_CLOSED: &str = r#"{"data":{"tab_id":"w1:t2","type":"tab_closed","workspace_id":"w1"},"event":"tab_closed"}"#;

    /// Accept one subscriber, check its request, send `lines`, hang up.
    async fn serve_once(listener: &UnixListener, lines: &[&str]) -> Value {
        let (stream, _) = listener.accept().await.unwrap();
        let (r, mut w) = stream.into_split();
        let mut req = String::new();
        BufReader::new(r).read_line(&mut req).await.unwrap();
        let req: Value = serde_json::from_str(&req).unwrap();
        for l in lines {
            w.write_all(format!("{l}\n").as_bytes()).await.unwrap();
        }
        req
    }

    #[tokio::test]
    async fn reconnects_and_asks_for_resync() {
        let path = temp_socket("re");
        let listener = UnixListener::bind(&path).unwrap();
        let client = Client::new(
            &path,
            ClientOptions {
                connect: Backoff {
                    initial: Duration::from_millis(10),
                    max: Duration::from_millis(50),
                    give_up_after: Duration::from_secs(1),
                },
                ..ClientOptions::default()
            },
        );
        let mut events = client.subscribe(vec![
            Subscription::TabClosed,
            Subscription::AgentStatus("w1:p1".into()),
        ]);
        let started = r#"{"id":"x","result":{"type":"subscription_started"}}"#;

        let req = serve_once(&listener, &[started, TAB_CLOSED]).await;
        assert_eq!(req["method"], "events.subscribe");
        assert_eq!(
            req["params"]["subscriptions"][1]["type"],
            "pane.agent_status_changed"
        );
        assert_eq!(events.recv().await, Some(StreamItem::Connected));
        assert!(matches!(
            events.recv().await,
            Some(StreamItem::Event(HerdrEvent::TabClosed { .. }))
        ));
        assert!(matches!(
            events.recv().await,
            Some(StreamItem::Disconnected(_))
        ));

        serve_once(&listener, &[started, TAB_CLOSED]).await;
        assert_eq!(events.recv().await, Some(StreamItem::Resync));
        assert!(matches!(
            events.recv().await,
            Some(StreamItem::Event(HerdrEvent::TabClosed { .. }))
        ));
    }
}
