//! A tab's layout: a binary tree of horizontal and vertical splits with tiles at
//! the leaves. Geometry is computed in a unit square, so focus-by-direction works
//! the same whatever size the shell draws at.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ModelError;
use crate::ids::TileId;

/// Smallest share a split may give either side.
pub const MIN_RATIO: f64 = 0.05;
/// Largest share a split may give either side.
pub const MAX_RATIO: f64 = 1.0 - MIN_RATIO;

/// How a split lays out its two children: `horizontal` puts them side by side,
/// `vertical` stacks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// A direction on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub const ALL: [Direction; 4] = [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ];

    /// The split axis that separates things in this direction.
    pub fn axis(self) -> Axis {
        match self {
            Direction::Left | Direction::Right => Axis::Horizontal,
            Direction::Up | Direction::Down => Axis::Vertical,
        }
    }

    /// True for right and down: the `b` side of a split.
    pub fn is_forward(self) -> bool {
        matches!(self, Direction::Right | Direction::Down)
    }

    /// The direction pointing the other way.
    pub fn opposite(self) -> Direction {
        match self {
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
        }
    }
}

/// A rectangle in the unit square, origin top left.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const UNIT: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    pub(crate) fn split(&self, axis: Axis, ratio: f64) -> (Rect, Rect) {
        match axis {
            Axis::Horizontal => {
                let w = self.width * ratio;
                (
                    Rect { width: w, ..*self },
                    Rect {
                        x: self.x + w,
                        width: self.width - w,
                        ..*self
                    },
                )
            }
            Axis::Vertical => {
                let h = self.height * ratio;
                (
                    Rect { height: h, ..*self },
                    Rect {
                        y: self.y + h,
                        height: self.height - h,
                        ..*self
                    },
                )
            }
        }
    }
}

/// One node of a layout tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Node {
    Split {
        axis: Axis,
        /// Share of the space given to `a`, in (0, 1).
        ratio: f64,
        a: Box<Node>,
        b: Box<Node>,
    },
    Leaf {
        id: TileId,
    },
}

const EPS: f64 = 1e-9;

impl Node {
    pub fn leaf(id: TileId) -> Node {
        Node::Leaf { id }
    }

    fn is_leaf(&self, tile: TileId) -> bool {
        matches!(self, Node::Leaf { id } if *id == tile)
    }

