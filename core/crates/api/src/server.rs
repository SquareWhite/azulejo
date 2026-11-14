//! The Unix socket listener.
//!
//! Security: the socket's directory is created with mode 0700 and the socket
//! itself is chmodded to 0600 right after `bind`. The short window in between
//! is harmless because nobody else can enter the directory. An existing
//! default directory owned by another user is refused.

use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream as StdUnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};

use crate::events::{Connection, DEFAULT_EVENT_BUFFER, Hub};
use crate::rpc::handle_line;
use crate::{Dispatcher, default_socket_path};

/// Lines waiting to be written to one connection. A reader that stops reading
/// fills this, then its subscriptions' buffers, then starts lagging.
const OUT_QUEUE: usize = 64;

/// Longest request line accepted, in bytes. Longer lines close the connection.
const MAX_LINE: usize = 8 * 1024 * 1024;

/// A bound, not yet serving, socket. Dropping it removes the socket file.
#[derive(Debug)]
pub struct Server {
    listener: StdUnixListener,
    guard: SocketGuard,
    event_buffer: usize,
}

/// Removes the socket file on drop, unless something else has replaced it.
#[derive(Debug)]
struct SocketGuard {
    path: PathBuf,
    dev: u64,
    ino: u64,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Ok(meta) = fs::symlink_metadata(&self.path)
            && meta.dev() == self.dev
            && meta.ino() == self.ino
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl Server {
    /// Binds the socket at `path`, creating its directory if needed.
    ///
    /// Fails with `AddrInUse` if another server is already listening there. A
    /// stale socket left by a crashed server is replaced. A non-socket file at
    /// `path` is never removed.
    pub fn bind(path: impl Into<PathBuf>) -> io::Result<Server> {
        let path = path.into();
        check_path_len(&path)?;
        let dir = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        prepare_dir(dir, path == default_socket_path())?;
        clear_stale(&path)?;
        let listener = StdUnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let meta = fs::symlink_metadata(&path)?;
        Ok(Server {
            listener,
            guard: SocketGuard {
                path,
                dev: meta.dev(),
                ino: meta.ino(),
            },
            event_buffer: DEFAULT_EVENT_BUFFER,
        })
    }

    /// How many events each subscription may hold before it lags
    /// ([`DEFAULT_EVENT_BUFFER`] unless set).
    pub fn event_buffer(mut self, events: usize) -> Server {
        self.event_buffer = events.max(1);
        self
    }

    pub fn path(&self) -> &Path {
        &self.guard.path
    }

    /// Serves until `shutdown` resolves. Must run inside a Tokio runtime.
    pub async fn serve_until(
        self,
        dispatcher: Arc<dyn Dispatcher>,
        shutdown: impl std::future::Future<Output = ()>,
    ) -> io::Result<()> {
        let hub = self.hub_for(dispatcher.as_ref());
        self.serve(dispatcher, hub, shutdown).await
    }

    /// The event hub for `dispatcher`, already fed, if it has events.
    fn hub_for(&self, dispatcher: &dyn Dispatcher) -> Option<Hub> {
        dispatcher.events().map(|events| {
            let hub = Hub::new(self.event_buffer);
            hub.pump(events);
            hub
        })
    }

    async fn serve(
        self,
        dispatcher: Arc<dyn Dispatcher>,
        hub: Option<Hub>,
        shutdown: impl std::future::Future<Output = ()>,
    ) -> io::Result<()> {
        let Server {
            listener, guard, ..
        } = self;
        listener.set_nonblocking(true)?;
        let listener = UnixListener::from_std(listener)?;
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        tokio::spawn(serve_connection(stream, dispatcher.clone(), hub.clone()));
                    }
                    // A failed accept (e.g. out of file descriptors) shouldn't
                    // take the server down.
                    Err(_) => tokio::task::yield_now().await,
                },
            }
        }
        if let Some(hub) = hub {
            hub.close();
        }
        drop(guard);
        Ok(())
    }

    /// Serves on a background thread with its own Tokio runtime. The server
    /// stops, and the socket file goes, when the handle is dropped.
    pub fn spawn(self, dispatcher: Arc<dyn Dispatcher>) -> io::Result<ServerHandle> {
        let path = self.path().to_owned();
        let hub = self.hub_for(dispatcher.as_ref());
        let (stop, stopped) = oneshot::channel::<()>();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let serve_hub = hub.clone();
        let thread = std::thread::Builder::new()
            .name("azulejo-api".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let _ = self
                        .serve(dispatcher, serve_hub, async {
                            let _ = stopped.await;
                        })
                        .await;
                });
            })?;
        Ok(ServerHandle {
            path,
            hub,
            stop: Some(stop),
            thread: Some(thread),
        })
    }
}

