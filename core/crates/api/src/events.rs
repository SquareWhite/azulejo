//! Event streams (F22): one hub per server fans the core's events out to
//! every `events.subscribe` on every connection.
//!
//! The core never waits for a subscriber. Each subscription has a bounded
//! buffer; when it's full, events for that subscription are dropped and
//! counted, and the count goes out as `events.lagged` just before the next
//! event that fits. Name filters run here, before the buffer; workspace and
//! tile filters need to know which tab and workspace a tile lives in, so they
//! run per subscription on the connection ([`Scope`]).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use azulejo_protocol::{
    Command, Error, ErrorCode, Event, EventsSubscribe, Node, Reply, Session, TabId, TileId,
    WorkspaceId, filter_matches,
};
use serde_json::{Value, json};
use tokio::sync::mpsc::{self, error::TrySendError};

use crate::Dispatcher;

/// Events a subscription may hold before it starts dropping them.
pub const DEFAULT_EVENT_BUFFER: usize = 1024;

/// One event, serialised once for every subscriber.
#[derive(Debug)]
pub(crate) struct Published {
    pub event: Event,
    /// `{"event": …, "data": …}`.
    pub json: Value,
    pub name: String,
}

/// What a subscription's receiver yields.
#[derive(Debug, Clone)]
pub(crate) enum Item {
    Event(Arc<Published>),
    /// This many events were dropped just before the next one.
    Lagged(u64),
}

struct Sub {
    filter: Vec<String>,
    tx: mpsc::Sender<Item>,
    dropped: u64,
}

#[derive(Default)]
struct Subs {
    next_id: u64,
    subs: HashMap<u64, Sub>,
}

/// Fans events out to subscriptions. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Hub {
    subs: Arc<Mutex<Subs>>,
    buffer: usize,
    closed: Arc<AtomicBool>,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hub")
            .field("subscriptions", &self.len())
            .field("buffer", &self.buffer)
            .finish()
    }
}

impl Hub {
    pub fn new(buffer: usize) -> Hub {
        Hub {
            subs: Arc::default(),
            buffer: buffer.max(1),
            closed: Arc::default(),
        }
    }

    /// Registers a subscription; events published from now on reach it.
    pub fn subscribe(&self, filter: Vec<String>) -> (u64, mpsc::Receiver<Item>) {
        let (tx, rx) = mpsc::channel(self.buffer);
        let mut subs = self.subs.lock().expect("hub lock");
        subs.next_id += 1;
        let id = subs.next_id;
        subs.subs.insert(
            id,
            Sub {
                filter,
                tx,
                dropped: 0,
            },
        );
        (id, rx)
    }

    pub fn unsubscribe(&self, id: u64) -> bool {
        self.subs
            .lock()
            .expect("hub lock")
            .subs
            .remove(&id)
            .is_some()
    }

    pub fn len(&self) -> usize {
        self.subs.lock().expect("hub lock").subs.len()
    }

