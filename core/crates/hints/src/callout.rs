//! Where ⌘S labels go: Homerow-style callouts, a pill next to each item's
//! first cell with a short tail pointing at it. Pure geometry in the
//! overlay's coordinates (points, y grows down); the shell measures the
//! pills (fonts) and draws them.

use serde::{Deserialize, Serialize};

/// A rectangle: origin and size, as `[x, y, width, height]` in JSON.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(from = "[f64; 4]", into = "[f64; 4]")]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl From<[f64; 4]> for Rect {
    fn from([x, y, w, h]: [f64; 4]) -> Rect {
        Rect { x, y, w, h }
    }
}

impl From<Rect> for [f64; 4] {
    fn from(r: Rect) -> [f64; 4] {
        [r.x, r.y, r.w, r.h]
    }
}

/// A point, as `[x, y]` in JSON.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(from = "[f64; 2]", into = "[f64; 2]")]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl From<[f64; 2]> for Point {
    fn from([x, y]: [f64; 2]) -> Point {
        Point { x, y }
    }
}

impl From<Point> for [f64; 2] {
    fn from(p: Point) -> [f64; 2] {
        [p.x, p.y]
    }
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn min_x(&self) -> f64 {
        self.x
    }
    pub fn max_x(&self) -> f64 {
        self.x + self.w
    }
    pub fn mid_x(&self) -> f64 {
        self.x + self.w / 2.0
    }
    pub fn min_y(&self) -> f64 {
        self.y
    }
    pub fn max_y(&self) -> f64 {
        self.y + self.h
    }
    pub fn mid_y(&self) -> f64 {
        self.y + self.h / 2.0
    }

    /// Shrunk by `dx` and `dy` on each side (grown when negative); `None`
    /// when nothing is left (Core Graphics' null rectangle).
    pub fn inset(&self, dx: f64, dy: f64) -> Option<Rect> {
        let r = Rect {
            x: self.x + dx,
            y: self.y + dy,
            w: self.w - 2.0 * dx,
            h: self.h - 2.0 * dy,
        };
        (r.w >= 0.0 && r.h >= 0.0).then_some(r)
    }

    /// Overlap with some area; rectangles that only touch don't.
    pub fn intersects(&self, other: &Rect) -> bool {
        !(self.max_x() <= other.min_x()
            || other.max_x() <= self.min_x()
            || self.max_y() <= other.min_y()
            || other.max_y() <= self.min_y())
    }

    /// The point is inside (the far edges excluded).
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.min_x() && p.x < self.max_x() && p.y >= self.min_y() && p.y < self.max_y()
    }

    /// The rectangle is wholly inside.
    pub fn contains_rect(&self, r: &Rect) -> bool {
        r.min_x() >= self.min_x()
            && r.max_x() <= self.max_x()
            && r.min_y() >= self.min_y()
            && r.max_y() <= self.max_y()
    }
}

fn intersects(a: Option<Rect>, b: Option<Rect>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if a.intersects(&b))
}

/// Which side of the item's first cell a callout sits on; its tail points
/// back at the cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// Over the row below, tail up at the cell's bottom middle.
    Below,
    /// Over the row above, tail down at the cell's top middle.
    Above,
    /// On the item's own row, in the blank margin before it, tail pointing
    /// right at the cell's left middle.
    Left,
    /// Over the item's own first cells, no tail: the last resort before
    /// covering another item's text (a column of paths at the pane's left
    /// edge, one per row).
    Inline,
}

/// A placed label.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Callout {
    pub frame: Rect,
    /// Where the tail points (see [`Side`]).
    pub tip: Point,
    /// Where the tail leaves the pill ([`tail_base`] with the placement's
    /// `inset`).
    pub base: Point,
    pub side: Side,
    /// Slid sideways off the cell's middle (below or above), within the
    /// reach that keeps its tail nearly vertical.
    pub nudged: bool,
    /// How many of `avoid`'s layers it had to give up (the overlay's: 0 is
    /// blank space, 1 plain text, 2 a paragraph, 3 another item).
    pub relaxed: usize,
    /// Overlaps another callout or another item's first cell anyway:
    /// nowhere free at all.
    pub clash: bool,
}

/// One label to place: its pill's size and the item's first cell.
#[derive(Debug, Clone)]
pub struct Request {
    pub cell: Rect,
    pub size: (f64, f64),
    /// What a pill sitting over the item's own first cells must keep off
    /// (other items' text, not the item's or what contains it); `None`:
    /// never inline.
    pub inline: Option<Vec<Rect>>,
    /// Where the item's first row ends: an inline pill stays before it.
    pub inline_end: Option<f64>,
}

