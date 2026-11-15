//! Dynamic layouts (ADR 0010): an ordered list of tiles and a rule that turns
//! it into rectangles, ported from Bismuth. A tab is either `manual` (its split
//! tree is the source of truth) or one of the dynamic kinds, whose `order`,
//! `weights` and `params` are; the tree is then derived after every change.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::TileId;
use crate::layout::{Axis, Direction, Node, Rect};

/// Tolerance for comparing edges computed from weighted splits.
pub(crate) const GEO_EPS: f64 = 1e-7;

/// How a tab arranges its tiles.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayoutKind {
    /// A split tree edited by hand; the tree is the source of truth.
    #[default]
    Manual,
    /// Masters on one side, a stack on the other.
    Tile,
    /// Every tile full size, the focused one on top.
    Monocle,
    /// Masters centred, stacks left and right.
    ThreeColumn,
}

impl LayoutKind {
    pub const DYNAMIC: [LayoutKind; 3] = [
        LayoutKind::Tile,
        LayoutKind::Monocle,
        LayoutKind::ThreeColumn,
    ];

    pub fn is_dynamic(self) -> bool {
        self != LayoutKind::Manual
    }

    /// The name used in config, layout files and the protocol.
    pub fn name(self) -> &'static str {
        match self {
            LayoutKind::Manual => "manual",
            LayoutKind::Tile => "tile",
            LayoutKind::Monocle => "monocle",
            LayoutKind::ThreeColumn => "three_column",
        }
    }

    /// Parses a name; `tall` is accepted as an alias for `tile`.
    pub fn parse(name: &str) -> Option<LayoutKind> {
        match name {
            "manual" => Some(LayoutKind::Manual),
            "tile" | "tall" => Some(LayoutKind::Tile),
            "monocle" => Some(LayoutKind::Monocle),
            "three_column" | "three-column" | "threecolumn" => Some(LayoutKind::ThreeColumn),
            _ => None,
        }
    }
}

/// Which side the Tile layout's masters sit on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    Left,
    Top,
    Right,
    Bottom,
}

impl Orientation {
    /// The axis that separates masters from the stack.
    fn main_axis(self) -> Axis {
        match self {
            Orientation::Left | Orientation::Right => Axis::Horizontal,
            Orientation::Top | Orientation::Bottom => Axis::Vertical,
        }
    }

    /// True when the masters take the first (left or top) part.
    fn masters_first(self) -> bool {
        matches!(self, Orientation::Left | Orientation::Top)
    }

    /// One quarter turn: clockwise (left → top → right → bottom) or back.
    /// Bismuth's Rotate (`Meta+R`, Azulejo's `alt+o`) and Rotate Reverse.
    pub fn turned(self, reverse: bool) -> Orientation {
        const CW: [Orientation; 4] = [
            Orientation::Left,
            Orientation::Top,
            Orientation::Right,
            Orientation::Bottom,
        ];
        let i = CW.iter().position(|o| *o == self).expect("listed");
        CW[if reverse { (i + 3) % 4 } else { (i + 1) % 4 }]
    }

    /// Left ↔ top, right ↔ bottom: `tall.py`'s rotate, now the
    /// `flip_layout` action.
    pub fn flipped(self) -> Orientation {
        match self {
            Orientation::Left => Orientation::Top,
            Orientation::Top => Orientation::Left,
            Orientation::Right => Orientation::Bottom,
            Orientation::Bottom => Orientation::Right,
        }
    }
}

/// Where a new tile goes in a dynamic tab.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InsertAt {
    /// The end of the order: the end of the stack (Bismuth, `tall.py`).
    #[default]
    End,
    /// The front of the order: the new tile becomes master.
    Master,
    /// Right after the focused tile.
    AfterFocused,
}

pub const TILE_MASTERS_MAX: u8 = 10;
pub const TILE_RATIO_MIN: f64 = 0.2;
pub const TILE_RATIO_MAX: f64 = 0.8;
pub const THREE_COLUMN_RATIO_MIN: f64 = 0.2;
pub const THREE_COLUMN_RATIO_MAX: f64 = 0.75;
/// Smallest share of its group a tile may be resized down to.
pub const MIN_TILE_SHARE: f64 = 0.05;

/// The Tile layout's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TileParams {
    /// How many tiles sit in the master area: 0–10.
    pub masters: u8,
    /// The master area's share of the tab: 0.2–0.8.
    pub ratio: f64,
    #[serde(default)]
    pub orientation: Orientation,
    /// Bismuth's "rotate part": masters split along the main axis instead of
    /// across it, so with the master area on the left several masters sit
    /// side by side rather than stacked (and the other way round on top).
    #[serde(default)]
    pub rotate_part: bool,
}

impl Default for TileParams {
    fn default() -> Self {
        Self {
            masters: 1,
            ratio: 0.5,
            orientation: Orientation::Left,
            rotate_part: false,
        }
    }
}

/// The Three Column layout's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ThreeColumnParams {
    /// How many tiles share the centre column: 1–10.
    pub masters: u8,
    /// The centre column's weight against each side column's `1 − ratio`,
    /// as in Bismuth: 0.2–0.75. With three columns the centre takes
    /// `ratio / (2 − ratio)` of the width (0.6 gives 3/7); with two (the
    /// masters and one stack tile) it takes `ratio` itself.
    pub ratio: f64,
}

impl Default for ThreeColumnParams {
    fn default() -> Self {
        Self {
            masters: 1,
            ratio: 0.6,
        }
    }
}

/// Every dynamic kind's parameters, remembered across switches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutParams {
    #[serde(default)]
    pub tile: TileParams,
    #[serde(default)]
    pub three_column: ThreeColumnParams,
}

impl LayoutParams {
    /// Brings every parameter into its allowed range; non-finite ratios reset.
    pub fn clamp(&mut self) {
        let t = &mut self.tile;
        t.masters = t.masters.min(TILE_MASTERS_MAX);
        t.ratio = clamp_ratio(t.ratio, TILE_RATIO_MIN, TILE_RATIO_MAX, 0.5);
        let c = &mut self.three_column;
        c.masters = c.masters.clamp(1, TILE_MASTERS_MAX);
        c.ratio = clamp_ratio(c.ratio, THREE_COLUMN_RATIO_MIN, THREE_COLUMN_RATIO_MAX, 0.6);
    }

