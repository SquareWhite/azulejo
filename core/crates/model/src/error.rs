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

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::WindowNotFound(id) => write!(f, "no window {id}"),
            ModelError::WorkspaceNotFound(id) => write!(f, "no workspace {id}"),
            ModelError::TabNotFound(id) => write!(f, "no tab {id}"),
            ModelError::TileNotFound(id) => write!(f, "no tile {id}"),
            ModelError::NothingFocused => write!(f, "nothing is focused"),
            ModelError::NoNeighbour { tile, direction } => {
                write!(f, "no tile {direction:?} of {tile}")
            }
            ModelError::NoEdge { tile, direction } => {
                write!(f, "{tile} has no divider on its {direction:?} edge")
            }
            ModelError::DifferentTabs { a, b } => write!(f, "{a} and {b} are in different tabs"),
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