    /// Tiles in reading order (depth first, `a` before `b`).
    pub fn tiles(&self) -> Vec<TileId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<TileId>) {
        match self {
            Node::Leaf { id } => out.push(*id),
            Node::Split { a, b, .. } => {
                a.collect(out);
                b.collect(out);
            }
        }
    }

    pub fn first_tile(&self) -> TileId {
        match self {
            Node::Leaf { id } => *id,
            Node::Split { a, .. } => a.first_tile(),
        }
    }

    pub fn contains(&self, tile: TileId) -> bool {
        match self {
            Node::Leaf { id } => *id == tile,
            Node::Split { a, b, .. } => a.contains(tile) || b.contains(tile),
        }
    }

    pub fn leaf_count(&self) -> usize {
        match self {
            Node::Leaf { .. } => 1,
            Node::Split { a, b, .. } => a.leaf_count() + b.leaf_count(),
        }
    }

    /// Every split ratio, depth first.
    pub fn ratios(&self) -> Vec<f64> {
        let mut out = Vec::new();
        fn walk(node: &Node, out: &mut Vec<f64>) {
            if let Node::Split { ratio, a, b, .. } = node {
                out.push(*ratio);
                walk(a, out);
                walk(b, out);
            }
        }
        walk(self, &mut out);
        out
    }

    /// Each tile's rectangle inside the unit square.
    pub fn rects(&self) -> Vec<(TileId, Rect)> {
        let mut out = Vec::new();
        self.place(Rect::UNIT, &mut out);
        out
    }

    fn place(&self, rect: Rect, out: &mut Vec<(TileId, Rect)>) {
        match self {
            Node::Leaf { id } => out.push((*id, rect)),
            Node::Split { axis, ratio, a, b } => {
                let (ra, rb) = rect.split(*axis, *ratio);
                a.place(ra, out);
                b.place(rb, out);
            }
        }
    }

    /// Replaces the leaf `target` with a split holding `target` and `new_tile`,
    /// with `new_tile` on the `direction` side.
    pub fn split(
        &mut self,
        target: TileId,
        direction: Direction,
        new_tile: TileId,
    ) -> Result<(), ModelError> {
        self.split_with_ratio(target, direction, new_tile, 0.5)
    }

    /// [`Node::split`] with the new split's `ratio` (its first child's share),
    /// clamped to the allowed range.
    pub fn split_with_ratio(
        &mut self,
        target: TileId,
        direction: Direction,
        new_tile: TileId,
        ratio: f64,
    ) -> Result<(), ModelError> {
        let ratio = if ratio.is_finite() {
            ratio.clamp(MIN_RATIO, MAX_RATIO)
        } else {
            0.5
        };
        let leaf = self
            .find_leaf_mut(target)
            .ok_or(ModelError::TileNotFound(target))?;
        let (a, b) = if direction.is_forward() {
            (Node::leaf(target), Node::leaf(new_tile))
        } else {
            (Node::leaf(new_tile), Node::leaf(target))
        };
        *leaf = Node::Split {
            axis: direction.axis(),
            ratio,
            a: Box::new(a),
            b: Box::new(b),
        };
        Ok(())
    }

    fn find_leaf_mut(&mut self, tile: TileId) -> Option<&mut Node> {
        match self {
            Node::Leaf { id } if *id == tile => Some(self),
            Node::Leaf { .. } => None,
            Node::Split { a, b, .. } => match a.find_leaf_mut(tile) {
                Some(found) => Some(found),
                None => b.find_leaf_mut(tile),
            },
        }
    }

    /// Removes `tile`'s leaf; its sibling takes the parent's place. Returns the
    /// tile that should take focus (first tile of the sibling), or `None` when
    /// `tile` isn't in the tree. The caller handles removing a lone root leaf.
    pub fn remove(&mut self, tile: TileId) -> Option<TileId> {
        let Node::Split { a, b, .. } = self else {
            return None;
        };
        let keep = if a.is_leaf(tile) {
            std::mem::replace(b.as_mut(), Node::leaf(tile))
        } else if b.is_leaf(tile) {
            std::mem::replace(a.as_mut(), Node::leaf(tile))
        } else {
            return a.remove(tile).or_else(|| b.remove(tile));
        };
        let next = keep.first_tile();
        *self = keep;
        Some(next)
    }

    /// Swaps the positions of two tiles.
    pub fn swap(&mut self, x: TileId, y: TileId) -> Result<(), ModelError> {
        for t in [x, y] {
            if !self.contains(t) {
                return Err(ModelError::TileNotFound(t));
            }
        }
        fn walk(node: &mut Node, x: TileId, y: TileId) {
            match node {
                Node::Leaf { id } if *id == x => *id = y,
                Node::Leaf { id } if *id == y => *id = x,
                Node::Leaf { .. } => {}
                Node::Split { a, b, .. } => {
                    walk(a, x, y);
                    walk(b, x, y);
                }
            }
        }
        walk(self, x, y);
        Ok(())
    }

    /// The nearest tile in `direction` from `from`, by geometry. Candidates must
    /// overlap `from` on the other axis; ties go to the largest overlap, then the
    /// closest centre.
    pub fn neighbour(&self, from: TileId, direction: Direction) -> Option<TileId> {
        self.neighbour_recent(from, direction, &[])
    }

    /// [`Node::neighbour`], with ties between equally near tiles going to the
    /// most recently focused (earliest in `recent`) first.
    pub fn neighbour_recent(
        &self,
        from: TileId,
        direction: Direction,
        recent: &[TileId],
    ) -> Option<TileId> {
        crate::dynamic::neighbour_among(&self.rects(), from, direction, recent)
    }

    /// Moves `tile` one step in `direction`, changing the tree as little as
    /// possible:
    ///
    /// - If the neighbour that way spans exactly the same edge (a sibling in the
    ///   same split, or the next tile in a stack chain), the two swap slots and
    ///   the tree keeps its shape. Moving back undoes it exactly.
    /// - Otherwise the tile leaves its slot and splits the neighbour, landing on
    ///   its far side.
    /// - With no neighbour, it moves to that edge of the whole tab.
    ///
    /// Returns false when nothing changed.
    pub fn move_tile(&mut self, tile: TileId, direction: Direction) -> Result<bool, ModelError> {
        if !self.contains(tile) {
            return Err(ModelError::TileNotFound(tile));
        }
        if self.leaf_count() == 1 {
            return Ok(false);
        }
        let before = self.clone();
        let target = self.neighbour(tile, direction);
        if let Some(neighbour) = target
            && self.aligned(tile, neighbour, direction.axis())
        {
            self.swap(tile, neighbour)?;
            return Ok(*self != before);
        }
        self.remove(tile);
        match target {
            Some(neighbour) => self.split(neighbour, direction, tile)?,
            None => {
                let rest = std::mem::replace(self, Node::leaf(tile));
                let (a, b) = if direction.is_forward() {
                    (rest, Node::leaf(tile))
                } else {
                    (Node::leaf(tile), rest)
                };
                *self = Node::Split {
                    axis: direction.axis(),
                    ratio: 0.5,
                    a: Box::new(a),
                    b: Box::new(b),
                };
            }
        }
        Ok(*self != before)
    }

    /// True when `x` and `y` cover exactly the same span across `axis` (same top
    /// and bottom for a horizontal move, same left and right for a vertical one).
    fn aligned(&self, x: TileId, y: TileId, axis: Axis) -> bool {
        let rects = self.rects();
        let rect = |t: TileId| rects.iter().find(|(id, _)| *id == t).map(|(_, r)| *r);
        let (Some(rx), Some(ry)) = (rect(x), rect(y)) else {
            return false;
        };
        let span = |r: Rect| match axis {
            Axis::Horizontal => (r.y, r.bottom()),
            Axis::Vertical => (r.x, r.right()),
        };
        let ((x0, x1), (y0, y1)) = (span(rx), span(ry));
        (x0 - y0).abs() < EPS && (x1 - y1).abs() < EPS
    }

    /// Moves a divider of `tile` in `direction` by `amount` (a share of the
    /// enclosing split; negative moves it the other way), tmux-style:
    ///
    /// - If the tile has a divider on its `direction` edge, that one moves, so
    ///   the tile grows that way.
    /// - Otherwise the divider on the opposite edge moves in `direction`, so the
    ///   tile shrinks (e.g. "resize left" on a tile at the left edge pulls its
    ///   right divider left).
    ///
    /// Uses the nearest split in each case. Errors only when the tile has no
    /// divider on either side of that axis. Ratios are clamped to
    /// [`MIN_RATIO`], [`MAX_RATIO`].
    pub fn resize(
        &mut self,
        tile: TileId,
        direction: Direction,
        amount: f64,
    ) -> Result<(), ModelError> {
        if !amount.is_finite() {
            return Err(ModelError::InvalidValue(format!(
                "resize amount {amount} is not finite"
            )));
        }
        if !self.contains(tile) {
            return Err(ModelError::TileNotFound(tile));
        }
        // None: tile not below this node. Some(false): below, edge not found yet.
        fn walk(node: &mut Node, tile: TileId, direction: Direction, amount: f64) -> Option<bool> {
            match node {
                Node::Leaf { id } => (*id == tile).then_some(false),
                Node::Split { axis, ratio, a, b } => {
                    let (found, in_a) = match walk(a, tile, direction, amount) {
                        Some(done) => (done, true),
                        None => (walk(b, tile, direction, amount)?, false),
                    };
                    if found {
                        return Some(true);
                    }
                    if *axis == direction.axis() && in_a == direction.is_forward() {
                        let delta = if in_a { amount } else { -amount };
                        *ratio = (*ratio + delta).clamp(MIN_RATIO, MAX_RATIO);
                        return Some(true);
                    }
                    Some(false)
                }
            }
        }
        if walk(self, tile, direction, amount) == Some(true) {
            return Ok(());
        }
        // No divider on that edge: move the opposite one the same way. `walk`
        // only changes a ratio when it succeeds, so the first try left no trace.
        match walk(self, tile, direction.opposite(), -amount) {
            Some(true) => Ok(()),
            _ => Err(ModelError::NoEdge { tile, direction }),
        }
    }

    // ----- tall (master-stack) -----
    //
    // kitty's `tall` layout as rules over the split tree, not a layout mode: one
    // master, every other tile in an evenly divided stack beside it, new tiles
    // appended to the end of the stack. Master and stack are read off the
    // geometry, so the ops work on any tree; the canonical shape they build is
    //
    //     Split { axis, a: Leaf(master), b: chain }
    //
    // where `chain` nests along the other axis, one tile per `a`, in stack order.
    // `axis` is horizontal for tall proper (master left) and vertical once
    // rotated (master on top).

    /// This tree read as a tall layout.
    pub fn tall(&self) -> Tall {
        let rects = self.rects();
        // Top-left corner in both orientations; ties on x mean a column, so y
        // breaks them. Rounded so float noise from ratios can't reorder them.
        let key = |r: &Rect| ((r.x * 1e6).round() as i64, (r.y * 1e6).round() as i64);
        let (master, master_rect) = *rects
            .iter()
            .min_by_key(|(_, r)| key(r))
            .expect("a tree has at least one leaf");
        let vertical = master_rect.height >= 1.0 - EPS;
        let mut stack: Vec<(TileId, Rect)> =
            rects.into_iter().filter(|(id, _)| *id != master).collect();
        stack.sort_by_key(|(_, r)| {
            let (x, y) = key(r);
            if vertical { (y, x) } else { (x, y) }
        });
        Tall {
            master,
            stack: stack.into_iter().map(|(id, _)| id).collect(),
            vertical,
        }
    }

    /// Adds `new_tile` the tall way: a lone tile becomes master | stack;
    /// otherwise the last stack tile is split along the stack axis. Then the
    /// stack is equalized.
    pub fn split_auto(&mut self, new_tile: TileId) -> Result<(), ModelError> {
        let tall = self.tall();
        let (target, direction) = match (tall.stack.last(), tall.vertical) {
            (None, true) => (tall.master, Direction::Right),
            (None, false) => (tall.master, Direction::Down),
            (Some(last), true) => (*last, Direction::Down),
            (Some(last), false) => (*last, Direction::Right),
        };
        self.split(target, direction, new_tile)?;
        self.equalize();
        Ok(())
    }

    /// Swaps `tile` with the master (dwm-style zoom). Returns false when `tile`
    /// already is the master.
    pub fn promote(&mut self, tile: TileId) -> Result<bool, ModelError> {
        if !self.contains(tile) {
            return Err(ModelError::TileNotFound(tile));
        }
        let master = self.tall().master;
        if master == tile {
            return Ok(false);
        }
        self.swap(tile, master)?;
        Ok(true)
    }

    /// Gives every stack tile the same share. Only touches the canonical shape;
    /// a hand-branched stack is left alone. Returns whether a ratio changed.
    pub fn equalize(&mut self) -> bool {
        let master = self.tall().master;
        let Node::Split { axis, a, b, .. } = self else {
            return false;
        };
        if !a.is_leaf(master) {
            return false;
        }
        let along = axis.other();
        let Some(n) = b.chain_len(along) else {
            return false;
        };
        // The i'th split of an n-tile chain gives 1/(n-i) to its `a` tile.
        let mut changed = false;
        let mut node = b.as_mut();
        let mut i = 0;
        while let Node::Split { ratio, b, .. } = node {
            let want = 1.0 / (n - i) as f64;
            if (*ratio - want).abs() > EPS {
                *ratio = want;
                changed = true;
            }
            node = b.as_mut();
            i += 1;
        }
        changed
    }

    /// Turns the layout on its side: master left ↔ master on top, keeping the
    /// stack's order and the master's share. Rebuilds the tree in canonical
    /// shape, so a hand-branched stack becomes an even one. Returns false for a
    /// lone tile.
    pub fn rotate(&mut self) -> bool {
        let tall = self.tall();
        if tall.stack.is_empty() {
            return false;
        }
        let ratio = match self {
            Node::Split { ratio, a, .. } if a.is_leaf(tall.master) => *ratio,
            _ => 0.5,
        };
        let axis = if tall.vertical {
            Axis::Vertical
        } else {
            Axis::Horizontal
        };
        *self = Node::Split {
            axis,
            ratio,
            a: Box::new(Node::leaf(tall.master)),
            b: Box::new(Node::chain(&tall.stack, axis.other())),
        };
        true
    }

    /// Tiles in a chain along `axis`, each split giving its `a` tile an equal
    /// share of what's left.
    fn chain(tiles: &[TileId], axis: Axis) -> Node {
        match tiles {
            [] => unreachable!("chain of no tiles"),
            [only] => Node::leaf(*only),
            [first, rest @ ..] => Node::Split {
                axis,
                ratio: 1.0 / tiles.len() as f64,
                a: Box::new(Node::leaf(*first)),
                b: Box::new(Node::chain(rest, axis)),
            },
        }
    }

    /// The tiles of the chain along `axis` rooted here, in order, or `None`
    /// if it branches.
    pub(crate) fn chain_tiles(&self, along: Axis) -> Option<Vec<TileId>> {
        match self {
            Node::Leaf { id } => Some(vec![*id]),
            Node::Split { axis, a, b, .. } if *axis == along => match a.as_ref() {
                Node::Leaf { id } => {
                    let mut rest = b.chain_tiles(along)?;
                    rest.insert(0, *id);
                    Some(rest)
                }
                _ => None,
            },
            Node::Split { .. } => None,
        }
    }

    /// Length of the chain along `axis` rooted here, or `None` if it branches.
    fn chain_len(&self, along: Axis) -> Option<usize> {
        match self {
            Node::Leaf { .. } => Some(1),
            Node::Split { axis, a, b, .. }
                if *axis == along && matches!(a.as_ref(), Node::Leaf { .. }) =>
            {
                b.chain_len(along).map(|n| n + 1)
            }
            Node::Split { .. } => None,
        }
    }
}

