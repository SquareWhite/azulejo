//! Human-readable output for replies. `--json` skips all of this.

use std::fmt::Write as _;

use serde_json::Value;

/// Formats a reply for people. Empty for a bare `ok`.
pub fn human(result: &Value) -> String {
    let mut out = String::new();
    match result.get("type").and_then(Value::as_str) {
        Some("ok") => {}
        Some("pong") => {
            let _ = writeln!(out, "pong (API v{})", result["api_version"]);
        }
        Some("version") => {
            let _ = writeln!(
                out,
                "azulejo {} (API v{})",
                result["version"].as_str().unwrap_or("?"),
                result["api_version"]
            );
        }
        Some("created") => {
            for key in ["window", "workspace", "tab", "tile"] {
                if let Some(id) = result.get(key).and_then(Value::as_str) {
                    let _ = writeln!(out, "{key} {id}");
                }
            }
        }
        Some("tiles") => {
            for tile in result["tiles"].as_array().into_iter().flatten() {
                out.push_str(&tile_line(tile));
                out.push('\n');
            }
        }
        Some("tile") => {
            for key in [
                "id",
                "kind",
                "title",
                "window",
                "workspace",
                "tab",
                "focused",
            ] {
                if let Some(v) = result.get(key) {
                    let _ = writeln!(out, "{key}: {}", scalar(v));
                }
            }
            for (key, v) in result.as_object().into_iter().flatten() {
                let shown = [
                    "type",
                    "id",
                    "kind",
                    "title",
                    "window",
                    "workspace",
                    "tab",
                    "focused",
                ];
                if !shown.contains(&key.as_str()) {
                    let _ = writeln!(out, "{key}: {}", scalar(v));
                }
            }
        }
        Some("text") => {
            out.push_str(result["text"].as_str().unwrap_or(""));
            if !out.ends_with('\n') && !out.is_empty() {
                out.push('\n');
            }
        }
        Some("matches") => {
            // grep-like: `line: text`, with where it sits when located.
            for m in result["matches"].as_array().into_iter().flatten() {
                let at = match (m["row"].as_u64(), m["offset_from_bottom"].as_u64()) {
                    (Some(row), Some(offset)) => format!(" (row {row}, scroll {offset})"),
                    _ => String::new(),
                };
                let _ = writeln!(
                    out,
                    "{}{at}: {}",
                    scalar(&m["line"]),
                    m["text"].as_str().unwrap_or("")
                );
            }
            if result["truncated"].as_bool() == Some(true) {
                out.push_str("(history cut short: older lines weren't searched)\n");
            }
        }
        Some("layout_file") => match result["path"].as_str() {
            Some(path) => {
                let _ = writeln!(out, "wrote {path}");
            }
            None => out.push_str(result["toml"].as_str().unwrap_or("")),
        },
        Some("commands") => {
            for c in result["commands"].as_array().into_iter().flatten() {
                let chords = c["chords"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "{:<24} {:<32} {}",
                    scalar(&c["id"]),
                    scalar(&c["title"]),
                    chords
                );
            }
        }
        Some("keys") => render_keys(result, &mut out),
        Some("layout") => {
            let _ = writeln!(
                out,
                "tab {} (workspace {})",
                scalar(&result["tab"]),
                scalar(&result["workspace"])
            );
            let marks = Marks {
                focused: result["focused"].as_str(),
                zoomed: result["zoomed"].as_str(),
                tiles: None,
            };
            render_node(&result["tree"], &marks, "", &mut out);
        }
        Some("session") => render_session(&result["session"], &mut out),
        _ => {
            out = serde_json::to_string_pretty(result).unwrap_or_default();
            out.push('\n');
        }
    }
    out
}

/// `config.keys`: one row per action and context, user overrides marked.
fn render_keys(result: &Value, out: &mut String) {
    let chords = |v: &Value| {
        let list: Vec<&str> = v
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if list.is_empty() {
            "-".to_owned()
        } else {
            list.join(", ")
        }
    };
    let rows: Vec<[String; 5]> = result["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|k| {
            [
                scalar(&k["action"]),
                scalar(&k["context"]),
                chords(&k["default"]),
                chords(&k["current"]),
                scalar(&k["source"]),
            ]
        })
        .collect();
    let header = ["ACTION", "CONTEXT", "DEFAULT", "CURRENT", "SOURCE"].map(str::to_owned);
    let mut width = [0usize; 4];
    for row in std::iter::once(&header).chain(&rows) {
        for (w, cell) in width.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let path = result["path"].as_str().unwrap_or("none, defaults only");
    let _ = writeln!(
        out,
        "{} keymap; config: {path}{}",
        scalar(&result["platform"]),
        result["prefix"]
            .as_str()
            .map(|p| format!("; prefix {p}"))
            .unwrap_or_default()
    );
    for row in std::iter::once(&header).chain(&rows) {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            if i < 4 {
                let _ = write!(line, "{cell:<w$}  ", w = width[i]);
            } else {
                line.push_str(cell);
            }
        }
        let _ = writeln!(out, "{}", line.trim_end());
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "-".into(),
        other => other.to_string(),
    }
}

