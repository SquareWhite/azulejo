//! Async client for herdr's socket API.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::transport::{BridgeCommand, Conn, Transport, spawn_bridge};

use crate::MIN_PROTOCOL;
use crate::error::{Error, Result};
use crate::event::Subscription;
use crate::stream::EventStream;
use crate::types::{
    EnsuredWorkspace, MoveTo, Moved, PaneId, PaneInfo, ProcessInfo, ReadResult, ReadSource,
    Resolved, ServerInfo, Snapshot, SplitSpec, Survey, TabId, TabLayout, TerminalId, TerminalRef,
    TerminalSpec, WorkspaceId, WorkspaceInfo, WorkspaceSpec, ZoomState,
};
use crate::wire::{self, method};

/// Exponential backoff for (re)connecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
    /// Stop retrying after this long. Zero means try once.
    pub give_up_after: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(50),
            max: Duration::from_secs(1),
            give_up_after: Duration::from_secs(3),
        }
    }
}

impl Backoff {
    pub(crate) fn next(&self, current: Duration) -> Duration {
        (current * 2).min(self.max)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientOptions {
    /// How long to wait for a reply once the request is sent.
    pub request_timeout: Duration,
    /// Retry policy while the socket refuses connections (herdr restarting).
    pub connect: Backoff,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(5),
            connect: Backoff::default(),
        }
    }
}

/// A handle on one herdr server. Cheap to clone.
///
/// herdr serves one request per connection, so every call opens a fresh
/// connection. Connecting retries with backoff, which rides out a herdr
/// restart. A request that was already sent is never retried, because most
/// herdr methods aren't idempotent.
#[derive(Debug, Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    transport: Transport,
    opts: ClientOptions,
    next_id: AtomicU64,
}

impl Client {
    pub fn new(socket: impl Into<PathBuf>, opts: ClientOptions) -> Self {
        Self::with_transport(Transport::Socket(socket.into()), opts)
    }

    /// A client whose connections are bridge processes (a remote machine,
    /// ADR 0011).
    pub fn bridged(bridge: BridgeCommand, opts: ClientOptions) -> Self {
        Self::with_transport(Transport::Bridge(bridge), opts)
    }

    pub fn with_transport(transport: Transport, opts: ClientOptions) -> Self {
        Self {
            inner: Arc::new(Inner {
                transport,
                opts,
                next_id: AtomicU64::new(1),
            }),
        }
    }

    /// Connect, ping and check the protocol version.
    pub async fn connect(
        socket: impl Into<PathBuf>,
        opts: ClientOptions,
    ) -> Result<(Self, ServerInfo)> {
        let client = Self::new(socket, opts);
        let info = client.ping().await?;
        Ok((client, info))
    }

    /// The local socket, for a client that talks to this machine's herdr.
    pub fn socket_path(&self) -> Option<&Path> {
        match &self.inner.transport {
            Transport::Socket(path) => Some(path),
            Transport::Bridge(_) => None,
        }
    }

    pub fn transport(&self) -> &Transport {
        &self.inner.transport
    }

    pub fn options(&self) -> ClientOptions {
        self.inner.opts
    }

    /// Ping and fail with [`Error::Version`] if herdr is too old.
    pub async fn ping(&self) -> Result<ServerInfo> {
        let result = self.call(method::PING, json!({})).await?;
        let info: ServerInfo = serde_json::from_value(result.clone())
            .map_err(|e| Error::Protocol(format!("ping: {e}: {}", wire::short(&result))))?;
        if info.protocol < MIN_PROTOCOL {
            return Err(Error::Version {
                found: info.protocol,
                min: MIN_PROTOCOL,
                version: info.version,
            });
        }
        Ok(info)
    }

    /// Ask the server to shut down. Panes die with it.
    pub async fn stop_server(&self) -> Result<()> {
        self.call(method::SERVER_STOP, json!({})).await.map(drop)
    }

    pub async fn snapshot(&self) -> Result<Snapshot> {
        let r = self.call(method::SESSION_SNAPSHOT, json!({})).await?;
        wire::field(&r, "snapshot")
    }

