//! The public API: JSON-RPC 2.0 over a Unix socket, one request per line.
//!
//! - [`Server`] listens on the socket and hands each command to a
//!   [`Dispatcher`]. The usual dispatcher is a [`CommandSender`]: the thread
//!   that owns the [`Engine`](azulejo_engine::Engine) pulls [`Request`]s off the
//!   matching [`CommandReceiver`] and runs them, so the engine never needs a
//!   lock.
//! - [`Client`] is a small blocking client, used by `azj` and the tests.
//!
//! Wire format, one JSON value per line in each direction:
//!
//! ```text
//! → {"jsonrpc":"2.0","id":1,"method":"layout.split","params":{"direction":"right"}}
//! ← {"jsonrpc":"2.0","id":1,"result":{"type":"created","tile":"tile_01…"}}
//! ← {"jsonrpc":"2.0","id":1,"error":{"code":-32001,"message":"…","data":{"code":"not_found"}}}
//! ```
//!
//! `result` is the protocol's `Reply`; `error.data.code` is its `ErrorCode`.
//! Requests without an `id` are notifications and get no response. Batches
//! (a JSON array of requests) are accepted.

mod client;
mod dispatch;
mod events;
mod rpc;
mod server;

use std::path::PathBuf;

pub use client::{Client, ClientError, RpcError};
pub use dispatch::{
    CommandReceiver, CommandSender, CoreDispatcher, DispatchFuture, Dispatcher, EventFeed, Request,
    WithEvents, WithInfo, command_channel, spawn_engine,
};
pub use events::DEFAULT_EVENT_BUFFER;
pub use rpc::{SERVER_METHODS, handle_text};
pub use server::{Server, ServerHandle, max_socket_path_len};

/// Overrides the socket path, for the server and for clients.
pub const SOCKET_ENV: &str = "AZULEJO_SOCKET";

/// Directory that holds the Azulejo socket for this platform:
/// `$TMPDIR/azulejo` on macOS, `$XDG_RUNTIME_DIR/azulejo` on Linux.
pub fn socket_dir() -> PathBuf {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("TMPDIR")
    } else {
        std::env::var_os("XDG_RUNTIME_DIR")
    };
    base.filter(|b| !b.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("azulejo")
}

/// The default socket path, ignoring `AZULEJO_SOCKET`.
pub fn default_socket_path() -> PathBuf {
    socket_dir().join("azulejo.sock")
}

/// The socket path: `AZULEJO_SOCKET` if set, else [`default_socket_path`].
pub fn socket_path() -> PathBuf {
    match std::env::var_os(SOCKET_ENV) {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => default_socket_path(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_lives_in_azulejo_dir() {
        assert!(default_socket_path().ends_with("azulejo/azulejo.sock"));
    }
}