/// What a tile shows, in a few words.
fn tile_detail(tile: &Value) -> String {
    let field = |k: &str| tile.get(k).and_then(Value::as_str);
    match field("kind") {
        Some("terminal") => {
            let mut parts = Vec::new();
            if let Some(cmd) = tile.get("command").and_then(Value::as_array) {
                let cmd: Vec<&str> = cmd.iter().filter_map(Value::as_str).collect();
                parts.push(cmd.join(" "));
            }
            if let Some(cwd) = field("cwd") {
                parts.push(cwd.to_owned());
            }
            if let Some(pane) = tile
                .get("herdr")
                .and_then(|h| h.get("pane_id"))
                .and_then(Value::as_str)
            {
                parts.push(format!("herdr {pane}"));
            }
            parts.join("  ")
        }
        Some("browser") => field("url").unwrap_or("").to_owned(),
        Some("preview") => field("path").unwrap_or("").to_owned(),
        Some("files") => field("root").unwrap_or("").to_owned(),
        _ => String::new(),
    }
}

fn tile_line(tile: &Value) -> String {
    let focused = tile.get("focused").and_then(Value::as_bool) == Some(true);
    let mut line = format!(
        "{} {}  {:<8}",
        if focused { "*" } else { " " },
        scalar(&tile["id"]),
        tile["kind"].as_str().unwrap_or("?"),
    );
    let detail = tile_detail(tile);
    if !detail.is_empty() {
        let _ = write!(line, "  {detail}");
    }
    if let Some(title) = tile.get("title").and_then(Value::as_str) {
        let _ = write!(line, "  \"{title}\"");
    }
    line.trim_end().to_owned()
}

struct Marks<'a> {
    focused: Option<&'a str>,
    zoomed: Option<&'a str>,
    /// The tab's tiles, for details next to each leaf.
    tiles: Option<&'a [Value]>,
}

fn render_node(node: &Value, marks: &Marks, indent: &str, out: &mut String) {
    match node.get("type").and_then(Value::as_str) {
        Some("leaf") => {
            let id = node["id"].as_str().unwrap_or("?");
            let mut line = id.to_owned();
            if let Some(tile) = marks
                .tiles
                .and_then(|tiles| tiles.iter().find(|t| t["id"] == id))
            {
                let _ = write!(line, "  {}", tile["kind"].as_str().unwrap_or("?"));
                let detail = tile_detail(tile);
                if !detail.is_empty() {
                    let _ = write!(line, "  {detail}");
                }
            }
            if marks.focused == Some(id) {
                line.push_str("  *");
            }
            if marks.zoomed == Some(id) {
                line.push_str("  (zoomed)");
            }
            let _ = writeln!(out, "{line}");
        }
        Some("split") => {
            let ratio = node["ratio"].as_f64().unwrap_or(0.5);
            let _ = writeln!(
                out,
                "{} {:.0}%",
                node["axis"].as_str().unwrap_or("split"),
                ratio * 100.0
            );
            for (child, last) in [(&node["a"], false), (&node["b"], true)] {
                let (branch, next) = if last {
                    ("└─ ", "   ")
                } else {
                    ("├─ ", "│  ")
                };
                out.push_str(indent);
                out.push_str(branch);
                render_node(child, marks, &format!("{indent}{next}"), out);
            }
        }
        _ => {
            let _ = writeln!(out, "{node}");
        }
    }
}

fn render_session(session: &Value, out: &mut String) {
    let focused_window = session["focused_window"].as_str();
    for window in session["windows"].as_array().into_iter().flatten() {
        let id = window["id"].as_str().unwrap_or("?");
        let mark = if focused_window == Some(id) {
            "  *"
        } else {
            ""
        };
        let _ = writeln!(out, "window {id}{mark}");
        let active_ws = window["active_workspace"].as_str();
        for ws in window["workspaces"].as_array().into_iter().flatten() {
            let wid = ws["id"].as_str().unwrap_or("?");
            let mark = if active_ws == Some(wid) { "  *" } else { "" };
            let mut line = format!(
                "  workspace {wid} \"{}\"",
                ws["name"].as_str().unwrap_or("")
            );
            if let Some(h) = ws.get("herdr_workspace").and_then(Value::as_str) {
                let _ = write!(line, "  herdr {h}");
            }
            let _ = writeln!(out, "{line}{mark}");
            let active_tab = ws["active_tab"].as_str();
            for tab in ws["tabs"].as_array().into_iter().flatten() {
                let tid = tab["id"].as_str().unwrap_or("?");
                let mark = if active_tab == Some(tid) { "  *" } else { "" };
                let name = tab
                    .get("name")
                    .and_then(Value::as_str)
                    .map(|n| format!(" \"{n}\""))
                    .unwrap_or_default();
                let _ = writeln!(out, "    tab {tid}{name}{mark}");
                let marks = Marks {
                    focused: tab["focused"].as_str(),
                    zoomed: tab["zoomed"].as_str(),
                    tiles: tab["tiles"].as_array().map(Vec::as_slice),
                };
                out.push_str("      ");
                render_node(&tab["root"], &marks, "      ", out);
            }
        }
    }
}