    /// A tab's split tree. Read-only (`layout.export`); Azulejo never calls
    /// `layout.apply`, which replaces live panes.
    pub async fn layout_export(&self, tab: &TabId) -> Result<TabLayout> {
        let r = self
            .call(method::LAYOUT_EXPORT, json!({ "tab_id": tab }))
            .await?;
        wire::field(&r, "layout")
    }

    /// Everything Azulejo needs to adopt what's already running: the
    /// snapshot, plus the split tree of each tab with more than one pane.
    /// Only reads. A tab that closes between the two calls is skipped.
    pub async fn survey(&self) -> Result<Survey> {
        let snapshot = self.snapshot().await?;
        let mut layouts = Vec::new();
        for tab in &snapshot.tabs {
            let panes = snapshot
                .panes
                .iter()
                .filter(|p| p.tab_id == tab.tab_id)
                .count();
            if panes < 2 {
                continue;
            }
            match self.layout_export(&tab.tab_id).await {
                Ok(layout) => layouts.push(layout),
                Err(e) if e.is_not_found() => {}
                Err(e) => return Err(e),
            }
        }
        Ok(Survey {
            workspaces: snapshot.workspaces,
            tabs: snapshot.tabs,
            panes: snapshot.panes,
            layouts,
            focused_workspace_id: snapshot.focused_workspace_id,
        })
    }

    // ---- workspaces ------------------------------------------------------

    pub async fn workspaces(&self) -> Result<Vec<WorkspaceInfo>> {
        let r = self.call(method::WORKSPACE_LIST, json!({})).await?;
        wire::field(&r, "workspaces")
    }