/// Where a callout's tail leaves its pill: on the edge facing the cell, in
/// line with the tip but at least `inset` from the pill's ends (its rounded
/// corners).
pub fn tail_base(frame: &Rect, tip: Point, side: Side, inset: f64) -> Point {
    let clamp = |v: f64, lo: f64, hi: f64| {
        if lo > hi {
            (lo + hi) / 2.0
        } else {
            v.max(lo).min(hi)
        }
    };
    match side {
        Side::Below => Point {
            x: clamp(tip.x, frame.min_x() + inset, frame.max_x() - inset),
            y: frame.min_y(),
        },
        Side::Above => Point {
            x: clamp(tip.x, frame.min_x() + inset, frame.max_x() - inset),
            y: frame.max_y(),
        },
        Side::Left => Point {
            x: frame.max_x(),
            y: clamp(tip.y, frame.min_y() + inset, frame.max_y() - inset),
        },
        Side::Inline => tip,
    }
}

/// Placement settings, in points.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub bounds: Rect,
    /// The tail's length: the gap between cell and pill.
    pub tail: f64,
    /// The gap kept between pills and what they keep off.
    pub spacing: f64,
    /// How far a tail's foot keeps from the pill's ends.
    pub inset: f64,
    /// How far a tail's foot may stray from the cell's middle.
    pub drift: f64,
}

/// Homerow-style callouts, per pane in reading order. Each pill sits next
/// to its item's first cell with a short tail pointing at it: below (over
/// the row underneath), above, or before it on its own row (the blank
/// margin left of an indented item; `bounds` may take in the pane's
/// padding). Below or above, a pill slides sideways only as far as keeps
/// its tail's foot within `drift` of the cell's middle, so tails stay
/// nearly vertical: never a long slant across the text.
///
/// Every spot keeps off earlier pills and other items' first cells
/// (`obstacles`). `avoid` lists more to keep off, strictest first (the
/// overlay passes all text, then paragraphs, then items); the first number
/// of layers kept that leaves a spot wins, trying below, above, then left
/// (above first on the last row). So a pill covers another item's text only
/// when no blank or plain-text spot exists on any side. Nothing free at
/// all: centred below (above on the last row), flagged `clash`.
pub fn place(
    requests: &[Request],
    obstacles: &[Rect],
    avoid: &[Vec<Rect>],
    m: Metrics,
) -> Vec<Callout> {
    let bounds = m.bounds;
    let mut placed: Vec<Rect> = Vec::new();
    let mut result = Vec::with_capacity(requests.len());
    for request in requests {
        let (width, height) = request.size;
        let cell = request.cell;
        let shrunk = cell.inset(1.0, 1.0);
        let others: Vec<Rect> = obstacles
            .iter()
            .copied()
            .filter(|o| !intersects(Some(*o), shrunk))
            .collect();
        let centre = cell.mid_x() - width / 2.0;
        let reach = (width / 2.0 - m.inset).max(0.0) + m.drift;
        let blockers = |placed: &[Rect], frame: Rect, layers: &[&Vec<Rect>]| -> Vec<Rect> {
            let grown = frame.inset(-m.spacing / 2.0, 0.0);
            placed
                .iter()
                .chain(others.iter())
                .chain(layers.iter().flat_map(|l| l.iter()))
                .filter(|r| intersects(r.inset(0.5, 0.5), grown))
                .copied()
                .collect()
        };
        // The free x nearest the centre on a row, within reach, walking
        // past each blocker to either side.
        let slide = |placed: &[Rect], y: f64, layers: &[&Vec<Rect>]| -> Option<f64> {
            let lo = bounds.min_x().max(centre - reach);
            let hi = (bounds.max_x() - width).min(centre + reach);
            if lo > hi {
                return None;
            }
            let mut best: Option<f64> = None;
            for right in [true, false] {
                let mut x = centre.max(lo).min(hi);
                for _ in 0..32 {
                    let hits = blockers(placed, Rect::new(x, y, width, height), layers);
                    if hits.is_empty() {
                        if best.is_none_or(|b| (x - centre).abs() < (b - centre).abs()) {
                            best = Some(x);
                        }
                        break;
                    }
                    let next = if right {
                        hits.iter().map(Rect::max_x).fold(f64::MIN, f64::max) + m.spacing
                    } else {
                        hits.iter().map(Rect::min_x).fold(f64::MAX, f64::min) - m.spacing - width
                    };
                    if next < lo || next > hi {
                        break;
                    }
                    x = next;
                }
            }
            best
        };
        let below_y = cell.max_y() + m.tail;
        let above_y = cell.min_y() - m.tail - height;
        let below_fits = below_y + height <= bounds.max_y();
        let above_fits = above_y >= bounds.min_y();
        let left = Rect::new(
            cell.min_x() - m.tail - width,
            cell.mid_y() - height / 2.0,
            width,
            height,
        );
        let left_fits = left.min_x() >= bounds.min_x()
            && left.min_y() >= bounds.min_y()
            && left.max_y() <= bounds.max_y();
        // The left pill and its tail, short of the item's own cell.
        let left_probe = Rect::new(
            left.min_x(),
            left.min_y(),
            (cell.min_x() - 1.0 - m.spacing / 2.0 - left.min_x()).max(0.0),
            left.h,
        );
        let sides = if below_fits {
            [Side::Below, Side::Above, Side::Left]
        } else {
            [Side::Above, Side::Below, Side::Left]
        };
        let callout =
            |frame: Rect, tip: Point, side: Side, nudged: bool, relaxed: usize, clash: bool| {
                Callout {
                    frame,
                    tip,
                    base: tail_base(&frame, tip, side, m.inset),
                    side,
                    nudged,
                    relaxed,
                    clash,
                }
            };
        let mut choice: Option<Callout> = None;
        'search: for relaxed in 0..=avoid.len() {
            let layers: Vec<&Vec<Rect>> = avoid[relaxed..].iter().collect();
            if relaxed == avoid.len()
                && relaxed > 0
                && let Some(inline) = &request.inline
            {
                // Before covering another item's text: over its own first
                // row, from its start, past earlier pills.
                let end = request
                    .inline_end
                    .unwrap_or(cell.max_x())
                    .max(cell.min_x() + width)
                    .min(bounds.max_x());
                let mut x = cell.min_x().max(bounds.min_x());
                for _ in 0..8 {
                    if x + width > end {
                        break;
                    }
                    let frame = Rect::new(x, cell.mid_y() - height / 2.0, width, height);
                    let hits = blockers(&placed, frame, &[inline]);
                    if hits.is_empty() {
                        let tip = Point {
                            x: cell.mid_x(),
                            y: cell.mid_y(),
                        };
                        choice = Some(callout(frame, tip, Side::Inline, false, relaxed, false));
                        break 'search;
                    }
                    x = hits.iter().map(Rect::max_x).fold(f64::MIN, f64::max) + m.spacing;
                }
            }
            for side in sides {
                match side {
                    Side::Below | Side::Above => {
                        if !(if side == Side::Below {
                            below_fits
                        } else {
                            above_fits
                        }) {
                            continue;
                        }
                        let y = if side == Side::Below {
                            below_y
                        } else {
                            above_y
                        };
                        let Some(x) = slide(&placed, y, &layers) else {
                            continue;
                        };
                        let tip = Point {
                            x: cell.mid_x(),
                            y: if side == Side::Below {
                                cell.max_y()
                            } else {
                                cell.min_y()
                            },
                        };
                        choice = Some(callout(
                            Rect::new(x, y, width, height),
                            tip,
                            side,
                            (x - centre).abs() > 0.5,
                            relaxed,
                            false,
                        ));
                    }
                    Side::Left => {
                        if !left_fits || !blockers(&placed, left_probe, &layers).is_empty() {
                            continue;
                        }
                        let tip = Point {
                            x: cell.min_x(),
                            y: cell.mid_y(),
                        };
                        choice = Some(callout(left, tip, Side::Left, false, relaxed, false));
                    }
                    Side::Inline => continue,
                }
                break 'search;
            }
        }
        let chosen = choice.unwrap_or_else(|| {
            let x = centre.max(bounds.min_x()).min(bounds.max_x() - width);
            let below = below_fits || !above_fits;
            let tip = Point {
                x: cell.mid_x(),
                y: if below { cell.max_y() } else { cell.min_y() },
            };
            callout(
                Rect::new(x, if below { below_y } else { above_y }, width, height),
                tip,
                if below { Side::Below } else { Side::Above },
                (x - centre).abs() > 0.5,
                avoid.len(),
                true,
            )
        });
        placed.push(chosen.frame);
        result.push(chosen);
    }
    result
}

