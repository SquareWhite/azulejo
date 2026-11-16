//! ⌘S "smart select" over the C ABI: detection, the selection keys and
//! label placement live in [`azulejo_hints`]; the shell sends the screen
//! and draws what comes back.
//!
//! One `azj_hints_detect` per pane per ⌘S returns its items (paragraphs
//! and lines included, in reading order) as JSON and a pane handle. Selection keys
//! then ask the handle (`azj_hints_pane_*`) with small C structs, without
//! sending the screen again. Positions are rows and UTF-16 offsets in them,
//! as the shell's strings count.

use std::ffi::{CString, c_char};

use azulejo_hints::callout::{self, Metrics, Rect};
use azulejo_hints::detect::{Input, StdFilesystem, detect, matching_colors};
use azulejo_hints::run;
use azulejo_hints::select::{self, Direction, Level, Pane, Pos, TextRange, Unit};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AzjCore, arg, failure, out};

/// A pane's items after ⌘S, for the selection keys. Free with
/// `azj_hints_pane_free`.
pub struct AzjHintsPane(Pane);

/// `start` up to `end` (exclusive): rows and UTF-16 offsets.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AzjHintRange {
    pub start_row: u32,
    pub start_offset: u32,
    pub end_row: u32,
    pub end_offset: u32,
}

/// A selection step: a range, its level (0 item, 1 text paragraph, 2 line,
/// 3 block, 4 screen) and, for an item, paragraph or line, its index in the
/// pane's kept items (-1 for none).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AzjHintUnit {
    pub range: AzjHintRange,
    pub level: u8,
    pub entry: i32,
}

impl From<AzjHintRange> for TextRange {
    fn from(r: AzjHintRange) -> TextRange {
        TextRange {
            start: Pos {
                row: r.start_row as usize,
                offset: r.start_offset as usize,
            },
            end: Pos {
                row: r.end_row as usize,
                offset: r.end_offset as usize,
            },
        }
    }
}

impl From<TextRange> for AzjHintRange {
    fn from(r: TextRange) -> AzjHintRange {
        AzjHintRange {
            start_row: r.start.row as u32,
            start_offset: r.start.offset as u32,
            end_row: r.end.row as u32,
            end_offset: r.end.offset as u32,
        }
    }
}

