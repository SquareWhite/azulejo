//! Object model: window → workspace → tab → layout tree → tile.
//!
//! Pure data and operations, no I/O. The engine drives it; the protocol crate
//! reuses its types on the wire.

mod dynamic;
mod error;
mod ids;
mod layout;
mod session;
mod tile;

pub use dynamic::{
    FIT_TOLERANCE, InsertAt, LayoutKind, LayoutParams, Orientation, Placement, TabLayout,
    ThreeColumnParams, TileParams, arrange, fit, freeze, neighbour_among,
};
pub use error::ModelError;
pub use ids::{ParseIdError, TabId, TileId, WindowId, WorkspaceId};
pub use layout::{Axis, Direction, MAX_RATIO, MIN_RATIO, Node, Rect, Tall, parse_path, path_text};
pub use session::{
    ClosedTile, ClosedWorkspace, Focus, Frame, GitInfo, LayoutPrefs, Session, SidebarState, Tab,
    TilePath, Window, Workspace,
};
pub use tile::{
    AgentInfo, AgentStatus, HerdrRef, OnExit, PaneMeta, PreviewMode, Tile, TileContent, TileHeader,
    TileKind, tilde,
};
