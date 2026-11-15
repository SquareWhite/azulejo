//! herdr adapter. No other crate may use herdr method names or JSON shapes.
//!
//! Mapping (see `docs/herdr-notes.md`):
//! - one app workspace ↔ one herdr workspace;
//! - one Azulejo tab ↔ one herdr tab, each terminal tile a pane in it, the
//!   tab's split tree mirrored both ways (ADR 0012): the first terminal
//!   makes the tab (`--label`, the tile's env vars), the rest split off the
//!   pane beside them ([`Client::split_terminal`]), and [`Client::mirror`]
//!   makes a tab match a target pane by pane; a herdr tab that already
//!   exists is adopted whole ([`Client::survey`], ADR 0008); every tile
//!   stores a [`TerminalRef`];
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
mod machine;
mod mirror;
mod server;
mod stream;
mod transport;
mod types;
mod wire;

pub use attach::{AttachCommand, AttachExit};
pub use client::{Backoff, Client, ClientOptions};
pub use env::{clean_env, clean_process_env};
pub use error::{Error, Result, needs_attention};
pub use event::{HerdrEvent, Subscription};
pub use machine::{
    MachineProfile, RemoteMachine, SSH_FAILED, Ssh, parse_machine_list, shell_quote,
};
pub use mirror::{
    ECHO_TOLERANCE, LayoutOps, MirrorError, MirrorOutcome, MirrorTarget, RATIO_TOLERANCE, diverged,
    prune_tree, same_layout, same_shape, same_tree, satisfies,
};
pub use server::{Herdr, ServerOrigin, Session};
pub use stream::{EventStream, StreamItem};
pub use transport::{BridgeCommand, Transport};
pub use types::*;

/// Minimum herdr socket protocol version Azulejo supports (herdr 0.9.x).
pub const MIN_PROTOCOL: u32 = 22;
