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
}