impl From<Unit> for AzjHintUnit {
    fn from(u: Unit) -> AzjHintUnit {
        AzjHintUnit {
            range: u.range.into(),
            level: u.level as u8,
            entry: u.entry.map_or(-1, |e| e as i32),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DetectRequest {
    rows: Vec<String>,
    /// Rows whose last cell soft-wraps into the next.
    #[serde(default)]
    joins: Vec<usize>,
    #[serde(default)]
    columns: Option<usize>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    local: bool,
    #[serde(default)]
    branch: Option<String>,
    /// The terminal's styled snapshot of the same rows (SGR colours).
    #[serde(default)]
    vt: Option<String>,
    #[serde(default)]
    row_classifier: bool,
}

/// Finds a pane's items. `request_json`:
///
/// `{"rows":["…"],"joins":[0],"columns":120,"cwd":"/abs/dir","local":true,
///   "branch":"main","vt":"<styled snapshot>","row_classifier":true}`
///
/// (`joins`: rows whose last cell soft-wraps into the next; `vt`: the
/// terminal's SGR snapshot of the same rows, whose colours mark inline
/// commands; `local`: paths may open on this machine.) Returns
/// `{"ok":true,"items":[{"kind":"path","value":"src/a.rs","spans":[[row,start,length]],
/// "open":"file:///…"}],"extents":[[start,end]|null],"runs":[[start,length,…]]}`:
/// items (paragraphs as `text`, every non-blank line as `line`) in reading
/// order, a line or paragraph before the items inside it; each row's text extent without its indent (null when
/// blank) and its runs of non-blank characters, flattened. All offsets are
/// UTF-16 units. Or `{"ok":false,"error":{…}}` on a bad request.
///
/// With `pane_out`, also hands back the pane for the selection keys
/// (`azj_hints_pane_*`); free it with `azj_hints_pane_free`. Pure apart
/// from filesystem checks for paths; safe from any thread. Free the string
/// with `azj_string_free`.
///
/// # Safety
/// `request_json` must be a valid C string or NULL; `pane_out` NULL or
/// writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_detect(
    request_json: *const c_char,
    pane_out: *mut *mut AzjHintsPane,
) -> *mut c_char {
    if !pane_out.is_null() {
        unsafe { *pane_out = std::ptr::null_mut() };
    }
    let request: DetectRequest = match unsafe { arg(request_json) }
        .ok_or("request isn't a UTF-8 C string".to_owned())
        .and_then(|text| serde_json::from_str(text).map_err(|e| e.to_string()))
    {
        Ok(request) => request,
        Err(e) => return out(failure("invalid_params", &e)),
    };
    let (reply, pane) = detect_json(request);
    if !pane_out.is_null() {
        unsafe { *pane_out = Box::into_raw(Box::new(AzjHintsPane(pane))) };
    }
    out(reply)
}

fn detect_json(request: DetectRequest) -> (Value, Pane) {
    let colors = request
        .vt
        .as_deref()
        .map(|vt| matching_colors(&request.rows, vt))
        .unwrap_or_default();
    let rows: Vec<&str> = request.rows.iter().map(String::as_str).collect();
    let input = Input {
        rows: &rows,
        joins: &request.joins,
        columns: request.columns,
        cwd: request.cwd.as_deref(),
        local: request.local,
        branch: request.branch.as_deref(),
        colors: &colors,
        row_classifier: request.row_classifier,
    };
    let detected = detect(&input, &StdFilesystem);
    let pane = Pane::new(
        request.rows.clone(),
        request.joins.clone(),
        detected,
        request.columns,
    );
    let items: Vec<Value> = pane
        .items
        .iter()
        .map(|item| {
            json!({
                "kind": item.kind.name(),
                "value": item.value,
                "spans": item.spans.iter().map(|s| [s.row, s.start, s.length]).collect::<Vec<_>>(),
                "open": item.open,
            })
        })
        .collect();
    let extents: Vec<Value> = (0..rows.len())
        .map(|r| select::row_extent(&rows, r).map_or(Value::Null, |(s, e)| json!([s, e])))
        .collect();
    let runs: Vec<Vec<usize>> = rows
        .iter()
        .map(|row| {
            select::text_runs(row)
                .into_iter()
                .flat_map(|(s, l)| [s, l])
                .collect()
        })
        .collect();
    (
        json!({"ok": true, "items": items, "extents": extents, "runs": runs}),
        pane,
    )
}

/// Keeps only these items (indices into what `azj_hints_detect` returned,
/// ascending): the ones the shell could place. Later `entry` indices count
/// the kept items.
///
/// # Safety
/// `pane` must come from `azj_hints_detect`; `indices` must point to
/// `count` values (or be NULL with `count` 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_pane_keep(
    pane: *mut AzjHintsPane,
    indices: *const u32,
    count: usize,
) {
    let Some(pane) = (unsafe { pane.as_mut() }) else {
        return;
    };
    pane.0.keep(&unsafe { self::indices(indices, count) });
}

fn write(unit: Option<Unit>, out: *mut AzjHintUnit) -> bool {
    match unit {
        Some(unit) if !out.is_null() => {
            unsafe { *out = unit.into() };
            true
        }
        _ => false,
    }
}

/// ⌥↑: the next thing out that holds `range` (the smallest item around
/// it, its rows, the smallest paragraph, its block, the screen). False when
/// nothing is bigger.
///
/// # Safety
/// `pane` must come from `azj_hints_detect`; `out` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_pane_grow(
    pane: *const AzjHintsPane,
    range: AzjHintRange,
    out: *mut AzjHintUnit,
) -> bool {
    let Some(pane) = (unsafe { pane.as_ref() }) else {
        return false;
    };
    write(pane.0.grow(&range.into()), out)
}

/// ⌥↓ with nothing to go back to: the first item inside `range` (from a
/// line, block or screen, one that fills it exactly counts). `level` is the
/// selection's level. False when there is none.
///
/// # Safety
/// `pane` must come from `azj_hints_detect`; `out` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_pane_shrink(
    pane: *const AzjHintsPane,
    range: AzjHintRange,
    level: u8,
    out: *mut AzjHintUnit,
) -> bool {
    let Some(pane) = (unsafe { pane.as_ref() }) else {
        return false;
    };
    let Some(level) = Level::from_u8(level) else {
        return false;
    };
    write(pane.0.shrink(&range.into(), level), out)
}

