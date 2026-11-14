//! Object model: window → workspace → tab → layout tree → tile.

/// The kind of content a tile shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileKind {
    Terminal,
    Browser,
    Preview,
    Files,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_kinds_are_distinct() {
        assert_ne!(TileKind::Terminal, TileKind::Browser);
    }
}