    fn is_clamped(&self) -> bool {
        let mut c = *self;
        c.clamp();
        c == *self
    }
}

fn clamp_ratio(r: f64, min: f64, max: f64, fallback: f64) -> f64 {
    if r.is_finite() {
        r.clamp(min, max)
    } else {
        fallback
    }
}

/// A tab's arrangement. For `manual` only `previous`, `params` and `kind`
/// matter; `order` and `weights` are empty.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TabLayout {
    pub kind: LayoutKind,
    /// The kind before the last switch, for toggling back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<LayoutKind>,
    /// Tile order, authoritative for dynamic kinds: masters first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<TileId>,
    /// Per-tile size weights within a column; absent means 1.0.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub weights: BTreeMap<TileId, f64>,
    #[serde(default)]
    pub params: LayoutParams,
    /// The last hand-made tree, restored when switching back to manual while
    /// it still holds the same tiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual: Option<Node>,
}

impl TabLayout {
    /// A manual layout that remembers nothing: what old state files load as.
    pub fn is_plain_manual(&self) -> bool {
        *self == TabLayout::default()
    }

    pub fn weight(&self, tile: TileId) -> f64 {
        self.weights.get(&tile).copied().unwrap_or(1.0)
    }

    /// Where every tile goes, for a dynamic kind. `focused` decides Monocle's
    /// top tile.
    pub fn arrange(&self, focused: TileId) -> Vec<Placement> {
        arrange(
            self.kind,
            &self.params,
            &self.order,
            &self.weights,
            Some(focused),
        )
    }

    /// The split tree reproducing this layout's rectangles. Monocle freezes to
    /// the Tile arrangement of the same order.
    pub fn frozen_tree(&self) -> Node {
        let kind = if self.kind == LayoutKind::Monocle {
            LayoutKind::Tile
        } else {
            self.kind
        };
        let rects: Vec<(TileId, Rect)> =
            arrange(kind, &self.params, &self.order, &self.weights, None)
                .into_iter()
                .map(|p| (p.tile, p.rect))
                .collect();
        freeze(&rects).unwrap_or_else(|| chain_of(&self.order))
    }

    /// Checks the dynamic-layout rules against the tab's tile set.
    pub(crate) fn check(&self, tiles: &std::collections::HashSet<TileId>) -> Result<(), String> {
        if !self.params.is_clamped() {
            return Err(format!("layout params out of range: {:?}", self.params));
        }
        if self.previous == Some(self.kind) {
            return Err("layout's previous kind equals its kind".into());
        }
        for w in self.weights.values() {
            if !(w.is_finite() && *w > 0.0) {
                return Err(format!("layout weight {w} is not positive"));
            }
        }
        if !self.kind.is_dynamic() {
            if !self.order.is_empty() || !self.weights.is_empty() {
                return Err("a manual layout keeps an order or weights".into());
            }
            return Ok(());
        }
        let order: std::collections::HashSet<TileId> = self.order.iter().copied().collect();
        if order.len() != self.order.len() {
            return Err("layout order lists a tile twice".into());
        }
        if &order != tiles {
            return Err("layout order isn't a permutation of the tab's tiles".into());
        }
        if let Some(stray) = self.weights.keys().find(|t| !tiles.contains(t)) {
            return Err(format!("layout weight for missing tile {stray}"));
        }
        Ok(())
    }
}

/// Where one tile goes. `rect` is in the unit square, like
/// [`Node::rects`]; higher `z` draws on top; hidden tiles keep their rect but
/// aren't drawn (Monocle's tiles under the top one).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Placement {
    pub tile: TileId,
    pub rect: Rect,
    pub z: u16,
    pub visible: bool,
}

impl Placement {
    fn flat(tile: TileId, rect: Rect) -> Self {
        Self {
            tile,
            rect,
            z: 0,
            visible: true,
        }
    }
}

/// The layout rule: ordered tiles → placements. Pure and deterministic.
/// `Manual` has no rule and returns nothing; use the tree instead.
pub fn arrange(
    kind: LayoutKind,
    params: &LayoutParams,
    order: &[TileId],
    weights: &BTreeMap<TileId, f64>,
    focused: Option<TileId>,
) -> Vec<Placement> {
    let w = |t: &TileId| weights.get(t).copied().unwrap_or(1.0);
    match kind {
        LayoutKind::Manual => Vec::new(),
        LayoutKind::Monocle => {
            let top = focused
                .filter(|f| order.contains(f))
                .or_else(|| order.first().copied());
            order
                .iter()
                .map(|t| Placement {
                    tile: *t,
                    rect: Rect::UNIT,
                    z: u16::from(Some(*t) == top),
                    visible: Some(*t) == top,
                })
                .collect()
        }
        LayoutKind::Tile => tile_groups(&params.tile, order)
            .into_iter()
            .flat_map(|g| {
                let ws: Vec<f64> = g.tiles.iter().map(w).collect();
                g.tiles
                    .iter()
                    .zip(split_weighted(g.area, g.axis, &ws))
                    .map(|(t, r)| Placement::flat(*t, r))
                    .collect::<Vec<_>>()
            })
            .collect(),
        LayoutKind::ThreeColumn => three_column_groups(&params.three_column, order)
            .into_iter()
            .flat_map(|g| {
                let ws: Vec<f64> = g.tiles.iter().map(w).collect();
                g.tiles
                    .iter()
                    .zip(split_weighted(g.area, g.axis, &ws))
                    .map(|(t, r)| Placement::flat(*t, r))
                    .collect::<Vec<_>>()
            })
            .collect(),
    }
}

/// A run of tiles sharing one area, split by weight along `axis`.
#[derive(Debug, Clone)]
struct Group {
    role: Role,
    area: Rect,
    axis: Axis,
    tiles: Vec<TileId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Masters,
    Stack,
    LeftStack,
    RightStack,
}