/// An arrow key from `from`, at its level: `direction` 0 left, 1 right,
/// 2 up, 3 down. False when there is nothing that way.
///
/// # Safety
/// `pane` must come from `azj_hints_detect`; `out` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_pane_neighbour(
    pane: *const AzjHintsPane,
    from: AzjHintUnit,
    direction: u8,
    out: *mut AzjHintUnit,
) -> bool {
    let Some(pane) = (unsafe { pane.as_ref() }) else {
        return false;
    };
    let (Some(level), Some(direction)) = (Level::from_u8(from.level), self::direction(direction))
    else {
        return false;
    };
    let from = Unit {
        range: from.range.into(),
        level,
        entry: usize::try_from(from.entry).ok(),
    };
    write(pane.0.neighbour(&from, direction), out)
}

fn direction(value: u8) -> Option<Direction> {
    [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ]
    .get(value as usize)
    .copied()
}

/// `count` indices at `indices` (none when NULL or 0).
///
/// # Safety
/// `indices` must point to `count` values or be NULL.
unsafe fn indices(indices: *const u32, count: usize) -> Vec<usize> {
    if indices.is_null() || count == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(indices, count) }
        .iter()
        .map(|&i| i as usize)
        .collect()
}

/// An arrow key while choosing, before anything is selected: the item the
/// focus ring moves to among `shown` (the kept items whose labels still
/// show, `count` of them). From item `focus`; with none (-1), from the
/// terminal cursor at `cursor_row` / `cursor_offset` (row -1: unknown,
/// taken as below the last row). `direction` 0 left, 1 right, 2 up, 3 down.
/// Writes the item's index to `out`; false when there is nothing that way.
///
/// # Safety
/// `pane` must come from `azj_hints_detect`; `shown` must point to `count`
/// values (or be NULL with `count` 0); `out` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_pane_focus(
    pane: *const AzjHintsPane,
    shown: *const u32,
    count: usize,
    focus: i32,
    cursor_row: i32,
    cursor_offset: u32,
    direction: u8,
    out: *mut u32,
) -> bool {
    let (Some(pane), Some(direction)) = (unsafe { pane.as_ref() }, self::direction(direction))
    else {
        return false;
    };
    let shown = unsafe { indices(shown, count) };
    let cursor = usize::try_from(cursor_row).ok().map(|row| Pos {
        row,
        offset: cursor_offset as usize,
    });
    let next = pane
        .0
        .focus_step(&shown, usize::try_from(focus).ok(), cursor, direction);
    match next {
        Some(index) if !out.is_null() => {
            unsafe { *out = index as u32 };
            true
        }
        _ => false,
    }
}

/// The item among `shown` nearest item `to` (fewest rows apart, then
/// columns): where the focus ring goes when typing filters out the focused
/// item. Writes its index to `out`; false when `shown` is empty.
///
/// # Safety
/// `pane` must come from `azj_hints_detect`; `shown` must point to `count`
/// values (or be NULL with `count` 0); `out` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_pane_nearest(
    pane: *const AzjHintsPane,
    shown: *const u32,
    count: usize,
    to: u32,
    out: *mut u32,
) -> bool {
    let Some(pane) = (unsafe { pane.as_ref() }) else {
        return false;
    };
    let shown = unsafe { indices(shown, count) };
    match pane.0.nearest(&shown, to as usize) {
        Some(index) if !out.is_null() => {
            unsafe { *out = index as u32 };
            true
        }
        _ => false,
    }
}

/// How many places arrow keys can go at `level` (0 when none).
///
/// # Safety
/// `pane` must come from `azj_hints_detect`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_pane_targets(pane: *const AzjHintsPane, level: u8) -> usize {
    let Some(pane) = (unsafe { pane.as_ref() }) else {
        return 0;
    };
    Level::from_u8(level).map_or(0, |level| pane.0.targets(level).len())
}

