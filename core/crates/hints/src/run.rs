//! Where ⌘S Run sends a command: the pane the user is in when it's an idle
//! shell, else the nearest idle shell in the tab, else a new pane.
//!
//! Pure policy. The shell says which panes are idle shells (from the core's
//! polled foreground process), where they sit and which machine they run
//! on; this picks one and works out the line to type.

use serde::{Deserialize, Serialize};

use crate::callout::Rect;
use crate::select::run_line;

/// `[hints] run_target`: reuse an idle shell, or always open a new pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Reuse,
    New,
}

/// One terminal pane in the tab.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Pane {
    pub tile: String,
    /// Where it sits in the tab's layout (any one coordinate space).
    pub rect: Rect,
    /// The herdr machine it runs on (`local` for Local).
    pub machine: String,
    /// A shell at its prompt with nothing in the foreground. Unknown counts
    /// as busy.
    pub idle: bool,
    /// Its shell's working directory, when known.
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub mode: Mode,
    /// The command to run.
    pub value: String,
    /// The directory it belongs in: its source pane's.
    #[serde(default)]
    pub cwd: Option<String>,
    /// The machine the command came from; it never runs anywhere else.
    pub machine: String,
    /// The tile the user is in (the tab's focused tile).
    pub current: String,
    /// The tab's terminal panes, in reading order.
    pub panes: Vec<Pane>,
    /// Tiles used as Run targets before, most recent first.
    #[serde(default)]
    pub recent: Vec<String>,
}

/// Where the command goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "target", content = "tile", rename_all = "lowercase")]
pub enum Target {
    /// The pane the user is in.
    Current,
    /// Another idle shell in the tab.
    Reuse(String),
    /// A new pane beside the source, as Run always did.
    New,
}

/// What Run does: the target and the line to type there (Return after it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    #[serde(flatten)]
    pub target: Target,
    pub line: String,
}

/// Rects this close count as equally near.
const EPS: f64 = 1e-6;

/// Picks the target and the line. A new pane starts in the source's
/// directory and gets the command as [`run_line`] has it; an existing shell
/// gets a `cd` first when it sits elsewhere.
pub fn plan(request: &Request) -> Plan {
    let line = run_line(&request.value);
    let new = || Plan {
        target: Target::New,
        line: line.clone(),
    };
    if request.mode == Mode::New {
        return new();
    }
    let usable = |p: &Pane| p.idle && p.machine == request.machine;
    let Some(here) = request.panes.iter().find(|p| p.tile == request.current) else {
        return new();
    };
    let chosen = if usable(here) {
        Some((Target::Current, here))
    } else {
        nearest(here, request).map(|p| (Target::Reuse(p.tile.clone()), p))
    };
    let Some((target, pane)) = chosen else {
        return new();
    };
    match in_dir(&line, request.cwd.as_deref(), pane.cwd.as_deref()) {
        Some(line) => Plan { target, line },
        None => new(),
    }
}

/// The idle shell nearest `from` on the source's machine: the smallest gap
/// between the two rects (neighbours touch: 0), then the most recent Run
/// target, then the closest centre, then reading order.
fn nearest<'a>(from: &Pane, request: &'a Request) -> Option<&'a Pane> {
    let rank = |tile: &str| {
        request
            .recent
            .iter()
            .position(|t| t == tile)
            .unwrap_or(usize::MAX)
    };
    let candidates: Vec<(usize, &Pane, f64, f64)> = request
        .panes
        .iter()
        .enumerate()
        .filter(|(_, p)| p.tile != from.tile && p.idle && p.machine == request.machine)
        .map(|(i, p)| {
            (
                i,
                p,
                gap(&from.rect, &p.rect),
                centre_distance(&from.rect, &p.rect),
            )
        })
        .collect();
    let near = candidates.iter().map(|c| c.2).min_by(f64::total_cmp)?;
    candidates
        .into_iter()
        .filter(|c| c.2 <= near + EPS)
        .min_by(|a, b| {
            rank(&a.1.tile)
                .cmp(&rank(&b.1.tile))
                .then(a.3.total_cmp(&b.3))
                .then(a.0.cmp(&b.0))
        })
        .map(|c| c.1)
}

/// The distance between two rects' nearest edges (0 when they touch or
/// overlap).
fn gap(a: &Rect, b: &Rect) -> f64 {
    let dx = (b.x - (a.x + a.w)).max(a.x - (b.x + b.w)).max(0.0);
    let dy = (b.y - (a.y + a.h)).max(a.y - (b.y + b.h)).max(0.0);
    dx.hypot(dy)
}

fn centre_distance(a: &Rect, b: &Rect) -> f64 {
    let (ax, ay) = (a.x + a.w / 2.0, a.y + a.h / 2.0);
    let (bx, by) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    (ax - bx).hypot(ay - by)
}

/// `line` for a shell sitting in `there`: as it is when that's `cwd` (or
/// `cwd` is unknown), else after a `cd`. None when the directory can't be
/// quoted the same way in POSIX shells and fish (a backslash or a control
/// character).
fn in_dir(line: &str, cwd: Option<&str>, there: Option<&str>) -> Option<String> {
    let Some(cwd) = cwd else {
        return Some(line.to_owned());
    };
    if there == Some(cwd) {
        return Some(line.to_owned());
    }
    if cwd.chars().any(|c| c == '\\' || c.is_control()) {
        return None;
    }
    Some(format!("cd {} && {line}", quote(cwd)))
}

/// Single quotes for POSIX shells and fish alike: `'\''` closes the quote,
/// adds an escaped `'` and reopens it in both.
fn quote(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
    {
        return word.to_owned();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}
