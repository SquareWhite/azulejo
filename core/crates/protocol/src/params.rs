//! Refuses params a method doesn't declare.
//!
//! serde ignores unknown fields, so `layout.split` with a top-level `kind`
//! and `url` (instead of `content`) used to make a terminal and reply OK.
//! Rather than mark every type `deny_unknown_fields` (which can't work for
//! `TileContent`: a `Tile` flattens it), commands are checked against the
//! API schema before they're parsed. That covers nested objects too.

use std::sync::OnceLock;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};

fn schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(crate::api_schema)
}

/// Checks `params` against `method`'s declared params. The error names the
/// first unknown one as a dotted path (`content.url`).
pub(crate) fn check(method: &str, params: Option<&Value>) -> Result<(), Error> {
    let defs = schema()["$defs"].as_object().expect("schema has $defs");
    let Some(variant) = defs["Command"]["oneOf"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["properties"]["method"]["const"] == method)
    else {
        return Ok(());
    };
    let refuse = |param: String| {
        Error::new(
            ErrorCode::InvalidParams,
            format!("unknown param `{param}` for `{method}`"),
        )
        .with_data(serde_json::json!({ "param": param }))
    };
    match (variant["properties"].get("params"), params) {
        (_, None | Some(Value::Null)) => Ok(()),
        (None, Some(Value::Object(given))) => match given.keys().next() {
            Some(first) => Err(refuse(first.clone())),
            None => Ok(()),
        },
        (None, Some(_)) => Ok(()),
        (Some(declared), Some(given)) => match unknown(declared, given, defs, "") {
            Some(param) => Err(refuse(param)),
            None => Ok(()),
        },
    }
}

/// The first key in `value` that `schema` doesn't declare, as a path.
fn unknown(schema: &Value, value: &Value, defs: &Map<String, Value>, at: &str) -> Option<String> {
    let schema = deref(schema, defs);
    let branches = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array);
    if let Some(branches) = branches {
        // A union: fine when any branch that could be meant takes it all.
        // When none could (a bad `kind`, say), parsing says so instead.
        let mut first = None;
        for branch in branches {
            let branch = deref(branch, defs);
            if !could_be(branch, value) {
                continue;
            }
            // A branch with nothing unknown settles it.
            let param = unknown(branch, value, defs, at)?;
            first.get_or_insert(param);
        }
        return first;
    }
    let path = |key: &str| {
        if at.is_empty() {
            key.to_owned()
        } else {
            format!("{at}.{key}")
        }
    };
    match value {
        Value::Object(map) => {
            let extra = schema.get("additionalProperties");
            let props = schema.get("properties").and_then(Value::as_object);
            for (key, v) in map {
                let found = match (props.and_then(|p| p.get(key)), extra) {
                    (Some(declared), _) => unknown(declared, v, defs, &path(key)),
                    // A map (`env`): every value has the same schema.
                    (None, Some(each)) if each.is_object() => unknown(each, v, defs, &path(key)),
                    (None, Some(Value::Bool(true))) => None,
                    // A struct, or a map whose values aren't described.
                    (None, _) if props.is_some() => Some(path(key)),
                    (None, _) => None,
                };
                if found.is_some() {
                    return found;
                }
            }
            None
        }
        Value::Array(items) => {
            let each = schema.get("items")?;
            items
                .iter()
                .enumerate()
                .find_map(|(i, v)| unknown(each, v, defs, &format!("{at}[{i}]")))
        }
        _ => None,
    }
}

fn deref<'a>(schema: &'a Value, defs: &'a Map<String, Value>) -> &'a Value {
    let mut schema = schema;
    while let Some(name) = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix("#/$defs/"))
    {
        match defs.get(name) {
            Some(target) => schema = target,
            None => break,
        }
    }
    schema
}

/// Whether `value` could be meant as `branch` of a union: objects only match
/// object branches whose tags (`const` properties) agree.
fn could_be(branch: &Value, value: &Value) -> bool {
    let Value::Object(map) = value else {
        return true;
    };
    let objecty = branch.get("properties").is_some()
        || branch.get("oneOf").is_some()
        || branch.get("anyOf").is_some()
        || match branch.get("type") {
            Some(Value::String(t)) => t == "object",
            Some(Value::Array(ts)) => ts.iter().any(|t| t == "object"),
            _ => false,
        };
    if !objecty {
        return false;
    }
    branch
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(k, p)| p.get("const").map(|c| (k, c)))
        .all(|(k, c)| map.get(k).is_none_or(|v| v == c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn refused(method: &str, params: Value) -> Option<String> {
        check(method, Some(&params))
            .err()
            .map(|e| e.data.unwrap()["param"].as_str().unwrap().to_owned())
    }

    #[test]
    fn declared_params_pass() {
        assert_eq!(refused("layout.split", json!({"direction": "right"})), None);
        assert_eq!(
            refused(
                "layout.split",
                json!({"direction": "right", "tile": null, "content": {"kind": "browser", "url": "http://x/"}})
            ),
            None
        );
        assert_eq!(
            refused(
                "tab.create",
                json!({"content": {"kind": "terminal", "cwd": "/", "env": {"A": "1"}, "command": ["ls"]}})
            ),
            None
        );
        assert_eq!(
            refused(
                "window.set_frame",
                json!({"window": "win_x", "frame": {"x": 0, "y": 0, "width": 1, "height": 1}})
            ),
            None
        );
        assert_eq!(
            refused("events.subscribe", json!({"filter": ["tile."]})),
            None
        );
        assert_eq!(refused("core.ping", json!({})), None);
    }

    #[test]
    fn unknown_params_are_named() {
        assert_eq!(
            refused(
                "layout.split",
                json!({"direction": "right", "kind": "browser", "url": "http://x/"})
            ),
            Some("kind".into())
        );
        assert_eq!(
            refused(
                "layout.split",
                json!({"direction": "right", "content": {"kind": "browser", "path": "/x"}})
            ),
            Some("content.path".into())
        );
        assert_eq!(
            refused(
                "window.set_frame",
                json!({"window": "win_x", "frame": {"x": 0, "y": 0, "width": 1, "height": 1, "z": 2}})
            ),
            Some("frame.z".into())
        );
        assert_eq!(
            refused("files.open", json!({"path": "a", "targte": "terminal"})),
            Some("targte".into())
        );
        assert_eq!(
            refused("layout.get", json!({"tab": "tab_x"})),
            Some("tab".into())
        );
    }

    #[test]
    fn a_bad_tag_is_left_to_parsing() {
        assert_eq!(
            refused(
                "layout.split",
                json!({"direction": "right", "content": {"kind": "spreadsheet"}})
            ),
            None
        );
    }
}