/// The text a terminal selection of `range` would copy: rows joined with
/// newlines (soft-wrapped rows without one), trailing blanks dropped. Free
/// with `azj_string_free`; NULL on a bad handle.
///
/// # Safety
/// `pane` must come from `azj_hints_detect`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_pane_text(
    pane: *const AzjHintsPane,
    range: AzjHintRange,
) -> *mut c_char {
    let Some(pane) = (unsafe { pane.as_ref() }) else {
        return std::ptr::null_mut();
    };
    CString::new(pane.0.text(&range.into())).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// # Safety
/// `pane` must come from `azj_hints_detect` (or be NULL) and not be used after.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_pane_free(pane: *mut AzjHintsPane) {
    if !pane.is_null() {
        drop(unsafe { Box::from_raw(pane) });
    }
}

/// Labels for every kept item of these panes, in order (pane by pane):
/// the kind's letter, plus home-row letters when a kind has more than one
/// item; an item repeated in a pane (same kind and value) gets one label,
/// on its last copy, the others "" (lines excepted: each line gets one);
/// paragraphs get "". Returns `{"ok":true,"labels":["pa","",…]}`.
/// Free with `azj_string_free`.
///
/// # Safety
/// `panes` must point to `count` handles from `azj_hints_detect` (or be
/// NULL with `count` 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_labels(
    panes: *const *const AzjHintsPane,
    count: usize,
) -> *mut c_char {
    let panes: &[*const AzjHintsPane] = if panes.is_null() || count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(panes, count) }
    };
    let mut keys = Vec::new();
    for (index, pane) in panes.iter().enumerate() {
        let Some(pane) = (unsafe { pane.as_ref() }) else {
            continue;
        };
        keys.extend(
            pane.0
                .items
                .iter()
                .map(|i| (index, i.kind, i.value.as_str())),
        );
    }
    out(json!({"ok": true, "labels": select::pane_labels(&keys)}))
}

/// The line Run types into a new pane's login shell for a command: the
/// command itself, or for a multi-line one a `/bin/sh` line that runs it
/// unchanged in any shell. Free with `azj_string_free`; NULL if `value`
/// isn't a UTF-8 C string.
///
/// # Safety
/// `value` must be a valid C string or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_run_line(value: *const c_char) -> *mut c_char {
    let Some(value) = (unsafe { arg(value) }) else {
        return std::ptr::null_mut();
    };
    CString::new(select::run_line(value)).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Where Run sends a command and what it types there. `request_json`:
///
/// `{"mode":"reuse","value":"make test","cwd":"/w","machine":"local","current":"<tile>",
///   "panes":[{"tile":"<tile>","rect":[x,y,w,h],"machine":"local","idle":true,"cwd":"/w"}],
///   "recent":["<tile>"]}`
///
/// the tab's terminal panes in reading order (idle: a shell at its prompt,
/// unknown counting as busy), the focused tile and earlier Run targets, most
/// recent first. Returns `{"ok":true,"target":"current"|"reuse"|"new",
/// "tile":"<tile>"(reuse only),"line":"…"}` or `{"ok":false,…}`. Free with
/// `azj_string_free`.
///
/// # Safety
/// `request_json` must be a valid C string or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_run_plan(request_json: *const c_char) -> *mut c_char {
    let request: run::Request = match unsafe { arg(request_json) }
        .ok_or("request isn't a UTF-8 C string".to_owned())
        .and_then(|text| serde_json::from_str(text).map_err(|e| e.to_string()))
    {
        Ok(request) => request,
        Err(e) => return out(failure("invalid_params", &e)),
    };
    let mut reply = serde_json::to_value(run::plan(&request)).unwrap_or_default();
    reply["ok"] = json!(true);
    out(reply)
}

#[derive(Debug, Deserialize)]
struct CalloutRequest {
    items: Vec<callout::Item>,
    #[serde(default)]
    text: Vec<Rect>,
    #[serde(default)]
    prose: Vec<Rect>,
    bounds: Rect,
    tail: f64,
    spacing: f64,
    #[serde(default)]
    inset: f64,
    #[serde(default)]
    drift: f64,
}