fn tile_groups(p: &TileParams, order: &[TileId]) -> Vec<Group> {
    let n = order.len();
    if n == 0 {
        return Vec::new();
    }
    let m = usize::from(p.masters).min(n);
    let main = p.orientation.main_axis();
    let master_axis = if p.rotate_part { main } else { main.other() };
    let stack_axis = main.other();
    if m == 0 {
        return vec![Group {
            role: Role::Stack,
            area: Rect::UNIT,
            axis: stack_axis,
            tiles: order.to_vec(),
        }];
    }
    if m == n {
        return vec![Group {
            role: Role::Masters,
            area: Rect::UNIT,
            axis: master_axis,
            tiles: masters_in_place(p, order),
        }];
    }
    let (master_area, stack_area) = if p.orientation.masters_first() {
        Rect::UNIT.split(main, p.ratio)
    } else {
        let (s, m) = Rect::UNIT.split(main, 1.0 - p.ratio);
        (m, s)
    };
    vec![
        Group {
            role: Role::Masters,
            area: master_area,
            axis: master_axis,
            tiles: masters_in_place(p, &order[..m]),
        },
        Group {
            role: Role::Stack,
            area: stack_area,
            axis: stack_axis,
            tiles: order[m..].to_vec(),
        },
    ]
}

/// The masters in the order they sit, left or top first. Bismuth turns the
/// whole layout, so with the master area on the right or at the bottom,
/// masters lined up along the main axis ("rotate part") run from the far
/// edge inward: mirrored, the first keeps the outer edge. Lined up across
/// it, a turn leaves their order alone.
fn masters_in_place(p: &TileParams, masters: &[TileId]) -> Vec<TileId> {
    let mut tiles = masters.to_vec();
    if p.rotate_part && !p.orientation.masters_first() {
        tiles.reverse();
    }
    tiles
}

fn three_column_groups(p: &ThreeColumnParams, order: &[TileId]) -> Vec<Group> {
    let n = order.len();
    if n == 0 {
        return Vec::new();
    }
    let m = usize::from(p.masters.max(1)).min(n);
    let col = |x: f64, width: f64| Rect {
        x,
        y: 0.0,
        width,
        height: 1.0,
    };
    if n == m {
        return vec![Group {
            role: Role::Masters,
            area: Rect::UNIT,
            axis: Axis::Vertical,
            tiles: order.to_vec(),
        }];
    }
    if n == m + 1 {
        return vec![
            Group {
                role: Role::Masters,
                area: col(0.0, p.ratio),
                axis: Axis::Vertical,
                tiles: order[..m].to_vec(),
            },
            Group {
                role: Role::RightStack,
                area: col(p.ratio, 1.0 - p.ratio),
                axis: Axis::Vertical,
                tiles: order[m..].to_vec(),
            },
        ];
    }
    let right = (n - m) / 2;
    // Bismuth weighs the columns side : centre : side as
    // (1 − ratio) : ratio : (1 − ratio).
    let side = (1.0 - p.ratio) / (2.0 - p.ratio);
    let centre = 1.0 - 2.0 * side;
    vec![
        Group {
            role: Role::LeftStack,
            area: col(0.0, side),
            axis: Axis::Vertical,
            tiles: order[m + right..].to_vec(),
        },
        Group {
            role: Role::Masters,
            area: col(side, centre),
            axis: Axis::Vertical,
            tiles: order[..m].to_vec(),
        },
        Group {
            role: Role::RightStack,
            area: col(side + centre, 1.0 - side - centre),
            axis: Axis::Vertical,
            tiles: order[m..m + right].to_vec(),
        },
    ]
}

/// Splits `area` along `axis` into one rect per weight, in order. The last
/// rect takes whatever is left, so the pieces cover `area` exactly.
fn split_weighted(area: Rect, axis: Axis, weights: &[f64]) -> Vec<Rect> {
    let total: f64 = weights.iter().sum();
    let (start, extent) = match axis {
        Axis::Horizontal => (area.x, area.width),
        Axis::Vertical => (area.y, area.height),
    };
    let end = start + extent;
    let mut pos = start;
    let mut out = Vec::with_capacity(weights.len());
    for (i, w) in weights.iter().enumerate() {
        let len = if i + 1 == weights.len() {
            end - pos
        } else {
            extent * w / total
        };
        out.push(match axis {
            Axis::Horizontal => Rect {
                x: pos,
                width: len,
                ..area
            },
            Axis::Vertical => Rect {
                y: pos,
                height: len,
                ..area
            },
        });
        pos += len;
    }
    out
}

/// A tree whose leaves are `tiles` in a chain of even splits; the fallback
/// when placements aren't a guillotine cut (never for the kinds we have).
fn chain_of(tiles: &[TileId]) -> Node {
    match tiles {
        [] => panic!("a layout holds at least one tile"),
        [only] => Node::leaf(*only),
        [first, rest @ ..] => Node::Split {
            axis: Axis::Vertical,
            ratio: 1.0 / tiles.len() as f64,
            a: Box::new(Node::leaf(*first)),
            b: Box::new(chain_of(rest)),
        },
    }
}

/// Builds a split tree reproducing `rects` (which must tile the unit square
/// without overlap) by guillotine cuts: find a line crossing the whole area
/// without cutting a rect, split there, recurse. Vertical lines (side-by-side
/// splits) are tried first, nearest the start first, so a stack comes out as
/// the same `a`-leaf chain the tall ops build. `None` when no cut exists.
pub fn freeze(rects: &[(TileId, Rect)]) -> Option<Node> {
    cut(rects.to_vec(), Rect::UNIT)
}