/// A server running on its own thread. Dropping it stops the server.
#[derive(Debug)]
pub struct ServerHandle {
    path: PathBuf,
    hub: Option<Hub>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl ServerHandle {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Live event subscriptions across all connections.
    pub fn subscriptions(&self) -> usize {
        self.hub.as_ref().map_or(0, Hub::len)
    }

    /// Stops the server and waits for it to finish.
    pub fn shutdown(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

fn prepare_dir(dir: &Path, is_default: bool) -> io::Result<()> {
    match fs::symlink_metadata(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir),
        Err(e) => Err(e),
        Ok(meta) if !meta.is_dir() => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} exists and isn't a directory", dir.display()),
        )),
        // An override path may live in a shared directory such as /tmp; only
        // our own default directory is held to ownership and mode rules.
        Ok(_) if !is_default => Ok(()),
        Ok(meta) => {
            // SAFETY: getuid has no preconditions and can't fail.
            let uid = unsafe { libc::getuid() };
            if meta.uid() != uid {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{} is owned by another user", dir.display()),
                ));
            }
            if meta.mode() & 0o077 != 0 {
                fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        }
    }
}

fn clear_stale(path: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
        Ok(meta) => meta,
    };
    if !meta.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} exists and isn't a socket", path.display()),
        ));
    }
    match StdUnixStream::connect(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("another Azulejo is already listening on {}", path.display()),
        )),
        Err(_) => match fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

async fn serve_connection(stream: UnixStream, dispatcher: Arc<dyn Dispatcher>, hub: Option<Hub>) {
    let (read, mut write) = stream.into_split();
    // One writer per connection: responses and event notifications share it,
    // each line written whole.
    let (out, mut lines) = mpsc::channel::<Vec<u8>>(OUT_QUEUE);
    let writer = tokio::spawn(async move {
        while let Some(line) = lines.recv().await {
            if write.write_all(&line).await.is_err() {
                return;
            }
        }
    });
    let mut conn = Connection::new(hub, out.clone());
    let mut reader = BufReader::new(read);
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = (&mut reader)
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut line)
            .await;
        match read {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.len() > MAX_LINE {
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {
                    "code": -32600,
                    "message": format!("request longer than {MAX_LINE} bytes"),
                    "data": {"code": "invalid_request"},
                },
            });
            let _ = out.send(format!("{response}\n").into_bytes()).await;
            break;
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(response) = handle_line(text, dispatcher.as_ref(), Some(&mut conn)).await {
            let mut response = response.into_bytes();
            response.push(b'\n');
            if out.send(response).await.is_err() {
                break;
            }
        }
        conn.start_pending();
    }
    // Closing the read side ends the connection and its subscriptions.
    drop(conn);
    drop(out);
    let _ = writer.await;
}

/// The longest socket path the OS takes: `sun_path` less its NUL (103 bytes
/// on macOS, 107 on Linux).
pub fn max_socket_path_len() -> usize {
    // SAFETY: sockaddr_un is plain old data; all-zero is a valid value.
    let addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    std::mem::size_of_val(&addr.sun_path) - 1
}

/// Refuses a path the OS would reject, naming the limit, rather than
/// failing later with a bare "invalid argument".
fn check_path_len(path: &Path) -> io::Result<()> {
    let len = path.as_os_str().len();
    let max = max_socket_path_len();
    if len > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "socket path is {len} bytes, over the OS limit of {max}: {} \
                 (use a shorter AZULEJO_SOCKET or state directory)",
                path.display()
            ),
        ));
    }
    Ok(())
}