/// Places one pane's labels (Homerow-style callouts). `request_json`:
///
/// `{"items":[{"anchor":[x,y,w,h],"rects":[[x,y,w,h]],"prose":false,"size":[w,h]}],
///   "text":[[x,y,w,h]],"prose":[[x,y,w,h]],"bounds":[x,y,w,h],"tail":4,"spacing":2,
///   "inset":7,"drift":4}`
///
/// in the overlay's points, y down: each item's first cell, its text's
/// rects, whether it's a paragraph and its pill's size; every run of
/// non-blank cells; the unlabelled paragraphs' rects, kept off before other
/// items' text; where pills may go; the tail's length, the gap kept,
/// how far a tail's foot keeps from the pill's ends and may stray from the
/// cell's middle. Returns `{"ok":true,"callouts":[{"frame":[…],"tip":[x,y],
/// "base":[x,y],"side":"below","nudged":false,"relaxed":0,"clash":false}]}`,
/// one per item, or `{"ok":false,…}`. Free with `azj_string_free`.
///
/// # Safety
/// `request_json` must be a valid C string or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_hints_place_callouts(request_json: *const c_char) -> *mut c_char {
    let request: CalloutRequest = match unsafe { arg(request_json) }
        .ok_or("request isn't a UTF-8 C string".to_owned())
        .and_then(|text| serde_json::from_str(text).map_err(|e| e.to_string()))
    {
        Ok(request) => request,
        Err(e) => return out(failure("invalid_params", &e)),
    };
    let metrics = Metrics {
        bounds: request.bounds,
        tail: request.tail,
        spacing: request.spacing,
        inset: request.inset,
        drift: request.drift,
    };
    let callouts = callout::place_items(&request.items, &request.text, &request.prose, metrics);
    out(json!({"ok": true, "callouts": callouts}))
}

/// The current `[hints]` settings, as JSON:
/// `{"row_classifier":true,"run_target":"reuse"}`.
/// Follows config reloads. Free with `azj_string_free`; NULL on a bad handle.
///
/// # Safety
/// `core` must come from `azj_core_new*` and not be freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_hints(core: *const AzjCore) -> *mut c_char {
    let Some(app) = (unsafe { core.as_ref() }).and_then(|c| c.app.as_ref()) else {
        return std::ptr::null_mut();
    };
    let hints = app.config().hints;
    out(json!({
        "row_classifier": hints.row_classifier,
        "run_target": hints.run_target.as_str(),
    }))
}

#[cfg(test)]
mod tests {
    use std::ffi::{CStr, CString};
    use std::time::Instant;

    use super::*;

    fn take(reply: *mut c_char) -> Value {
        let value =
            serde_json::from_str(unsafe { CStr::from_ptr(reply) }.to_str().unwrap()).unwrap();
        unsafe { crate::azj_string_free(reply) };
        value
    }

    fn call(request: &str) -> (Value, *mut AzjHintsPane) {
        let request = CString::new(request).unwrap();
        let mut pane = std::ptr::null_mut();
        let reply = unsafe { azj_hints_detect(request.as_ptr(), &mut pane) };
        (take(reply), pane)
    }

    fn range(a: (u32, u32), b: (u32, u32)) -> AzjHintRange {
        AzjHintRange {
            start_row: a.0,
            start_offset: a.1,
            end_row: b.0,
            end_offset: b.1,
        }
    }

    #[test]
    fn detect_round_trip() {
        let (reply, pane) = call(
            r#"{"rows":["$ cargo test","test result: ok. 3 passed","","$ ls src/"],
                "columns":80,"vt":"$ \u001b[32mcargo\u001b[0m test","row_classifier":true}"#,
        );
        assert_eq!(reply["ok"], true);
        let items = reply["items"].as_array().unwrap();
        let described: Vec<String> = items
            .iter()
            .map(|i| {
                format!(
                    "{} {} {}",
                    i["kind"].as_str().unwrap(),
                    i["value"].as_str().unwrap(),
                    i["spans"]
                )
            })
            .collect();
        assert_eq!(
            described,
            [
                "line $ cargo test [[0,0,12]]",
                "command cargo test [[0,2,10]]",
                "line test result: ok. 3 passed [[1,0,25]]",
                "line $ ls src/ [[3,0,9]]",
                "command ls src/ [[3,2,7]]",
                "path src/ [[3,5,4]]"
            ],
            "every non-blank line is an item too, before the items on it"
        );
        assert_eq!(reply["extents"][2], Value::Null);
        assert_eq!(reply["extents"][0], json!([0, 12]));
        assert_eq!(reply["runs"][1], json!([0, 4, 5, 7, 13, 3, 17, 1, 19, 6]));
        assert!(!pane.is_null());