fn cut(items: Vec<(TileId, Rect)>, area: Rect) -> Option<Node> {
    if items.is_empty() {
        return None;
    }
    if items.len() == 1 {
        return Some(Node::leaf(items[0].0));
    }
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let (lo, hi, extent, origin) = match axis {
            Axis::Horizontal => (area.x, area.right(), area.width, area.x),
            Axis::Vertical => (area.y, area.bottom(), area.height, area.y),
        };
        let far = |r: &Rect| match axis {
            Axis::Horizontal => r.right(),
            Axis::Vertical => r.bottom(),
        };
        let near = |r: &Rect| match axis {
            Axis::Horizontal => r.x,
            Axis::Vertical => r.y,
        };
        let mut lines: Vec<f64> = items
            .iter()
            .map(|(_, r)| far(r))
            .filter(|x| *x > lo + GEO_EPS && *x < hi - GEO_EPS)
            .collect();
        lines.sort_by(f64::total_cmp);
        lines.dedup_by(|a, b| (*a - *b).abs() < GEO_EPS);
        for line in lines {
            let clean = items
                .iter()
                .all(|(_, r)| far(r) <= line + GEO_EPS || near(r) >= line - GEO_EPS);
            if !clean {
                continue;
            }
            let (a, b): (Vec<_>, Vec<_>) = items
                .iter()
                .copied()
                .partition(|(_, r)| far(r) <= line + GEO_EPS);
            let ratio = (line - origin) / extent;
            let (area_a, area_b) = area.split(axis, ratio);
            return Some(Node::Split {
                axis,
                ratio,
                a: Box::new(cut(a, area_a)?),
                b: Box::new(cut(b, area_b)?),
            });
        }
    }
    None
}

/// How far a fitted layout's rects may be from the ones it was fitted to:
/// half a cell at 100 columns.
pub const FIT_TOLERANCE: f64 = 0.005;