    /// `None` if the workspace doesn't exist (any more).
    pub async fn workspace(&self, id: &WorkspaceId) -> Result<Option<WorkspaceInfo>> {
        match self
            .call(method::WORKSPACE_GET, json!({ "workspace_id": id }))
            .await
        {
            Ok(r) => wire::field(&r, "workspace").map(Some),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Create a herdr workspace. herdr always creates a first tab with one
    /// terminal; it's returned so the caller can use it as the first
    /// terminal tile or kill it.
    pub async fn create_workspace(
        &self,
        spec: &WorkspaceSpec,
    ) -> Result<(WorkspaceInfo, TerminalRef)> {
        let r = self
            .call(
                method::WORKSPACE_CREATE,
                json!({
                    "label": spec.label,
                    "cwd": spec.cwd,
                    "env": env_map(&spec.env),
                    "focus": false,
                }),
            )
            .await?;
        let workspace: WorkspaceInfo = wire::field(&r, "workspace")?;
        let pane: PaneInfo = wire::field(&r, "root_pane")?;
        Ok((workspace, pane.terminal_ref()))
    }

    /// The app ↔ herdr workspace mapping is 1:1. Reuse `existing` if herdr
    /// still has it, otherwise create a new one. herdr deletes a workspace
    /// when its last tab closes, so callers must not cache the ID blindly.
    /// After a server restart herdr restores workspaces (same IDs) with
    /// fresh processes; use [`Client::reresolve`] for the terminals in it.
    pub async fn ensure_workspace(
        &self,
        existing: Option<&WorkspaceId>,
        spec: &WorkspaceSpec,
    ) -> Result<EnsuredWorkspace> {
        if let Some(id) = existing
            && let Some(workspace) = self.workspace(id).await?
        {
            return Ok(EnsuredWorkspace {
                workspace,
                created_terminal: None,
            });
        }
        let (workspace, terminal) = self.create_workspace(spec).await?;
        Ok(EnsuredWorkspace {
            workspace,
            created_terminal: Some(terminal),
        })
    }

    /// Close a herdr workspace and kill every process in it.
    pub async fn close_workspace(&self, id: &WorkspaceId) -> Result<()> {
        self.call(method::WORKSPACE_CLOSE, json!({ "workspace_id": id }))
            .await
            .map(drop)
    }

    // ---- terminals ---------------------------------------------------------

    /// Create the herdr tab behind a new terminal tile: one tab per tile,
    /// labelled, with the tile's env vars.
    pub async fn create_terminal(&self, spec: &TerminalSpec) -> Result<TerminalRef> {
        let r = self
            .call(
                method::TAB_CREATE,
                json!({
                    "workspace_id": spec.workspace_id,
                    "label": spec.label,
                    "cwd": spec.cwd,
                    "env": env_map(&spec.env),
                    "focus": false,
                }),
            )
            .await?;
        let pane: PaneInfo = wire::field(&r, "root_pane")?;
        Ok(pane.terminal_ref())
    }

    /// Kill a terminal's process (explicit user choice, F14). Detaching
    /// needs no herdr call: the tile just ends its attach process.
    ///
    /// Closes the pane rather than the tab, so panes someone split off in the
    /// same tab (e.g. from a phone) survive. herdr closes the tab when its
    /// last pane goes, and the workspace when its last tab goes.
    pub async fn kill_terminal(&self, pane: &PaneId) -> Result<()> {
        self.call(method::PANE_CLOSE, json!({ "pane_id": pane }))
            .await
            .map(drop)
    }

    // ---- layout (ADR 0012) ---------------------------------------------------

    /// A terminal made by splitting a pane of an existing tab, with the
    /// tile's env vars. The new pane is the split's second child.
    pub async fn split_terminal(&self, spec: &SplitSpec) -> Result<TerminalRef> {
        let r = self
            .call(
                method::PANE_SPLIT,
                json!({
                    "target_pane_id": spec.target,
                    "direction": spec.direction,
                    "ratio": spec.ratio,
                    "cwd": spec.cwd,
                    "env": env_map(&spec.env),
                    "focus": false,
                }),
            )
            .await?;
        let pane: PaneInfo = wire::field(&r, "pane")?;
        Ok(pane.terminal_ref())
    }

    /// Moves a pane to another tab or a new one. Its process, terminal ID
    /// and any attached client stay; herdr closes a tab its last pane left.
    /// A refusal (`same_tab`, `zoomed_tab`) isn't an error: see
    /// [`Moved::changed`].
    pub async fn move_pane(&self, pane: &PaneId, to: &MoveTo) -> Result<Moved> {
        let destination = match to {
            MoveTo::Tab {
                tab,
                split,
                target,
                ratio,
            } => json!({
                "type": "tab",
                "tab_id": tab,
                "split": split,
                "target_pane_id": target,
                "ratio": ratio,
            }),
            MoveTo::NewTab { workspace, label } => json!({
                "type": "new_tab",
                "workspace_id": workspace,
                "label": label,
            }),
        };
        let r = self
            .call(
                method::PANE_MOVE,
                json!({ "pane_id": pane, "destination": destination, "focus": false }),
            )
            .await?;
        let result = r
            .get("move_result")
            .ok_or_else(|| Error::Protocol(format!("pane.move: {}", wire::short(&r))))?;
        Ok(Moved {
            pane: wire::field(result, "pane")?,
            changed: result.get("changed").and_then(Value::as_bool) == Some(true),
            reason: result
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }

    /// Swaps two panes of one tab; the split ratios stay where they are.
    pub async fn swap_panes(&self, a: &PaneId, b: &PaneId) -> Result<()> {
        self.call(
            method::PANE_SWAP,
            json!({ "source_pane_id": a, "target_pane_id": b }),
        )
        .await
        .map(drop)
    }

    /// Sets one split's ratio (its first child's share). `path` picks the
    /// split from the root: `false` is the first child, `true` the second.
    /// Returns the tab's tree after the change.
    pub async fn set_split_ratio(
        &self,
        tab: &TabId,
        path: &[bool],
        ratio: f64,
    ) -> Result<TabLayout> {
        let r = self
            .call(
                method::LAYOUT_SET_SPLIT_RATIO,
                json!({ "tab_id": tab, "path": path, "ratio": ratio }),
            )
            .await?;
        wire::field(&r, "layout")
    }

    /// Zooms `pane` (and focuses it) or unzooms its tab. Returns the zoom
    /// as herdr has it right after: a tab's only pane doesn't zoom.
    pub async fn zoom_pane(&self, pane: &PaneId, on: bool) -> Result<ZoomState> {
        let mode = if on { "on" } else { "off" };
        let r = self
            .call(method::PANE_ZOOM, json!({ "pane_id": pane, "mode": mode }))
            .await?;
        let zoom = r
            .get("zoom")
            .ok_or_else(|| Error::Protocol(format!("pane.zoom: {}", wire::short(&r))))?;
        Ok(ZoomState {
            zoomed: zoom.get("zoomed").and_then(Value::as_bool) == Some(true),
            focused: zoom
                .get("focused_pane_id")
                .and_then(Value::as_str)
                .map(PaneId::from),
        })
    }

    pub async fn rename_tab(&self, tab: &TabId, label: &str) -> Result<()> {
        self.call(method::TAB_RENAME, json!({ "tab_id": tab, "label": label }))
            .await
            .map(drop)
    }

    /// `None` if the pane doesn't exist.
    pub async fn pane(&self, pane: &PaneId) -> Result<Option<PaneInfo>> {
        match self
            .call(method::PANE_GET, json!({ "pane_id": pane }))
            .await
        {
            Ok(r) => wire::field(&r, "pane").map(Some),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn panes(&self) -> Result<Vec<PaneInfo>> {
        let r = self.call(method::PANE_LIST, json!({})).await?;
        wire::field(&r, "panes")
    }

    /// Find the pane currently showing `terminal`. Pane IDs change when a
    /// pane moves between workspaces; terminal IDs don't.
    pub async fn resolve_terminal(&self, terminal: &TerminalId) -> Result<Option<PaneInfo>> {
        Ok(self
            .panes()
            .await?
            .into_iter()
            .find(|p| &p.terminal_id == terminal))
    }

    /// Work out what happened to a stored terminal, e.g. on relaunch or after
    /// a reconnect. Matches by terminal ID first, then by pane and tab ID:
    /// herdr restores workspaces, tabs and pane IDs when its server restarts,
    /// but with fresh processes, new terminal IDs and without the tab's
    /// `--env` vars.
    pub async fn reresolve(&self, stored: &TerminalRef) -> Result<Resolved> {
        let panes = self.panes().await?;
        Ok(resolve_in(&panes, stored))
    }

    /// [`Client::reresolve`] for several terminals with one `pane.list`.
    pub async fn reresolve_many(&self, stored: &[TerminalRef]) -> Result<Vec<Resolved>> {
        let panes = self.panes().await?;
        Ok(stored.iter().map(|s| resolve_in(&panes, s)).collect())
    }

    // ---- terminal operations (F20) -----------------------------------------

    /// Type literal text into a pane (no key translation; `\n` is a newline).
    pub async fn send_text(&self, pane: &PaneId, text: &str) -> Result<()> {
        self.call(
            method::PANE_SEND_TEXT,
            json!({ "pane_id": pane, "text": text }),
        )
        .await
        .map(drop)
    }

    /// Press keys in a pane. Key names use herdr's syntax (`Enter`, `Up`,
    /// `C-c`); the engine translates Azulejo key names before calling this.
    pub async fn send_keys<S: AsRef<str>>(&self, pane: &PaneId, keys: &[S]) -> Result<()> {
        let keys: Vec<&str> = keys.iter().map(AsRef::as_ref).collect();
        self.call(
            method::PANE_SEND_KEYS,
            json!({ "pane_id": pane, "keys": keys }),
        )
        .await
        .map(drop)
    }

    /// Read a pane as plain text. This is also how scrollback is read:
    /// terminal tiles have no local scrollback (herdr owns it).
    pub async fn read(
        &self,
        pane: &PaneId,
        source: ReadSource,
        lines: Option<u32>,
    ) -> Result<ReadResult> {
        let r = self
            .call(
                method::PANE_READ,
                json!({ "pane_id": pane, "source": source, "lines": lines, "format": "text" }),
            )
            .await?;
        wire::field(&r, "read")
    }

    /// Scroll a pane's view; 0 is the live bottom.
    pub async fn scroll(&self, pane: &PaneId, offset_from_bottom: u64) -> Result<PaneInfo> {
        let r = self
            .call(
                method::PANE_SCROLL,
                json!({ "pane_id": pane, "offset_from_bottom": offset_from_bottom }),
            )
            .await?;
        wire::field(&r, "pane")
    }

    /// What runs in a pane: its shell and the foreground process group. herdr
    /// sends no event when the foreground process changes; poll this.
    pub async fn process_info(&self, pane: &PaneId) -> Result<ProcessInfo> {
        let r = self
            .call(method::PANE_PROCESS_INFO, json!({ "pane_id": pane }))
            .await?;
        wire::field(&r, "process_info")
    }

    // ---- events ------------------------------------------------------------

    /// Open an event stream. It reconnects by itself; see [`EventStream`].
    pub fn subscribe(&self, subscriptions: Vec<Subscription>) -> EventStream {
        EventStream::spawn(self.clone(), subscriptions)
    }

    // ---- transport -----------------------------------------------------------

    fn next_id(&self) -> String {
        format!("azj-{}", self.inner.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Send one request and return its `result` object.
    pub(crate) async fn call<P: Serialize>(&self, method: &str, params: P) -> Result<Value> {
        let id = self.next_id();
        let line = wire::encode(&id, method, params)?;
        let mut conn = self.connect_with_backoff(self.inner.opts.connect).await?;
        let timeout = self.inner.opts.request_timeout;
        let reply = tokio::time::timeout(timeout, round_trip(&mut conn, &line))
            .await
            .map_err(|_| Error::Timeout {
                method: method.to_owned(),
                timeout,
            })?;
        match reply {
            Ok(Some(reply)) => wire::decode_reply(&reply, &id),
            // A bridge that died says why on stderr (ssh unreachable, no
            // herdr on the machine, …).
            Ok(None) | Err(Error::Io(_)) if conn.guard.is_some() => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Err(conn.bridge_error().expect("bridge"))
            }
            Ok(None) => Err(Error::Closed {
                method: method.to_owned(),
            }),
            Err(e) => Err(e),
        }
    }

    pub(crate) async fn connect_with_backoff(&self, backoff: Backoff) -> Result<Conn> {
        let socket = match &self.inner.transport {
            Transport::Socket(path) => path,
            // A bridge always starts; whether it reaches herdr shows on the
            // first read. Retrying is the caller's (the event stream's) job.
            Transport::Bridge(cmd) => return spawn_bridge(cmd),
        };
        let start = Instant::now();
        let mut delay = backoff.initial;
        loop {
            match UnixStream::connect(socket).await {
                Ok(s) => return Ok(Conn::unix(s)),
                Err(e) if retryable(&e) && start.elapsed() + delay <= backoff.give_up_after => {
                    tokio::time::sleep(delay).await;
                    delay = backoff.next(delay);
                }
                Err(source) => {
                    return Err(Error::Connect {
                        path: socket.clone(),
                        source,
                    });
                }
            }
        }
    }
}

/// What became of `stored`, given every pane herdr has now.
fn resolve_in(panes: &[PaneInfo], stored: &TerminalRef) -> Resolved {
    if let Some(p) = panes.iter().find(|p| p.terminal_id == stored.terminal_id) {
        return if p.pane_id == stored.pane_id && p.tab_id == stored.tab_id {
            Resolved::Same(p.clone())
        } else {
            Resolved::Moved(p.clone())
        };
    }
    if let Some(p) = panes
        .iter()
        .find(|p| p.pane_id == stored.pane_id && p.tab_id == stored.tab_id)
    {
        return Resolved::Respawned(p.clone());
    }
    Resolved::Gone
}

async fn round_trip(conn: &mut Conn, line: &str) -> Result<Option<String>> {
    conn.write.write_all(line.as_bytes()).await?;
    conn.write.flush().await?;
    let mut reader = BufReader::new(&mut conn.read);
    let mut reply = String::new();
    let n = reader.read_line(&mut reply).await?;
    Ok((n > 0).then_some(reply))
}

fn retryable(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(
        e.kind(),
        NotFound | ConnectionRefused | ConnectionReset | ConnectionAborted
    )
}

fn env_map(env: &[(String, String)]) -> serde_json::Map<String, Value> {
    env.iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    /// A socket path in a fresh temp folder, removed when the guard drops.
    fn temp_socket(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("azj-h-{name}-"))
            .tempdir()
            .unwrap();
        let path = dir.path().join("s.sock");
        (dir, path)
    }

    /// A fake herdr that answers `n` requests with `reply(request_line)`.
    fn fake_server(
        path: PathBuf,
        n: usize,
        reply: impl Fn(&Value) -> String + Send + 'static,
    ) -> tokio::task::JoinHandle<Vec<Value>> {
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let mut seen = Vec::new();
            for _ in 0..n {
                let (stream, _) = listener.accept().await.unwrap();
                let (r, mut w) = stream.into_split();
                let mut line = String::new();
                BufReader::new(r).read_line(&mut line).await.unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                w.write_all(reply(&req).as_bytes()).await.unwrap();
                seen.push(req);
            }
            seen
        })
    }

    fn opts() -> ClientOptions {
        ClientOptions {
            request_timeout: Duration::from_millis(500),
            connect: Backoff {
                initial: Duration::from_millis(10),
                max: Duration::from_millis(50),
                give_up_after: Duration::from_secs(2),
            },
        }
    }

    #[tokio::test]
    async fn ping_checks_protocol_version() {
        let (_dir, path) = temp_socket("old");
        let server = fake_server(path.clone(), 1, |req| {
            format!(
                "{{\"id\":{},\"result\":{{\"type\":\"pong\",\"version\":\"0.1.0\",\"protocol\":3}}}}\n",
                req["id"]
            )
        });
        let err = Client::connect(&path, opts()).await.unwrap_err();
        assert!(
            matches!(
                err,
                Error::Version {
                    found: 3,
                    min: 22,
                    ..
                }
            ),
            "{err}"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn correlates_ids_and_types_results() {
        let (_dir, path) = temp_socket("ok");
        let server = fake_server(path.clone(), 2, |req| {
            match req["method"].as_str().unwrap() {
                "ping" => format!(
                    "{{\"id\":{},\"result\":{{\"type\":\"pong\",\"version\":\"0.9.1\",\"protocol\":22}}}}\n",
                    req["id"]
                ),
                _ => format!(
                    "{{\"id\":{},\"result\":{{\"type\":\"tab_created\",\"tab\":{{}},\"root_pane\":{{\"pane_id\":\"w1:p2\",\"terminal_id\":\"term_x\",\"workspace_id\":\"w1\",\"tab_id\":\"w1:t2\"}}}}}}\n",
                    req["id"]
                ),
            }
        });
        let (client, info) = Client::connect(&path, opts()).await.unwrap();
        assert_eq!(info.protocol, 22);
        let t = client
            .create_terminal(&TerminalSpec {
                workspace_id: "w1".into(),
                label: Some("tile".into()),
                cwd: None,
                env: vec![("AZULEJO_TILE_ID".into(), "t1".into())],
            })
            .await
            .unwrap();
        assert_eq!(t.terminal_id.as_str(), "term_x");
        let seen = server.await.unwrap();
        assert_eq!(seen[1]["method"], "tab.create");
        assert_eq!(seen[1]["params"]["env"]["AZULEJO_TILE_ID"], "t1");
        assert_eq!(seen[1]["params"]["label"], "tile");
        assert_ne!(seen[0]["id"], seen[1]["id"]);
    }

    #[tokio::test]
    async fn survey_reads_the_snapshot_and_multi_pane_layouts() {
        // Replies as herdr 0.9.1 sends them (trimmed).
        let (_dir, path) = temp_socket("survey");
        let server = fake_server(path.clone(), 2, |req| {
            let id = &req["id"];
            match req["method"].as_str().unwrap() {
                "session.snapshot" => {
                    format!(
                        r#"{{"id":{id},"result":{{"type":"session_snapshot","snapshot":{{"version":"0.9.1","protocol":22,"focused_workspace_id":"w1","workspaces":[{{"workspace_id":"w1","number":1,"label":"probe","active_tab_id":"w1:t1"}}],"tabs":[{{"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"1","pane_count":3}},{{"tab_id":"w1:t2","workspace_id":"w1","number":2,"pane_count":1}}],"panes":[{{"pane_id":"w1:p1","terminal_id":"term_a","workspace_id":"w1","tab_id":"w1:t1"}},{{"pane_id":"w1:p2","terminal_id":"term_b","workspace_id":"w1","tab_id":"w1:t1"}},{{"pane_id":"w1:p3","terminal_id":"term_c","workspace_id":"w1","tab_id":"w1:t1"}},{{"pane_id":"w1:p4","terminal_id":"term_d","workspace_id":"w1","tab_id":"w1:t2"}}],"layouts":[]}}}}}}"#
                    ) + "\n"
                }
                "layout.export" => {
                    assert_eq!(req["params"]["tab_id"], "w1:t1", "only multi-pane tabs");
                    format!(
                        r#"{{"id":{id},"result":{{"type":"layout_export","layout":{{"workspace_id":"w1","tab_id":"w1:t1","zoomed":false,"focused_pane_id":"w1:p1","root":{{"type":"split","direction":"right","ratio":0.5,"first":{{"type":"pane","pane_id":"w1:p1","cwd":"/Users/x"}},"second":{{"type":"split","direction":"down","ratio":0.3,"first":{{"type":"pane","pane_id":"w1:p2"}},"second":{{"type":"pane","pane_id":"w1:p3"}}}}}}}}}}}}"#
                    ) + "\n"
                }
                other => panic!("survey must only read, not call {other}"),
            }
        });
        let survey = Client::new(&path, opts()).survey().await.unwrap();
        assert_eq!(survey.focused_workspace_id, Some("w1".into()));
        assert_eq!(survey.panes.len(), 4);
        assert_eq!(survey.layouts.len(), 1);
        let layout = &survey.layouts[0];
        let panes: Vec<&str> = layout.root.panes().iter().map(|p| p.as_str()).collect();
        assert_eq!(panes, ["w1:p1", "w1:p2", "w1:p3"]);
        assert!(matches!(
            layout.root,
            crate::LayoutNode::Split {
                direction: crate::SplitDirection::Right,
                ..
            }
        ));
        let seen = server.await.unwrap();
        let methods: Vec<&str> = seen.iter().map(|r| r["method"].as_str().unwrap()).collect();
        assert_eq!(methods, ["session.snapshot", "layout.export"]);
    }

    #[tokio::test]
    async fn not_found_becomes_none() {
        let (_dir, path) = temp_socket("nf");
        let server = fake_server(path.clone(), 1, |req| {
            format!(
                "{{\"id\":{},\"error\":{{\"code\":\"pane_not_found\",\"message\":\"gone\"}}}}\n",
                req["id"]
            )
        });
        let client = Client::new(&path, opts());
        assert_eq!(client.pane(&"w9:p9".into()).await.unwrap(), None);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn waits_for_a_server_that_comes_up_late() {
        let (_dir, path) = temp_socket("late");
        let p2 = path.clone();
        let server = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            fake_server(p2, 1, |req| {
                format!(
                    "{{\"id\":{},\"result\":{{\"type\":\"pong\",\"version\":\"0.9.1\",\"protocol\":22}}}}\n",
                    req["id"]
                )
            })
            .await
            .unwrap()
        });
        let client = Client::new(&path, opts());
        assert_eq!(client.ping().await.unwrap().protocol, 22);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn gives_up_when_nobody_listens() {
        let (_dir, path) = temp_socket("none");
        let client = Client::new(
            &path,
            ClientOptions {
                connect: Backoff {
                    give_up_after: Duration::from_millis(100),
                    ..opts().connect
                },
                ..opts()
            },
        );
        let err = client.ping().await.unwrap_err();
        assert!(err.is_unreachable(), "{err}");
    }

    #[tokio::test]
    async fn times_out_on_a_silent_server() {
        let (_dir, path) = temp_socket("silent");
        let listener = UnixListener::bind(&path).unwrap();
        let hold = tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
            drop(s);
        });
        let err = Client::new(&path, opts()).ping().await.unwrap_err();
        assert!(matches!(err, Error::Timeout { .. }), "{err}");
        hold.abort();
    }
}