        // Selection keys through the handle.
        let mut unit = AzjHintUnit {
            range: range((0, 0), (0, 0)),
            level: 0,
            entry: -1,
        };
        let path = range((3, 5), (3, 9));
        assert!(unsafe { azj_hints_pane_grow(pane, path, &mut unit) });
        assert_eq!(
            (unit.level, unit.entry, unit.range),
            (0, 4, range((3, 2), (3, 9))),
            "a path grows to its command"
        );
        assert!(unsafe { azj_hints_pane_grow(pane, unit.range, &mut unit) });
        assert_eq!((unit.level, unit.entry), (2, 3), "then its line");
        assert!(unsafe { azj_hints_pane_shrink(pane, range((3, 2), (3, 9)), 0, &mut unit) });
        assert_eq!(unit.entry, 5, "shrinks to the path inside");
        let from = AzjHintUnit {
            range: range((0, 2), (0, 12)),
            level: 0,
            entry: 1,
        };
        assert!(unsafe { azj_hints_pane_neighbour(pane, from, 3, &mut unit) });
        assert_eq!(unit.entry, 4, "↓ to the next row's command");
        let line = AzjHintUnit {
            range: range((0, 0), (0, 12)),
            level: 2,
            entry: 0,
        };
        assert!(unsafe { azj_hints_pane_neighbour(pane, line, 3, &mut unit) });
        assert_eq!(
            (unit.level, unit.entry),
            (2, 2),
            "a line's ↓: the next line"
        );
        assert!(
            !unsafe { azj_hints_pane_neighbour(pane, from, 2, &mut unit) },
            "nothing above"
        );
        assert_eq!(
            unsafe { azj_hints_pane_targets(pane, 2) },
            3,
            "three non-blank lines"
        );

        // Choosing: the focus ring among the shown items (the commands).
        let shown = [1u32, 4];
        let mut index = u32::MAX;
        assert!(unsafe { azj_hints_pane_focus(pane, shown.as_ptr(), 2, -1, 4, 0, 2, &mut index) });
        assert_eq!(index, 4, "↑ from the cursor: the closest command above");
        assert!(unsafe { azj_hints_pane_focus(pane, shown.as_ptr(), 2, 4, -1, 0, 2, &mut index) });
        assert_eq!(index, 1, "↑ again, past the rows between");
        assert!(!unsafe { azj_hints_pane_focus(pane, shown.as_ptr(), 2, 1, -1, 0, 2, &mut index) });
        assert!(!unsafe { azj_hints_pane_focus(pane, shown.as_ptr(), 2, 1, -1, 0, 9, &mut index) });
        assert!(unsafe { azj_hints_pane_nearest(pane, [1u32].as_ptr(), 1, 5, &mut index) });
        assert_eq!(index, 1, "the path filtered out: the nearest command left");
        assert!(!unsafe { azj_hints_pane_nearest(pane, std::ptr::null(), 0, 2, &mut index) });
        let text = unsafe { azj_hints_pane_text(pane, range((0, 2), (1, 4))) };
        assert_eq!(
            unsafe { CStr::from_ptr(text) }.to_str().unwrap(),
            "cargo test\ntest"
        );
        unsafe { crate::azj_string_free(text) };

        // Labels across panes, after the shell drops an item it can't place.
        let (_, other) = call(r#"{"rows":["$ git status","$ git status"]}"#);
        unsafe { azj_hints_pane_keep(pane, [0u32, 1, 5].as_ptr(), 3) };
        let panes = [pane as *const AzjHintsPane, other as *const AzjHintsPane];
        let labels = take(unsafe { azj_hints_labels(panes.as_ptr(), 2) });
        assert_eq!(
            labels["labels"],
            json!(["la", "ca", "p", "ls", "", "ld", "cs"]),
            "one label per repeated value in a pane, but one per line"
        );
        unsafe {
            azj_hints_pane_free(pane);
            azj_hints_pane_free(other);
        }
    }

