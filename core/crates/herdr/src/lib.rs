//! herdr adapter. No other crate may use herdr method names or JSON shapes.
//!
//! Mapping (see `docs/herdr-notes.md`):
//! - one app workspace ↔ one herdr workspace;
//! - one terminal tile ↔ one herdr tab, created with `--label` and the
//!   tile's env vars; the tile stores a [`TerminalRef`];
//! - closing a tile detaches (the shell ends its attach process); killing
//!   is the explicit [`Client::kill_terminal`];
//! - `layout.apply` is never used: it replaces live panes.
//!
//! Terminal tiles run [`Herdr::attach`] as their PTY child; everything else
//! goes through [`Client`].

mod attach;
mod client;
mod env;
mod error;
mod event;
mod server;
mod stream;
mod types;
mod wire;

pub use attach::{AttachCommand, AttachExit};
pub use client::{Backoff, Client, ClientOptions};
pub use env::{clean_env, clean_process_env};
pub use error::{Error, Result};
pub use event::{HerdrEvent, Subscription};
pub use server::{Herdr, Session};
pub use stream::{EventStream, StreamItem};
pub use types::*;

/// Minimum herdr socket protocol version Azulejo supports (herdr 0.9.x).
pub const MIN_PROTOCOL: u32 = 22;
