use std::fmt;

use crate::ids::{TabId, TileId, WindowId, WorkspaceId};
use crate::layout::Direction;

/// Why a model operation was refused. A refused operation never changes state.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelError {
    WindowNotFound(WindowId),
    WorkspaceNotFound(WorkspaceId),
    TabNotFound(TabId),
    TileNotFound(TileId),
    /// There is no focused window, workspace, tab or tile to act on.
    NothingFocused,
    /// No tile lies in that direction.
    NoNeighbour {
        tile: TileId,
        direction: Direction,
    },
    /// The tile has no divider on that edge to move.
    NoEdge {
        tile: TileId,
        direction: Direction,
    },
    /// Both tiles must be in the same tab.
    DifferentTabs {
        a: TileId,
        b: TileId,
    },
    InvalidValue(String),
}

/// "to the right", "above"…
fn towards(direction: Direction) -> &'static str {
    match direction {
        Direction::Left => "to the left",
        Direction::Right => "to the right",
        Direction::Up => "above",
        Direction::Down => "below",
    }
}

/// Words for people: shells show these as they are, so they name no IDs.
/// The IDs go in the API error's data (see the protocol's `Error`).
impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::WindowNotFound(_) => f.write_str("no such window"),
            ModelError::WorkspaceNotFound(_) => f.write_str("no such workspace"),
            ModelError::TabNotFound(_) => f.write_str("no such tab"),
            ModelError::TileNotFound(_) => f.write_str("no such tile"),
            ModelError::NothingFocused => f.write_str("nothing is focused"),
            ModelError::NoNeighbour { direction, .. } => {
                write!(f, "no tile {}", towards(*direction))
            }
            ModelError::NoEdge { direction, .. } => {
                write!(f, "no divider {} to move", towards(*direction))
            }
            ModelError::DifferentTabs { .. } => f.write_str("the tiles are in different tabs"),
            ModelError::InvalidValue(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ModelError {}

impl ModelError {
    /// True when the error names something that doesn't exist.
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            ModelError::WindowNotFound(_)
                | ModelError::WorkspaceNotFound(_)
                | ModelError::TabNotFound(_)
                | ModelError::TileNotFound(_)
        )
    }
}