    #[test]
    fn run_plan() {
        let request = CString::new(
            r#"{"value":"ls","cwd":"/w","machine":"local","current":"a",
                "panes":[{"tile":"a","rect":[0,0,1,1],"machine":"local","idle":false},
                         {"tile":"b","rect":[1,0,1,1],"machine":"local","idle":true,"cwd":"/w"}]}"#,
        )
        .unwrap();
        let reply = take(unsafe { azj_hints_run_plan(request.as_ptr()) });
        assert_eq!(
            reply,
            json!({"ok": true, "target": "reuse", "tile": "b", "line": "ls"})
        );
        let bad = CString::new("{}").unwrap();
        let reply = take(unsafe { azj_hints_run_plan(bad.as_ptr()) });
        assert_eq!(reply["ok"], json!(false));
    }

    #[test]
    fn run_line_and_callouts() {
        let line = CString::new("cat <<EOF\nx\nEOF").unwrap();
        let reply = unsafe { azj_hints_run_line(line.as_ptr()) };
        assert!(
            unsafe { CStr::from_ptr(reply) }
                .to_str()
                .unwrap()
                .starts_with("/bin/sh -c '")
        );
        unsafe { crate::azj_string_free(reply) };
        let request = CString::new(
            r#"{"items":[{"anchor":[50,20,10,20],"rects":[[50,20,40,20]],"size":[18,16]}],
                "bounds":[0,0,400,100],"tail":4,"spacing":2,"inset":7,"drift":5}"#,
        )
        .unwrap();
        let reply = take(unsafe { azj_hints_place_callouts(request.as_ptr()) });
        assert_eq!(
            reply["callouts"][0]["frame"],
            json!([46.0, 44.0, 18.0, 16.0])
        );
        assert_eq!(reply["callouts"][0]["side"], "below");
        assert_eq!(reply["callouts"][0]["base"], json!([55.0, 44.0]));
    }

    #[test]
    fn bad_requests_fail_cleanly() {
        assert_eq!(call("not json").0["ok"], false);
        let mut pane = std::ptr::null_mut();
        let reply = unsafe { azj_hints_detect(std::ptr::null(), &mut pane) };
        assert!(pane.is_null());
        assert!(
            unsafe { CStr::from_ptr(reply) }
                .to_str()
                .unwrap()
                .contains("\"ok\":false")
        );
        unsafe { crate::azj_string_free(reply) };
        assert!(!unsafe {
            azj_hints_pane_grow(
                std::ptr::null(),
                range((0, 0), (0, 1)),
                std::ptr::null_mut(),
            )
        });
        let reply = take(unsafe { azj_hints_place_callouts(std::ptr::null()) });
        assert_eq!(reply["ok"], false);
    }

    /// ⌘S on a full 60×120 screen through the C ABI, JSON both ways, as
    /// the shell calls it. Median of runs.
    #[test]
    #[ignore = "timing: run with --release --ignored --nocapture"]
    fn full_screen_round_trip_timing() {
        let corpus = include_str!("../../hints/data/hints-corpus.txt");
        let rows: Vec<String> = corpus
            .lines()
            .filter(|l| !l.starts_with("@@") && !l.starts_with('#') && !l.contains("\\e"))
            .map(|l| l.chars().take(120).collect())
            .collect();
        let cwd = std::env::temp_dir();
        let mut samples = Vec::new();
        for _ in 0..15 {
            for screen in rows.chunks(60) {
                let request = json!({"rows": screen, "columns": 120, "cwd": cwd, "local": true, "branch": "main",
                                     "row_classifier": true})
                .to_string();
                let start = Instant::now();
                let request = CString::new(request).unwrap();
                let mut pane = std::ptr::null_mut();
                let reply = unsafe { azj_hints_detect(request.as_ptr(), &mut pane) };
                let value = take(reply);
                std::hint::black_box(value);
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
                unsafe { azj_hints_pane_free(pane) };
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "full screen through the C ABI: {:.3} ms (median of {})",
            samples[samples.len() / 2],
            samples.len()
        );
    }
}