/// One event notification as a line for people: the event's name, then the
/// IDs and values that say what it's about. Lag notices say how many events
/// were skipped.
pub fn event_line(message: &Value) -> String {
    let params = &message["params"];
    if message["method"] == "events.lagged" {
        return format!("(skipped {} events: reading too slowly)", params["dropped"]);
    }
    let name = params["event"].as_str().unwrap_or("?");
    let data = &params["data"];
    let mut parts: Vec<String> = Vec::new();
    let mut push = |key: &str, v: &Value| {
        if !v.is_null() {
            parts.push(format!("{key}={}", scalar(v)));
        }
    };
    match name {
        "tile.created" | "tile.updated" | "tile.closed" => {
            push("tile", &data["tile"]["id"]);
            push("kind", &data["tile"]["kind"]);
            push("title", &data["tile"]["title"]);
            push("tab", &data["tab"]);
            if data["kill"] == true {
                parts.push("kill".into());
            }
        }
        "layout.changed" => {
            push("tab", &data["tab"]);
            push("workspace", &data["workspace"]);
            push("focused", &data["focused"]);
            push("zoomed", &data["zoomed"]);
        }
        "agent.state_changed" => {
            push("tile", &data["tile"]);
            push("state", &data["state"]);
            push("previous", &data["previous"]);
            push("agent", &data["agent"]);
        }
        "preview.updated" => {
            push("tile", &data["tile"]);
            push("kind", &data["kind"]);
            push("path", &data["path"]);
        }
        _ => {
            for (key, v) in data.as_object().into_iter().flatten() {
                match v {
                    Value::Object(inner) => {
                        if let Some(id) = inner.get("id") {
                            push(key, id);
                        }
                    }
                    Value::Array(_) => {}
                    other => push(key, other),
                }
            }
        }
    }
    if parts.is_empty() {
        name.to_owned()
    } else {
        format!("{name} {}", parts.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn layout_renders_as_a_tree() {
        let text = human(&json!({
            "type": "layout", "window": "win_1", "workspace": "ws_1", "tab": "tab_1",
            "focused": "tile_b",
            "tree": {"type": "split", "axis": "horizontal", "ratio": 0.5,
                "a": {"type": "leaf", "id": "tile_a"},
                "b": {"type": "split", "axis": "vertical", "ratio": 0.25,
                    "a": {"type": "leaf", "id": "tile_b"},
                    "b": {"type": "leaf", "id": "tile_c"}}}
        }));
        assert_eq!(
            text,
            "tab tab_1 (workspace ws_1)\n\
             horizontal 50%\n\
             ├─ tile_a\n\
             └─ vertical 25%\n\
             \u{20}  ├─ tile_b  *\n\
             \u{20}  └─ tile_c\n"
        );
    }

    #[test]
    fn created_lists_ids() {
        assert_eq!(
            human(&json!({"type": "created", "tab": "tab_1", "tile": "tile_1"})),
            "tab tab_1\ntile tile_1\n"
        );
        assert_eq!(human(&json!({"type": "ok"})), "");
    }

    #[test]
    fn tiles_show_what_they_hold() {
        let text = human(&json!({"type": "tiles", "tiles": [
            {"id": "tile_1", "kind": "browser", "url": "http://localhost:3000", "focused": true,
             "window": "win_1", "workspace": "ws_1", "tab": "tab_1"},
            {"id": "tile_2", "kind": "terminal", "herdr": {"pane_id": "w1:p2", "terminal_id": "t"},
             "focused": false, "window": "win_1", "workspace": "ws_1", "tab": "tab_1"}
        ]}));
        assert_eq!(
            text,
            "* tile_1  browser   http://localhost:3000\n  tile_2  terminal  herdr w1:p2\n"
        );
    }

    #[test]
    fn matches_read_like_grep() {
        let text = human(&json!({"type": "matches", "truncated": true, "matches": [
            {"line": 3, "text": "error: boom"},
            {"line": 9, "text": "error again", "row": 12, "offset_from_bottom": 40}
        ]}));
        assert_eq!(
            text,
            "3: error: boom\n9 (row 12, scroll 40): error again\n\
             (history cut short: older lines weren't searched)\n"
        );
    }
}