/// A dynamic layout reading `rects` (one per tile, covering the tab): the
/// tiles' order, and for Tile and Three Column the ratio and weights, that
/// reproduce them within [`FIT_TOLERANCE`]. Masters, orientation and kind
/// stay. `None` when the layout's rule can't produce them: the caller
/// freezes the tab instead (ADR 0012).
///
/// Monocle reads its order off the Tile arrangement it freezes to and keeps
/// its parameters: it has no sizes of its own.
pub fn fit(layout: &TabLayout, rects: &[(TileId, Rect)]) -> Option<TabLayout> {
    let shape = match layout.kind {
        LayoutKind::Manual => return None,
        LayoutKind::Monocle => LayoutKind::Tile,
        k => k,
    };
    if rects.len() != layout.order.len() || rects.iter().any(|(t, _)| !layout.order.contains(t)) {
        return None;
    }
    // Which tile now sits in each slot of the current arrangement: nearest
    // centres first, so a swap trades slots and a resize keeps them.
    let placed = arrange(shape, &layout.params, &layout.order, &layout.weights, None);
    let slots: Vec<Rect> = layout
        .order
        .iter()
        .map(|t| {
            placed
                .iter()
                .find(|p| p.tile == *t)
                .map(|p| p.rect)
                .expect("arrange places every tile")
        })
        .collect();
    let centre = |r: &Rect| (r.x + r.width / 2.0, r.y + r.height / 2.0);
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (i, slot) in slots.iter().enumerate() {
        for (j, (_, r)) in rects.iter().enumerate() {
            let (a, b) = (centre(slot), centre(r));
            pairs.push(((a.0 - b.0).hypot(a.1 - b.1), i, j));
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut order: Vec<Option<TileId>> = vec![None; slots.len()];
    let mut used = vec![false; rects.len()];
    for (_, i, j) in pairs {
        if order[i].is_none() && !used[j] {
            order[i] = Some(rects[j].0);
            used[j] = true;
        }
    }
    let mut out = layout.clone();
    out.order = order
        .into_iter()
        .map(|t| t.expect("every slot filled"))
        .collect();
    // Weights travel with tiles; a reorder alone keeps them.
    if layout.kind == LayoutKind::Monocle {
        return Some(out);
    }

    let rect_of = |t: &TileId| rects.iter().find(|(id, _)| id == t).map(|(_, r)| *r);
    let groups = match shape {
        LayoutKind::Tile => tile_groups(&out.params.tile, &out.order),
        _ => three_column_groups(&out.params.three_column, &out.order),
    };
    let bounds = |g: &Group| {
        g.tiles
            .iter()
            .filter_map(rect_of)
            .reduce(|a, b| {
                let x = a.x.min(b.x);
                let y = a.y.min(b.y);
                Rect {
                    x,
                    y,
                    width: a.right().max(b.right()) - x,
                    height: a.bottom().max(b.bottom()) - y,
                }
            })
            .expect("groups hold tiles")
    };
    // The ratio, from the masters' area.
    if let Some(masters) = groups.iter().find(|g| g.role == Role::Masters)
        && groups.len() > 1
    {
        let m = bounds(masters);
        match shape {
            LayoutKind::Tile => {
                let p = &mut out.params.tile;
                let main = p.orientation.main_axis();
                p.ratio = match (main, p.orientation.masters_first()) {
                    (Axis::Horizontal, true) => m.right(),
                    (Axis::Horizontal, false) => 1.0 - m.x,
                    (Axis::Vertical, true) => m.bottom(),
                    (Axis::Vertical, false) => 1.0 - m.y,
                };
            }
            _ => {
                let p = &mut out.params.three_column;
                p.ratio = if groups.len() == 2 {
                    m.width
                } else {
                    2.0 * m.width / (1.0 + m.width)
                };
            }
        }
    }
    out.params.clamp();
    // Weights: each tile's share of its group, against the group's mean.
    out.weights.clear();
    for g in &groups {
        let extents: Vec<f64> = g
            .tiles
            .iter()
            .filter_map(rect_of)
            .map(|r| match g.axis {
                Axis::Horizontal => r.width,
                Axis::Vertical => r.height,
            })
            .collect();
        let mean = extents.iter().sum::<f64>() / extents.len() as f64;
        for (t, e) in g.tiles.iter().zip(extents) {
            let w = e / mean;
            if (w - 1.0).abs() > 1e-9 && w.is_finite() && w > 0.0 {
                out.weights.insert(*t, w);
            }
        }
    }
    let fitted = arrange(shape, &out.params, &out.order, &out.weights, None);
    let close = |a: f64, b: f64| (a - b).abs() <= FIT_TOLERANCE;
    let reproduces = fitted.iter().all(|p| {
        rect_of(&p.tile).is_some_and(|r| {
            close(r.x, p.rect.x)
                && close(r.y, p.rect.y)
                && close(r.width, p.rect.width)
                && close(r.height, p.rect.height)
        })
    });
    reproduces.then_some(out)
}

/// Tile parameters and stack weights that reproduce a tree exactly.
pub(crate) type TileFit = (TileParams, BTreeMap<TileId, f64>);

/// The ordered tiles a manual tree converts to: master first, then the stack
/// in reading order (see [`Node::tall`]). When the tree is in canonical tall
/// shape, also the Tile parameters and weights that reproduce it exactly.
pub(crate) fn from_tree(root: &Node) -> (Vec<TileId>, Option<TileFit>) {
    let tall = root.tall();
    let mut order = vec![tall.master];
    order.extend(tall.stack.iter().copied());
    let canonical = match root {
        Node::Split { axis, ratio, a, b } if matches!(a.as_ref(), Node::Leaf { id } if *id == tall.master) =>
        {
            let chain = b.chain_tiles(axis.other());
            (chain.as_deref() == Some(&tall.stack[..])).then_some((*axis, *ratio))
        }
        _ => None,
    };
    let fit = canonical.and_then(|(axis, ratio)| {
        if !(TILE_RATIO_MIN..=TILE_RATIO_MAX).contains(&ratio) {
            return None;
        }
        let params = TileParams {
            masters: 1,
            ratio,
            orientation: if axis == Axis::Horizontal {
                Orientation::Left
            } else {
                Orientation::Top
            },
            rotate_part: false,
        };
        // Stack weights from each tile's share of the stack.
        let rects = root.rects();
        let along = axis.other();
        let extent = |t: &TileId| {
            let r = rects.iter().find(|(id, _)| id == t).expect("leaf").1;
            match along {
                Axis::Horizontal => r.width,
                Axis::Vertical => r.height,
            }
        };
        let total: f64 = tall.stack.iter().map(extent).sum();
        let n = tall.stack.len() as f64;
        let weights = tall
            .stack
            .iter()
            .map(|t| (*t, extent(t) / total * n))
            .filter(|(_, w)| (w - 1.0).abs() > 1e-9)
            .collect();
        Some((params, weights))
    });
    (order, fit)
}

/// The geometric neighbour of `from` among `rects` in `direction`. Candidates
/// lie beyond `from` that way and overlap it on the other axis; the nearest
/// wins. Ties go to the most recently focused (earliest in `recent`), then the
/// largest overlap, then the closest centre.
pub fn neighbour_among(
    rects: &[(TileId, Rect)],
    from: TileId,
    direction: Direction,
    recent: &[TileId],
) -> Option<TileId> {
    let cur = rects.iter().find(|(id, _)| *id == from)?.1;
    let rank = |t: TileId| recent.iter().position(|r| *r == t).unwrap_or(usize::MAX);
    let candidates: Vec<(TileId, f64, f64, f64)> = rects
        .iter()
        .filter(|(id, _)| *id != from)
        .filter_map(|(id, r)| {
            let (distance, overlap, centre_offset) = match direction {
                Direction::Right if r.x >= cur.right() - GEO_EPS => (
                    r.x - cur.right(),
                    overlap(cur.y, cur.bottom(), r.y, r.bottom()),
                    ((r.y + r.bottom()) - (cur.y + cur.bottom())).abs(),
                ),
                Direction::Left if r.right() <= cur.x + GEO_EPS => (
                    cur.x - r.right(),
                    overlap(cur.y, cur.bottom(), r.y, r.bottom()),
                    ((r.y + r.bottom()) - (cur.y + cur.bottom())).abs(),
                ),
                Direction::Down if r.y >= cur.bottom() - GEO_EPS => (
                    r.y - cur.bottom(),
                    overlap(cur.x, cur.right(), r.x, r.right()),
                    ((r.x + r.right()) - (cur.x + cur.right())).abs(),
                ),
                Direction::Up if r.bottom() <= cur.y + GEO_EPS => (
                    cur.y - r.bottom(),
                    overlap(cur.x, cur.right(), r.x, r.right()),
                    ((r.x + r.right()) - (cur.x + cur.right())).abs(),
                ),
                _ => return None,
            };
            (overlap > GEO_EPS).then_some((*id, distance, overlap, centre_offset))
        })
        .collect();
    let nearest = candidates.iter().map(|c| c.1).min_by(f64::total_cmp)?;
    candidates
        .into_iter()
        .filter(|c| c.1 <= nearest + GEO_EPS)
        .min_by(|p, q| {
            rank(p.0)
                .cmp(&rank(q.0))
                .then(q.2.total_cmp(&p.2))
                .then(p.3.total_cmp(&q.3))
        })
        .map(|(id, ..)| id)
}

fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// Moves `tile`'s edge on its `direction` side outward by `amount` (unit-square
/// distance; negative moves it inward), as parameter changes. Returns false
/// when that side has no edge this layout can move.
pub(crate) fn adjust(
    layout: &mut TabLayout,
    tile: TileId,
    direction: Direction,
    amount: f64,
) -> bool {
    let groups = match layout.kind {
        LayoutKind::Tile => tile_groups(&layout.params.tile, &layout.order),
        LayoutKind::ThreeColumn => three_column_groups(&layout.params.three_column, &layout.order),
        LayoutKind::Manual | LayoutKind::Monocle => return false,
    };
    let Some(group) = groups.iter().find(|g| g.tiles.contains(&tile)) else {
        return false;
    };
    let i = group.tiles.iter().position(|t| *t == tile).expect("found");
    // An edge shared with the next or previous tile of the same group.
    if direction.axis() == group.axis {
        let j = if direction.is_forward() {
            (i + 1 < group.tiles.len()).then_some(i + 1)
        } else {
            i.checked_sub(1)
        };
        if let Some(j) = j {
            transfer(layout, group, i, j, amount);
            return true;
        }
    }
    // An edge between groups: becomes a ratio change.
    match layout.kind {
        LayoutKind::Tile => {
            let p = &mut layout.params.tile;
            if direction.axis() != p.orientation.main_axis() || groups.len() < 2 {
                return false;
            }
            let toward_stack = direction.is_forward() == p.orientation.masters_first();
            let delta = match (group.role, toward_stack) {
                (Role::Masters, true) => amount,
                (Role::Stack, false) => -amount,
                _ => return false,
            };
            p.ratio = clamp_ratio(p.ratio + delta, TILE_RATIO_MIN, TILE_RATIO_MAX, 0.5);
            true
        }
        LayoutKind::ThreeColumn => {
            if direction.axis() != Axis::Horizontal {
                return false;
            }
            let p = &mut layout.params.three_column;
            let right = direction == Direction::Right;
            // With three columns, moving an edge by `amount` changes the
            // ratio by `amount · (2 − ratio)`, as Bismuth's adjust works it
            // out; both sides then even out, so the centre stays centred.
            let k = 2.0 - p.ratio;
            let delta = match (groups.len(), group.role, right) {
                // master | stack
                (2, Role::Masters, true) => amount,
                (2, Role::RightStack, false) => -amount,
                // left | masters | right. The centre has an edge on both
                // sides and they move together, so "the edge that way" would
                // make left and right both grow it. As in Bismuth, its right
                // edge decides: right grows the centre, left shrinks it.
                (3, Role::Masters, true) => k * amount,
                (3, Role::Masters, false) => -k * amount,
                (3, Role::LeftStack, true) => -k * amount,
                (3, Role::RightStack, false) => -k * amount,
                _ => return false,
            };
            p.ratio = clamp_ratio(
                p.ratio + delta,
                THREE_COLUMN_RATIO_MIN,
                THREE_COLUMN_RATIO_MAX,
                0.6,
            );
            true
        }
        _ => false,
    }
}

/// Moves the edge between group tiles `i` and `j` (adjacent) so tile `i`
/// grows by `amount` and `j` shrinks by as much, keeping each at least
/// [`MIN_TILE_SHARE`] of the group.
fn transfer(layout: &mut TabLayout, group: &Group, i: usize, j: usize, amount: f64) {
    let ws: Vec<f64> = group.tiles.iter().map(|t| layout.weight(*t)).collect();
    let total: f64 = ws.iter().sum();
    let extent = match group.axis {
        Axis::Horizontal => group.area.width,
        Axis::Vertical => group.area.height,
    };
    let len = |w: f64| w / total * extent;
    let (ei, ej) = (len(ws[i]), len(ws[j]));
    let min = MIN_TILE_SHARE * extent;
    let lo = -(ei - min).max(0.0);
    let hi = (ej - min).max(0.0);
    let delta = amount.clamp(lo.min(0.0), hi.max(0.0));
    let to_weight = |e: f64| e / extent * total;
    let (ti, tj) = (group.tiles[i], group.tiles[j]);
    set_weight(layout, ti, to_weight(ei + delta));
    set_weight(layout, tj, to_weight(ej - delta));
}

fn set_weight(layout: &mut TabLayout, tile: TileId, w: f64) {
    if (w - 1.0).abs() < 1e-12 || !w.is_finite() || w <= 0.0 {
        layout.weights.remove(&tile);
    } else {
        layout.weights.insert(tile, w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids<const N: usize>() -> [TileId; N] {
        std::array::from_fn(|_| TileId::new())
    }

    fn rect_of(p: &[Placement], t: TileId) -> Rect {
        p.iter().find(|x| x.tile == t).unwrap().rect
    }

    fn close(a: Rect, b: (f64, f64, f64, f64)) -> bool {
        (a.x - b.0).abs() < 1e-9
            && (a.y - b.1).abs() < 1e-9
            && (a.width - b.2).abs() < 1e-9
            && (a.height - b.3).abs() < 1e-9
    }

    fn arrange_kind(kind: LayoutKind, params: LayoutParams, order: &[TileId]) -> Vec<Placement> {
        arrange(
            kind,
            &params,
            order,
            &BTreeMap::new(),
            order.first().copied(),
        )
    }

    #[test]
    fn tile_golden() {
        let [a, b, c, d] = ids();
        let p = arrange_kind(LayoutKind::Tile, LayoutParams::default(), &[a, b, c, d]);
        assert!(close(rect_of(&p, a), (0.0, 0.0, 0.5, 1.0)));
        assert!(close(rect_of(&p, b), (0.5, 0.0, 0.5, 1.0 / 3.0)));
        assert!(close(rect_of(&p, c), (0.5, 1.0 / 3.0, 0.5, 1.0 / 3.0)));
        assert!(close(rect_of(&p, d), (0.5, 2.0 / 3.0, 0.5, 1.0 / 3.0)));

        // Two masters on top at 0.6: side by side, the stack below.
        let mut params = LayoutParams {
            tile: TileParams {
                masters: 2,
                ratio: 0.6,
                orientation: Orientation::Top,
                rotate_part: false,
            },
            ..LayoutParams::default()
        };
        let p = arrange_kind(LayoutKind::Tile, params, &[a, b, c, d]);
        assert!(close(rect_of(&p, a), (0.0, 0.0, 0.5, 0.6)));
        assert!(close(rect_of(&p, b), (0.5, 0.0, 0.5, 0.6)));
        assert!(close(rect_of(&p, c), (0.0, 0.6, 0.5, 0.4)));
        assert!(close(rect_of(&p, d), (0.5, 0.6, 0.5, 0.4)));
        // Rotate part: the two masters stack instead.
        params.tile.rotate_part = true;
        let p = arrange_kind(LayoutKind::Tile, params, &[a, b, c, d]);
        assert!(close(rect_of(&p, a), (0.0, 0.0, 1.0, 0.3)));
        assert!(close(rect_of(&p, b), (0.0, 0.3, 1.0, 0.3)));
        // Left with two masters: stacked; rotated: side by side.
        params.tile = TileParams {
            masters: 2,
            ..TileParams::default()
        };
        let p = arrange_kind(LayoutKind::Tile, params, &[a, b, c]);
        assert!(close(rect_of(&p, a), (0.0, 0.0, 0.5, 0.5)));
        assert!(close(rect_of(&p, b), (0.0, 0.5, 0.5, 0.5)));
        params.tile.rotate_part = true;
        let p = arrange_kind(LayoutKind::Tile, params, &[a, b, c]);
        assert!(close(rect_of(&p, a), (0.0, 0.0, 0.25, 1.0)));
        assert!(close(rect_of(&p, b), (0.25, 0.0, 0.25, 1.0)));

        // Master on the right.
        params.tile = TileParams {
            orientation: Orientation::Right,
            ..TileParams::default()
        };
        let p = arrange_kind(LayoutKind::Tile, params, &[a, b]);
        assert!(close(rect_of(&p, a), (0.5, 0.0, 0.5, 1.0)));
        assert!(close(rect_of(&p, b), (0.0, 0.0, 0.5, 1.0)));
        // Two masters side by side on the right are mirrored, as Bismuth
        // turns the whole layout: the first keeps the outer edge. Stacked,
        // they keep their order.
        params.tile = TileParams {
            masters: 2,
            orientation: Orientation::Right,
            rotate_part: true,
            ..TileParams::default()
        };
        let p = arrange_kind(LayoutKind::Tile, params, &[a, b, c, d]);
        assert!(close(rect_of(&p, a), (0.75, 0.0, 0.25, 1.0)));
        assert!(close(rect_of(&p, b), (0.5, 0.0, 0.25, 1.0)));
        assert!(close(rect_of(&p, c), (0.0, 0.0, 0.5, 0.5)));
        params.tile.rotate_part = false;
        let p = arrange_kind(LayoutKind::Tile, params, &[a, b, c, d]);
        assert!(close(rect_of(&p, a), (0.5, 0.0, 0.5, 0.5)));
        assert!(close(rect_of(&p, b), (0.5, 0.5, 0.5, 0.5)));
        // At the bottom, two masters stacked (rotate part): the first sits on
        // the bottom edge. The stack runs left to right.
        params.tile = TileParams {
            masters: 2,
            orientation: Orientation::Bottom,
            rotate_part: true,
            ..TileParams::default()
        };
        let p = arrange_kind(LayoutKind::Tile, params, &[a, b, c, d]);
        assert!(close(rect_of(&p, a), (0.0, 0.75, 1.0, 0.25)));
        assert!(close(rect_of(&p, b), (0.0, 0.5, 1.0, 0.25)));
        assert!(close(rect_of(&p, c), (0.0, 0.0, 0.5, 0.5)));
        assert!(close(rect_of(&p, d), (0.5, 0.0, 0.5, 0.5)));

        // No masters: one stack.
        params.tile = TileParams {
            masters: 0,
            ..TileParams::default()
        };
        let p = arrange_kind(LayoutKind::Tile, params, &[a, b]);
        assert!(close(rect_of(&p, a), (0.0, 0.0, 1.0, 0.5)));
        assert!(close(rect_of(&p, b), (0.0, 0.5, 1.0, 0.5)));

        // A lone tile fills the tab.
        let p = arrange_kind(LayoutKind::Tile, LayoutParams::default(), &[a]);
        assert!(close(rect_of(&p, a), (0.0, 0.0, 1.0, 1.0)));
    }

    #[test]
    fn three_column_golden() {
        let [a, b, c, d, e] = ids();
        let params = LayoutParams::default();
        let p = arrange_kind(LayoutKind::ThreeColumn, params, &[a]);
        assert!(close(rect_of(&p, a), (0.0, 0.0, 1.0, 1.0)));
        let p = arrange_kind(LayoutKind::ThreeColumn, params, &[a, b]);
        assert!(close(rect_of(&p, a), (0.0, 0.0, 0.6, 1.0)));
        assert!(close(rect_of(&p, b), (0.6, 0.0, 0.4, 1.0)));
        // 5 tiles, 1 master: right gets floor(4/2) = 2 (b, c), left the rest
        // (d, e). The columns weigh 0.4 : 0.6 : 0.4, as in Bismuth: 2/7, 3/7,
        // 2/7 of the width.
        let (s, m) = (2.0 / 7.0, 3.0 / 7.0);
        let p = arrange_kind(LayoutKind::ThreeColumn, params, &[a, b, c, d, e]);
        assert!(close(rect_of(&p, a), (s, 0.0, m, 1.0)));
        assert!(close(rect_of(&p, b), (s + m, 0.0, s, 0.5)));
        assert!(close(rect_of(&p, c), (s + m, 0.5, s, 0.5)));
        assert!(close(rect_of(&p, d), (0.0, 0.0, s, 0.5)));
        assert!(close(rect_of(&p, e), (0.0, 0.5, s, 0.5)));
        // 3 tiles: right gets 1, left gets 1.
        let p = arrange_kind(LayoutKind::ThreeColumn, params, &[a, b, c]);
        assert!(close(rect_of(&p, b), (s + m, 0.0, s, 1.0)));
        assert!(close(rect_of(&p, c), (0.0, 0.0, s, 1.0)));
    }

    #[test]
    fn monocle_shows_only_the_focused_tile() {
        let [a, b, c] = ids();
        let p = arrange(
            LayoutKind::Monocle,
            &LayoutParams::default(),
            &[a, b, c],
            &BTreeMap::new(),
            Some(b),
        );
        for x in &p {
            assert!(close(x.rect, (0.0, 0.0, 1.0, 1.0)));
            assert_eq!(x.visible, x.tile == b);
            assert_eq!(x.z, u16::from(x.tile == b));
        }
    }

    #[test]
    fn freeze_reproduces_tile_and_three_column() {
        let order: Vec<TileId> = (0..7).map(|_| TileId::new()).collect();
        for kind in [LayoutKind::Tile, LayoutKind::ThreeColumn] {
            for n in 1..=7 {
                let p = arrange_kind(kind, LayoutParams::default(), &order[..n]);
                let rects: Vec<_> = p.iter().map(|x| (x.tile, x.rect)).collect();
                let tree = freeze(&rects).expect("guillotine");
                for (t, r) in tree.rects() {
                    let want = rects.iter().find(|(id, _)| *id == t).unwrap().1;
                    assert!(
                        close(r, (want.x, want.y, want.width, want.height)),
                        "{kind:?} n={n}"
                    );
                }
            }
        }
    }

    #[test]
    fn freeze_tile_left_is_the_canonical_tall_shape() {
        let [m, s1, s2] = ids();
        let mut tall = Node::leaf(m);
        tall.split_auto(s1).unwrap();
        tall.split_auto(s2).unwrap();
        let p = arrange_kind(LayoutKind::Tile, LayoutParams::default(), &[m, s1, s2]);
        let rects: Vec<_> = p.iter().map(|x| (x.tile, x.rect)).collect();
        let frozen = freeze(&rects).unwrap();
        assert_eq!(frozen.tiles(), tall.tiles());
        for (x, y) in frozen.ratios().iter().zip(tall.ratios()) {
            assert!((x - y).abs() < 1e-9);
        }
    }

    #[test]
    fn adjust_moves_ratio_and_weights() {
        let [a, b, c] = ids();
        let mut l = TabLayout {
            kind: LayoutKind::Tile,
            order: vec![a, b, c],
            ..TabLayout::default()
        };
        // Master's right edge outward: ratio grows.
        assert!(adjust(&mut l, a, Direction::Right, 0.1));
        assert!((l.params.tile.ratio - 0.6).abs() < 1e-9);
        // Master has no left edge.
        assert!(!adjust(&mut l, a, Direction::Left, 0.1));
        // Stack tile's left edge outward: ratio shrinks.
        assert!(adjust(&mut l, b, Direction::Left, 0.1));
        assert!((l.params.tile.ratio - 0.5).abs() < 1e-9);
        // b's bottom edge down: b grows, c shrinks by as much.
        assert!(adjust(&mut l, b, Direction::Down, 0.1));
        let p = l.arrange(a);
        assert!((rect_of(&p, b).height - 0.6).abs() < 1e-9);
        assert!((rect_of(&p, c).height - 0.4).abs() < 1e-9);
        // c has no bottom edge.
        assert!(!adjust(&mut l, c, Direction::Down, 0.1));
        // Clamped at both ends.
        assert!(adjust(&mut l, a, Direction::Right, 5.0));
        assert_eq!(l.params.tile.ratio, TILE_RATIO_MAX);
        assert!(adjust(&mut l, b, Direction::Down, 5.0));
        let p = l.arrange(a);
        assert!((rect_of(&p, c).height - MIN_TILE_SHARE).abs() < 1e-9);
    }

    #[test]
    fn adjust_three_column_keeps_the_centre() {
        let [a, b, c] = ids();
        let mut l = TabLayout {
            kind: LayoutKind::ThreeColumn,
            order: vec![a, b, c],
            ..TabLayout::default()
        };
        // The centre's right edge out by 0.05: ratio + 0.05 · (2 − 0.6).
        assert!(adjust(&mut l, a, Direction::Right, 0.05));
        assert!((l.params.three_column.ratio - 0.67).abs() < 1e-9);
        // Its left edge "out" shrinks it, as Bismuth's decrease width does;
        // otherwise left and right would both grow the centre.
        assert!(adjust(&mut l, a, Direction::Left, 0.05));
        assert!((l.params.three_column.ratio - (0.67 - 0.05 * 1.33)).abs() < 1e-9);
        l.params.three_column.ratio = 0.6;
        // The left stack's right edge out: the centre shrinks.
        assert!(adjust(&mut l, c, Direction::Right, 0.05));
        assert!((l.params.three_column.ratio - 0.53).abs() < 1e-9);
        assert!(!adjust(&mut l, c, Direction::Left, 0.05));
        assert!(!adjust(&mut l, b, Direction::Right, 0.05));
        // Two columns (one master, one stack tile): the ratio is the
        // master's share and moves with the edge.
        let mut l = TabLayout {
            kind: LayoutKind::ThreeColumn,
            order: vec![a, b],
            ..TabLayout::default()
        };
        assert!(adjust(&mut l, a, Direction::Right, 0.05));
        assert!((l.params.three_column.ratio - 0.65).abs() < 1e-9);
    }

    #[test]
    fn from_tree_reads_canonical_tall() {
        let [m, s1, s2] = ids();
        let mut tall = Node::leaf(m);
        tall.split_auto(s1).unwrap();
        tall.split_auto(s2).unwrap();
        tall.resize(m, Direction::Right, 0.1).unwrap();
        tall.resize(s1, Direction::Down, 0.1).unwrap();
        let (order, fit) = from_tree(&tall);
        assert_eq!(order, vec![m, s1, s2]);
        let (params, weights) = fit.expect("canonical");
        let layout = TabLayout {
            kind: LayoutKind::Tile,
            order,
            weights,
            params: LayoutParams {
                tile: params,
                ..LayoutParams::default()
            },
            ..TabLayout::default()
        };
        let p = layout.arrange(m);
        for (t, r) in tall.rects() {
            let got = rect_of(&p, t);
            assert!(close(got, (r.x, r.y, r.width, r.height)), "{t}");
        }
    }

    #[test]
    fn neighbour_ties_go_to_the_most_recent() {
        let [a, b, c] = ids();
        let p = arrange_kind(LayoutKind::Tile, LayoutParams::default(), &[a, b, c]);
        let rects: Vec<_> = p.iter().map(|x| (x.tile, x.rect)).collect();
        assert_eq!(neighbour_among(&rects, a, Direction::Right, &[]), Some(b));
        assert_eq!(
            neighbour_among(&rects, a, Direction::Right, &[c, b]),
            Some(c)
        );
        assert_eq!(neighbour_among(&rects, a, Direction::Left, &[]), None);
    }
}
