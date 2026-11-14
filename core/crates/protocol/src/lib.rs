//! Commands, events, replies and errors shared by the shells, the API server and
//! the CLI. These types are the API contract: `schema/api.v1.json` is generated
//! from them (`just schema`), and a test fails when the file is stale.
//!
//! Wire shapes:
//! - command: `{"method": "layout.split", "params": {…}}` (`params` may be left
//!   out when every parameter is optional)
//! - event: `{"event": "layout.changed", "data": {…}}`
//! - reply: `{"type": "created", …}`
//! - error: `{"code": "not_found", "message": "…"}`

mod command;
mod error;
mod event;
mod files;
mod machines;
mod project;
mod reply;
mod schema;
mod stream;

pub use azulejo_model::{
    Axis, Direction, Focus, Frame, HerdrRef, InsertAt, LayoutKind, LayoutParams, LayoutPrefs, Node,
    OnExit, Orientation, Placement, Rect, Session, TabId, ThreeColumnParams, Tile, TileContent,
    TileId, TileKind, TileParams, WindowId, WorkspaceId,
};
pub use command::*;
pub use error::{Error, ErrorCode};
pub use event::*;
pub use files::*;
pub use machines::*;
pub use project::*;
pub use reply::*;
pub use schema::{api_schema, api_schema_text};
pub use stream::*;

/// Version of the public API schema. Breaking changes bump it.
pub const API_VERSION: u32 = 1;