    /// Hands `event` to every matching subscription without ever blocking.
    pub fn publish(&self, event: Event) {
        let mut subs = self.subs.lock().expect("hub lock");
        if subs.subs.is_empty() {
            return;
        }
        let json = serde_json::to_value(&event).unwrap_or(Value::Null);
        let name = json
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let published = Arc::new(Published { event, json, name });
        subs.subs.retain(|_, sub| {
            if !filter_matches(&sub.filter, &published.name) {
                return true;
            }
            if sub.dropped > 0 {
                match sub.tx.try_send(Item::Lagged(sub.dropped)) {
                    Ok(()) => sub.dropped = 0,
                    Err(TrySendError::Full(_)) => {
                        sub.dropped += 1;
                        return true;
                    }
                    Err(TrySendError::Closed(_)) => return false,
                }
            }
            match sub.tx.try_send(Item::Event(published.clone())) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) => {
                    sub.dropped += 1;
                    true
                }
                Err(TrySendError::Closed(_)) => false,
            }
        });
    }

    /// Feeds the hub from `events` on a background thread until the source
    /// ends or [`Hub::close`] is called.
    pub fn pump(&self, events: std_mpsc::Receiver<Event>) {
        let hub = self.clone();
        let spawned = std::thread::Builder::new()
            .name("azulejo-api-events".into())
            .spawn(move || {
                while !hub.closed.load(Ordering::Relaxed) {
                    match events.recv_timeout(Duration::from_millis(200)) {
                        Ok(event) => hub.publish(event),
                        Err(std_mpsc::RecvTimeoutError::Timeout) => {}
                        Err(std_mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            });
        if spawned.is_err() {
            // No thread, no events; requests still work.
            self.close();
        }
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.subs.lock().expect("hub lock").subs.clear();
    }
}

/// A subscription's workspace and tile filters, with just enough of the
/// session to tell which workspace a tile or tab belongs to. Seeded from
/// `session.get` and kept current from the events themselves.
#[derive(Debug, Default)]
pub(crate) struct Scope {
    workspace: Option<WorkspaceId>,
    tile: Option<TileId>,
    tab_workspace: HashMap<TabId, WorkspaceId>,
    tile_tab: HashMap<TileId, TabId>,
    /// Tiles in each tab, for `layout.changed`.
    tab_tiles: HashMap<TabId, HashSet<TileId>>,
    focused_tile: Option<TileId>,
}

impl Scope {
    pub fn new(workspace: Option<WorkspaceId>, tile: Option<TileId>) -> Scope {
        Scope {
            workspace,
            tile,
            ..Scope::default()
        }
    }

    pub fn is_open(&self) -> bool {
        self.workspace.is_none() && self.tile.is_none()
    }

    pub fn seed(&mut self, session: &Session) {
        for window in &session.windows {
            for ws in &window.workspaces {
                for tab in &ws.tabs {
                    self.tab_workspace.insert(tab.id, ws.id);
                    let mut leaves = HashSet::new();
                    collect_leaves(&tab.root, &mut leaves);
                    for tile in &leaves {
                        self.tile_tab.insert(*tile, tab.id);
                    }
                    self.tab_tiles.insert(tab.id, leaves);
                }
            }
        }
        if let Some(window) = session
            .focused_window
            .and_then(|id| session.windows.iter().find(|w| w.id == id))
            && let Some(ws) = window
                .workspaces
                .iter()
                .find(|w| w.id == window.active_workspace)
            && let Some(tab) = ws
                .active_tab
                .and_then(|id| ws.tabs.iter().find(|t| t.id == id))
        {
            self.focused_tile = Some(tab.focused);
        }
    }

    /// Updates what's known from `event` (`json` is its serialised form),
    /// then says whether it passes the filters.
    pub fn admit(&mut self, event: &Event, json: &Value) -> bool {
        let previous_focus = self.focused_tile;
        let ws_of_tab = |s: &Scope, tab: &TabId| s.tab_workspace.get(tab).copied();
        // What the event is about, worked out before forgetting closed things.
        let (workspace, tiles): (Option<WorkspaceId>, Vec<TileId>) = match event {
            Event::WorkspaceCreated { workspace, .. }
            | Event::WorkspaceUpdated { workspace, .. } => (Some(workspace.id), vec![]),
            Event::WorkspaceClosed { workspace, .. } => (Some(*workspace), vec![]),
            Event::TabCreated { workspace, tab } | Event::TabUpdated { workspace, tab } => {
                self.tab_workspace.insert(tab.id, *workspace);
                (Some(*workspace), vec![])
            }
            Event::TabClosed { workspace, .. } | Event::TabsReordered { workspace, .. } => {
                (Some(*workspace), vec![])
            }
            Event::TileCreated { tab, tile } | Event::TileUpdated { tab, tile } => {
                self.tile_tab.insert(tile.id, *tab);
                self.tab_tiles.entry(*tab).or_default().insert(tile.id);
                (ws_of_tab(self, tab), vec![tile.id])
            }
            Event::TileClosed { tab, tile, .. } => (ws_of_tab(self, tab), vec![tile.id]),
            Event::LayoutChanged(layout) => {
                self.tab_workspace.insert(layout.tab, layout.workspace);
                let mut leaves = HashSet::new();
                collect_leaves(&layout.tree, &mut leaves);
                for tile in &leaves {
                    self.tile_tab.insert(*tile, layout.tab);
                }
                let tiles = leaves.iter().copied().collect();
                self.tab_tiles.insert(layout.tab, leaves);
                (Some(layout.workspace), tiles)
            }
            Event::FocusChanged(focus) => {
                self.focused_tile = focus.tile;
                let tiles = focus.tile.into_iter().chain(previous_focus).collect();
                (focus.workspace, tiles)
            }
            Event::WindowCreated { .. }
            | Event::WindowClosed { .. }
            | Event::WindowUpdated { .. }
            | Event::WorkspacesReordered { .. }
            | Event::HerdrStatus { .. } => (None, vec![]),
            // Everything else (`preview.updated`, `agent.state_changed`, events
            // added later): read `workspace`, `tab` and `tile` off the data.
            _ => self.about(&json["data"]),
        };
        let admitted = self.workspace.is_none_or(|want| workspace == Some(want))
            && self.tile.is_none_or(|want| tiles.contains(&want));
        // Forget what's gone.
        match event {
            Event::TileClosed { tab, tile, .. } => {
                self.tile_tab.remove(&tile.id);
                if let Some(set) = self.tab_tiles.get_mut(tab) {
                    set.remove(&tile.id);
                }
            }
            Event::TabClosed { tab, .. } => {
                self.tab_workspace.remove(tab);
                for tile in self.tab_tiles.remove(tab).unwrap_or_default() {
                    self.tile_tab.remove(&tile);
                }
            }
            _ => {}
        }
        admitted
    }

    /// The workspace and tile an event's data names, by field.
    fn about(&self, data: &Value) -> (Option<WorkspaceId>, Vec<TileId>) {
        let id = |key: &str| {
            let v = &data[key];
            v.as_str().or_else(|| v["id"].as_str())
        };
        let tile: Option<TileId> = id("tile").and_then(|s| s.parse().ok());
        let tab: Option<TabId> = id("tab").and_then(|s| s.parse().ok());
        let workspace = id("workspace")
            .and_then(|s| s.parse::<WorkspaceId>().ok())
            .or_else(|| tab.and_then(|tab| self.tab_workspace.get(&tab).copied()))
            .or_else(|| tile.and_then(|t| self.workspace_of_tile(&t)));
        (workspace, tile.into_iter().collect())
    }

    fn workspace_of_tile(&self, tile: &TileId) -> Option<WorkspaceId> {
        self.tile_tab
            .get(tile)
            .and_then(|tab| self.tab_workspace.get(tab))
            .copied()
    }
}

fn collect_leaves(node: &Node, out: &mut HashSet<TileId>) {
    match node {
        Node::Leaf { id } => {
            out.insert(*id);
        }
        Node::Split { a, b, .. } => {
            collect_leaves(a, out);
            collect_leaves(b, out);
        }
    }
}

/// A socket connection's subscriptions, and its way out.
pub(crate) struct Connection {
    hub: Option<Hub>,
    out: mpsc::Sender<Vec<u8>>,
    running: HashMap<u64, tokio::task::JoinHandle<()>>,
    /// Subscriptions whose reply hasn't been queued yet. They start in
    /// [`Connection::start_pending`], so a subscriber always reads the
    /// `subscribed` reply before the first event.
    pending: Vec<(u64, mpsc::Receiver<Item>, Scope)>,
}

impl Connection {
    pub fn new(hub: Option<Hub>, out: mpsc::Sender<Vec<u8>>) -> Connection {
        Connection {
            hub,
            out,
            running: HashMap::new(),
            pending: Vec::new(),
        }
    }

    pub async fn subscribe(
        &mut self,
        params: EventsSubscribe,
        dispatcher: &dyn Dispatcher,
    ) -> Result<u64, Error> {
        let Some(hub) = &self.hub else {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "this server has no event source",
            ));
        };
        // Subscribe before reading the session, so nothing falls in between.
        let (id, rx) = hub.subscribe(params.filter);
        let mut scope = Scope::new(params.workspace, params.tile);
        if !scope.is_open() {
            match dispatcher.dispatch(Command::SessionGet).await {
                Ok(Reply::Session { session }) => scope.seed(&session),
                Ok(_) => {}
                Err(e) => {
                    hub.unsubscribe(id);
                    return Err(e);
                }
            }
        }
        self.pending.push((id, rx, scope));
        Ok(id)
    }

    pub fn unsubscribe(&mut self, id: u64) -> Result<(), Error> {
        let pending = self.pending.iter().position(|(p, ..)| *p == id);
        let found = match pending {
            Some(i) => {
                self.pending.remove(i);
                true
            }
            None => match self.running.remove(&id) {
                Some(task) => {
                    task.abort();
                    true
                }
                None => false,
            },
        };
        if !found {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("no subscription {id} on this connection"),
            ));
        }
        if let Some(hub) = &self.hub {
            hub.unsubscribe(id);
        }
        Ok(())
    }

    /// Starts forwarding for subscriptions whose reply is now queued.
    pub fn start_pending(&mut self) {
        for (id, mut rx, mut scope) in self.pending.drain(..) {
            let out = self.out.clone();
            let task = tokio::spawn(async move {
                while let Some(item) = rx.recv().await {
                    let message = match item {
                        Item::Event(p) => {
                            if !scope.admit(&p.event, &p.json) {
                                continue;
                            }
                            let mut params = match &p.json {
                                Value::Object(map) => map.clone(),
                                _ => continue,
                            };
                            params.insert("subscription".into(), id.into());
                            json!({"jsonrpc": "2.0", "method": "events.event", "params": params})
                        }
                        Item::Lagged(dropped) => json!({
                            "jsonrpc": "2.0",
                            "method": "events.lagged",
                            "params": {"subscription": id, "dropped": dropped},
                        }),
                    };
                    let mut line = message.to_string().into_bytes();
                    line.push(b'\n');
                    if out.send(line).await.is_err() {
                        break;
                    }
                }
            });
            self.running.insert(id, task);
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let ids: Vec<u64> = self
            .running
            .keys()
            .copied()
            .chain(self.pending.iter().map(|(id, ..)| *id))
            .collect();
        for task in self.running.values() {
            task.abort();
        }
        if let Some(hub) = &self.hub {
            for id in ids {
                hub.unsubscribe(id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use azulejo_protocol::{AgentState, AgentStateChange, Focus};

    impl Scope {
        fn admit_event(&mut self, event: &Event) -> bool {
            let json = serde_json::to_value(event).unwrap();
            self.admit(event, &json)
        }
    }

    fn herdr_status() -> Event {
        Event::HerdrStatus {
            connected: true,
            message: None,
        }
    }

    fn agent(tile: TileId) -> Event {
        Event::AgentStateChanged(AgentStateChange {
            tile,
            state: AgentState::Working,
            previous: None,
            agent: None,
        })
    }

    #[test]
    fn slow_subscribers_lag_and_are_told_how_much() {
        let hub = Hub::new(2);
        let (_, mut rx) = hub.subscribe(vec![]);
        for _ in 0..5 {
            hub.publish(herdr_status());
        }
        // Two fit, three were dropped.
        assert!(matches!(rx.try_recv(), Ok(Item::Event(_))));
        assert!(matches!(rx.try_recv(), Ok(Item::Event(_))));
        assert!(rx.try_recv().is_err());
        // With room again, the notice goes first, then the event.
        hub.publish(herdr_status());
        assert!(matches!(rx.try_recv(), Ok(Item::Lagged(3))));
        assert!(matches!(rx.try_recv(), Ok(Item::Event(_))));
        assert!(rx.try_recv().is_err());
        // Room for the notice but not the event: the event counts as dropped.
        for _ in 0..2 {
            hub.publish(herdr_status());
        }
        hub.publish(herdr_status());
        let _ = rx.try_recv();
        hub.publish(herdr_status());
        assert!(matches!(rx.try_recv(), Ok(Item::Event(_))));
        assert!(matches!(rx.try_recv(), Ok(Item::Lagged(1))));
        assert!(
            rx.try_recv().is_err(),
            "the event after the notice didn't fit"
        );
    }

    #[test]
    fn closed_receivers_are_forgotten_on_the_next_publish() {
        let hub = Hub::new(4);
        let (_, rx) = hub.subscribe(vec![]);
        let (_, _keep) = hub.subscribe(vec![]);
        assert_eq!(hub.len(), 2);
        drop(rx);
        hub.publish(herdr_status());
        assert_eq!(hub.len(), 1);
    }

    #[test]
    fn name_filters_apply_before_the_buffer() {
        let hub = Hub::new(1);
        let (_, mut rx) = hub.subscribe(vec!["agent.".into()]);
        for _ in 0..10 {
            hub.publish(herdr_status());
        }
        hub.publish(agent(TileId::new()));
        match rx.try_recv() {
            Ok(Item::Event(p)) => assert_eq!(p.name, "agent.state_changed"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn scope_follows_tiles_into_their_workspace() {
        let ws = WorkspaceId::new();
        let other_ws = WorkspaceId::new();
        let tab = TabId::new();
        let tile = TileId::new();
        let mut scope = Scope::new(Some(ws), None);
        assert!(scope.admit_event(&Event::TabCreated {
            workspace: ws,
            tab: azulejo_protocol::TabInfo {
                id: tab,
                name: None,
                kind: azulejo_protocol::LayoutKind::Manual,
            },
        }));
        assert!(!scope.admit_event(&Event::TabCreated {
            workspace: other_ws,
            tab: azulejo_protocol::TabInfo {
                id: TabId::new(),
                name: None,
                kind: azulejo_protocol::LayoutKind::Manual,
            },
        }));
        assert!(
            scope.admit_event(&Event::LayoutChanged(azulejo_protocol::LayoutState {
                window: azulejo_protocol::WindowId::new(),
                workspace: ws,
                tab,
                tree: Node::Leaf { id: tile },
                focused: tile,
                zoomed: None,
                kind: azulejo_protocol::LayoutKind::Manual,
                params: Default::default(),
                order: Vec::new(),
                rects: Vec::new(),
            }))
        );
        assert!(
            scope.admit_event(&agent(tile)),
            "tile's workspace was learnt"
        );
        assert!(!scope.admit_event(&agent(TileId::new())));
        assert!(!scope.admit_event(&herdr_status()));
    }

    #[test]
    fn tile_scope_sees_focus_moving_away() {
        let tile = TileId::new();
        let mut scope = Scope::new(None, Some(tile));
        let focus = |t: Option<TileId>| {
            Event::FocusChanged(Focus {
                tile: t,
                ..Focus::default()
            })
        };
        assert!(scope.admit_event(&focus(Some(tile))));
        assert!(
            scope.admit_event(&focus(Some(TileId::new()))),
            "focus left it"
        );
        assert!(!scope.admit_event(&focus(Some(TileId::new()))));
        assert!(scope.admit_event(&agent(tile)));
    }
}