/// An item as callout placement sees it: its first cell, the rects of its
/// text, whether it's a paragraph, and its pill's size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub anchor: Rect,
    pub rects: Vec<Rect>,
    #[serde(default)]
    pub prose: bool,
    /// `[width, height]`.
    pub size: [f64; 2],
}

/// The overlay's placement: pills keep off blank space first, then plain
/// text (`text`: every run of non-blank cells), then paragraphs, then other
/// items' text.
pub fn place_items(items: &[Item], text: &[Rect], m: Metrics) -> Vec<Callout> {
    let requests: Vec<Request> = items
        .iter()
        .map(|item| {
            // Inline, a pill may cover its item and what contains it (a
            // path's command, an address's paragraph), nothing else.
            let point = Point {
                x: item.anchor.mid_x(),
                y: item.anchor.mid_y(),
            };
            let inline: Vec<Rect> = items
                .iter()
                .filter(|other| !other.rects.iter().any(|r| r.contains(point)))
                .flat_map(|other| other.rects.iter().copied())
                .collect();
            Request {
                cell: item.anchor,
                size: (item.size[0], item.size[1]),
                inline: Some(inline),
                inline_end: item.rects.first().map(Rect::max_x),
            }
        })
        .collect();
    let anchors: Vec<Rect> = items.iter().map(|i| i.anchor).collect();
    let prose: Vec<Rect> = items
        .iter()
        .filter(|i| i.prose)
        .flat_map(|i| i.rects.iter().copied())
        .collect();
    let others: Vec<Rect> = items
        .iter()
        .filter(|i| !i.prose)
        .flat_map(|i| i.rects.iter().copied())
        .collect();
    place(&requests, &anchors, &[text.to_vec(), prose, others], m)
}
