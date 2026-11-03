//! Object model: window → workspace → tab → layout tree → tile.
//!
//! Pure data and operations, no I/O. The engine drives it; the protocol crate
//! reuses its types on the wire.

mod error;
mod ids;
mod layout;
mod session;
mod tile;

pub use error::ModelError;
pub use ids::{ParseIdError, TabId, TileId, WindowId, WorkspaceId};
pub use layout::{Axis, Direction, MAX_RATIO, MIN_RATIO, Node, Rect};
pub use session::{
    ClosedTile, ClosedWorkspace, Focus, Frame, Session, Tab, TilePath, Window, Workspace,
};
pub use tile::{HerdrRef, Tile, TileContent, TileKind};
