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

    fn split(&self, axis: Axis, ratio: f64) -> (Rect, Rect) {
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
            ratio: 0.5,
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
        let rects = self.rects();
        let cur = rects.iter().find(|(id, _)| *id == from)?.1;
        rects
            .iter()
            .filter(|(id, _)| *id != from)
            .filter_map(|(id, r)| {
                let (distance, overlap, centre_offset) = match direction {
                    Direction::Right if r.x >= cur.right() - EPS => (
                        r.x - cur.right(),
                        overlap(cur.y, cur.bottom(), r.y, r.bottom()),
                        ((r.y + r.bottom()) - (cur.y + cur.bottom())).abs(),
                    ),
                    Direction::Left if r.right() <= cur.x + EPS => (
                        cur.x - r.right(),
                        overlap(cur.y, cur.bottom(), r.y, r.bottom()),
                        ((r.y + r.bottom()) - (cur.y + cur.bottom())).abs(),
                    ),
                    Direction::Down if r.y >= cur.bottom() - EPS => (
                        r.y - cur.bottom(),
                        overlap(cur.x, cur.right(), r.x, r.right()),
                        ((r.x + r.right()) - (cur.x + cur.right())).abs(),
                    ),
                    Direction::Up if r.bottom() <= cur.y + EPS => (
                        cur.y - r.bottom(),
                        overlap(cur.x, cur.right(), r.x, r.right()),
                        ((r.x + r.right()) - (cur.x + cur.right())).abs(),
                    ),
                    _ => return None,
                };
                (overlap > EPS).then_some((*id, distance, overlap, centre_offset))
            })
            .min_by(|p, q| {
                p.1.total_cmp(&q.1)
                    .then(q.2.total_cmp(&p.2))
                    .then(p.3.total_cmp(&q.3))
            })
            .map(|(id, ..)| id)
    }

    /// Moves `tile` one step in `direction`. If there's a neighbour that way, the
    /// tile leaves its slot and splits the neighbour, landing on its far side.
    /// Otherwise it moves to that edge of the whole tab. Returns false when
    /// nothing changed.
    pub fn move_tile(&mut self, tile: TileId, direction: Direction) -> Result<bool, ModelError> {
        if !self.contains(tile) {
            return Err(ModelError::TileNotFound(tile));
        }
        if self.leaf_count() == 1 {
            return Ok(false);
        }
        let before = self.clone();
        let target = self.neighbour(tile, direction);
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

    /// Moves `tile`'s edge in `direction` by `amount` (a share of the enclosing
    /// split, negative to shrink). Uses the nearest split whose divider is on that
    /// edge. Ratios are clamped to [`MIN_RATIO`], [`MAX_RATIO`].
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
        match walk(self, tile, direction, amount) {
            Some(true) => Ok(()),
            _ => Err(ModelError::NoEdge { tile, direction }),
        }
    }
}

fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
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
        assert!(matches!(
            root.resize(a, Direction::Left, 0.1),
            Err(ModelError::NoEdge { .. })
        ));
    }

    #[test]
    fn swap_exchanges_positions() {
        let (mut root, [a, _, c]) = three();
        root.swap(a, c).unwrap();
        assert_eq!(root.tiles()[0], c);
        assert_eq!(root.tiles()[2], a);
    }

    #[test]
    fn tree_json_is_tagged() {
        let [a] = ids();
        let json = serde_json::to_string(&Node::leaf(a)).unwrap();
        assert_eq!(json, format!(r#"{{"type":"leaf","id":"{a}"}}"#));
    }
}