impl Axis {
    pub fn other(self) -> Axis {
        match self {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        }
    }
}

/// A tree read as kitty's `tall` layout; see [`Node::tall`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tall {
    /// The tile in the top-left corner.
    pub master: TileId,
    /// Every other tile, in order along the stack.
    pub stack: Vec<TileId>,
    /// True for master on the left (tall proper), false when rotated. A lone
    /// tile counts as vertical.
    pub vertical: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids<const N: usize>() -> [TileId; N] {
        std::array::from_fn(|_| TileId::new())
    }

    /// [a | (b / c)]
    fn three() -> (Node, [TileId; 3]) {
        let [a, b, c] = ids();
        let mut root = Node::leaf(a);
        root.split(a, Direction::Right, b).unwrap();
        root.split(b, Direction::Down, c).unwrap();
        (root, [a, b, c])
    }

    #[test]
    fn split_places_the_new_tile_on_the_requested_side() {
        let [a, b] = ids();
        let mut root = Node::leaf(a);
        root.split(a, Direction::Left, b).unwrap();
        assert_eq!(root.tiles(), vec![b, a]);
        assert!(matches!(
            root,
            Node::Split {
                axis: Axis::Horizontal,
                ..
            }
        ));
    }

    #[test]
    fn neighbours_follow_geometry() {
        let (root, [a, b, c]) = three();
        assert_eq!(root.neighbour(a, Direction::Right), Some(b));
        assert_eq!(root.neighbour(b, Direction::Down), Some(c));
        assert_eq!(root.neighbour(c, Direction::Up), Some(b));
        assert_eq!(root.neighbour(c, Direction::Left), Some(a));
        assert_eq!(root.neighbour(a, Direction::Left), None);
        assert_eq!(root.neighbour(a, Direction::Up), None);
    }

    #[test]
    fn remove_promotes_the_sibling() {
        let (mut root, [a, b, c]) = three();
        assert_eq!(root.remove(b), Some(c));
        assert_eq!(root.tiles(), vec![a, c]);
        assert_eq!(root.remove(a), Some(c));
        assert_eq!(root, Node::leaf(c));
        assert_eq!(root.remove(c), None);
    }

    #[test]
    fn move_into_a_neighbour_and_to_an_edge() {
        let (mut root, [a, b, c]) = three();
        assert!(root.move_tile(a, Direction::Right).unwrap());
        // a left its slot and split b, landing on b's right.
        assert_eq!(root.neighbour(b, Direction::Right), Some(a));
        assert_eq!(root.neighbour(a, Direction::Down), Some(c));
        // c already spans the bottom edge alone, so moving it down changes nothing.
        assert!(!root.move_tile(c, Direction::Down).unwrap());
        // b has nothing above it, so it moves to the top edge of the tab.
        assert!(root.move_tile(b, Direction::Up).unwrap());
        assert_eq!(root.neighbour(a, Direction::Up), Some(b));
        assert_eq!(root.leaf_count(), 3);
    }

    #[test]
    fn resize_moves_the_shared_divider() {
        let (mut root, [a, b, _]) = three();
        root.resize(a, Direction::Right, 0.1).unwrap();
        assert!((root.ratios()[0] - 0.6).abs() < 1e-9);
        root.resize(b, Direction::Left, 0.2).unwrap();
        assert!((root.ratios()[0] - 0.4).abs() < 1e-9);
        root.resize(a, Direction::Right, 5.0).unwrap();
        assert_eq!(root.ratios()[0], MAX_RATIO);
    }

    #[test]
    fn resize_without_an_edge_moves_the_opposite_divider() {
        let (mut root, [a, b, c]) = three();
        // a touches the tab's left edge: "resize left" pulls its right divider left.
        root.resize(a, Direction::Left, 0.1).unwrap();
        assert!((root.ratios()[0] - 0.4).abs() < 1e-9);
        // b sits at the top: "resize up" pulls its bottom divider up.
        root.resize(b, Direction::Up, 0.2).unwrap();
        assert!((root.ratios()[1] - 0.3).abs() < 1e-9);
        // c has a divider above, so "resize up" grows it.
        root.resize(c, Direction::Up, 0.1).unwrap();
        assert!((root.ratios()[1] - 0.2).abs() < 1e-9);
        // A lone tile has nothing to move.
        let mut lone = Node::leaf(a);
        assert!(matches!(
            lone.resize(a, Direction::Left, 0.1),
            Err(ModelError::NoEdge { .. })
        ));
        // Neither does a tile with no divider on that axis at all.
        let mut row = Node::leaf(a);
        row.split(a, Direction::Right, b).unwrap();
        assert!(matches!(
            row.resize(a, Direction::Up, 0.1),
            Err(ModelError::NoEdge { .. })
        ));
    }

    #[test]
    fn moving_into_an_aligned_neighbour_swaps_and_reverses() {
        // A sibling pair: move right then left restores the tree.
        let [a, b] = ids();
        let mut pair = Node::leaf(a);
        pair.split(a, Direction::Right, b).unwrap();
        pair.resize(a, Direction::Right, 0.2).unwrap();
        let before = pair.clone();
        assert!(pair.move_tile(a, Direction::Right).unwrap());
        assert_eq!(pair.tiles(), vec![b, a]);
        assert!(pair.move_tile(a, Direction::Left).unwrap());
        assert_eq!(pair, before);

        // Inside a tall stack the chain keeps its shape.
        let [m, s1, s2, s3] = ids();
        let mut tall = Node::leaf(m);
        for t in [s1, s2, s3] {
            tall.split_auto(t).unwrap();
        }
        let before = tall.clone();
        assert!(tall.move_tile(s1, Direction::Down).unwrap());
        assert_eq!(tall.tall().stack, vec![s2, s1, s3]);
        assert_eq!(tall.ratios(), before.ratios());
        assert!(tall.move_tile(s1, Direction::Up).unwrap());
        assert_eq!(tall, before);
    }

    #[test]
    fn swap_exchanges_positions() {
        let (mut root, [a, _, c]) = three();
        root.swap(a, c).unwrap();
        assert_eq!(root.tiles()[0], c);
        assert_eq!(root.tiles()[2], a);
    }

    fn heights(root: &Node, tiles: &[TileId]) -> Vec<f64> {
        let rects = root.rects();
        tiles
            .iter()
            .map(|t| rects.iter().find(|(id, _)| id == t).unwrap().1.height)
            .collect()
    }

    #[test]
    fn split_auto_builds_master_and_an_even_stack() {
        let [m, s1, s2, s3, s4] = ids();
        let mut root = Node::leaf(m);
        for t in [s1, s2, s3, s4] {
            root.split_auto(t).unwrap();
        }
        let tall = root.tall();
        assert_eq!(tall.master, m);
        assert_eq!(tall.stack, vec![s1, s2, s3, s4]);
        assert!(tall.vertical);
        for h in heights(&root, &[s1, s2, s3, s4]) {
            assert!((h - 0.25).abs() < 1e-9, "stack height {h}");
        }
        assert!(!root.equalize(), "already even");
    }

    #[test]
    fn split_auto_ignores_focus_and_appends() {
        let [m, s1, s2, s3] = ids();
        let mut root = Node::leaf(m);
        root.split_auto(s1).unwrap();
        root.split_auto(s2).unwrap();
        // Promoting doesn't change where the next tile lands: end of the stack.
        root.promote(s2).unwrap();
        root.split_auto(s3).unwrap();
        let tall = root.tall();
        assert_eq!(tall.master, s2);
        assert_eq!(tall.stack, vec![s1, m, s3]);
    }

    #[test]
    fn promote_swaps_with_the_master() {
        let [m, s1, s2] = ids();
        let mut root = Node::leaf(m);
        root.split_auto(s1).unwrap();
        root.split_auto(s2).unwrap();
        assert!(!root.promote(m).unwrap());
        assert!(root.promote(s2).unwrap());
        assert_eq!(root.tall().master, s2);
        assert_eq!(root.tall().stack, vec![s1, m]);
    }

    #[test]
    fn rotate_toggles_and_keeps_order_and_master_share() {
        let [m, s1, s2] = ids();
        let mut root = Node::leaf(m);
        assert!(!root.rotate(), "a lone tile has nothing to rotate");
        root.split_auto(s1).unwrap();
        root.split_auto(s2).unwrap();
        root.resize(m, Direction::Right, 0.1).unwrap();
        let original = root.clone();

        assert!(root.rotate());
        let tall = root.tall();
        assert!(!tall.vertical);
        assert_eq!(tall.master, m);
        assert_eq!(tall.stack, vec![s1, s2]);
        assert!((root.ratios()[0] - 0.6).abs() < 1e-9);

        assert!(root.rotate());
        assert_eq!(root, original);
    }

    #[test]
    fn equalize_leaves_a_hand_branched_stack_alone() {
        let [m, s1, s2, x] = ids();
        let mut root = Node::leaf(m);
        root.split_auto(s1).unwrap();
        root.split_auto(s2).unwrap();
        // Branch the stack by hand: s1 gets a tile beside it.
        root.split(s1, Direction::Right, x).unwrap();
        root.resize(s2, Direction::Up, 0.2).unwrap();
        let before = root.clone();
        assert!(!root.equalize());
        assert_eq!(root, before);
        // split_auto still works and still appends at the end of the stack.
        let [y] = ids();
        root.split_auto(y).unwrap();
        assert_eq!(root.tall().stack.last(), Some(&y));
    }

    #[test]
    fn tree_json_is_tagged() {
        let [a] = ids();
        let json = serde_json::to_string(&Node::leaf(a)).unwrap();
        assert_eq!(json, format!(r#"{{"type":"leaf","id":"{a}"}}"#));
    }
}
