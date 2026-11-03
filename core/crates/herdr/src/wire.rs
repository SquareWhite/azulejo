//! herdr's NDJSON wire format. herdr serves one request per connection: it
//! writes one reply line and closes, except for `events.subscribe`, which
//! keeps streaming.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};

/// herdr method names. Nothing outside this crate may spell these.
pub(crate) mod method {
    pub const PING: &str = "ping";
    pub const SERVER_STOP: &str = "server.stop";
    pub const SESSION_SNAPSHOT: &str = "session.snapshot";
    pub const WORKSPACE_CREATE: &str = "workspace.create";
    pub const WORKSPACE_GET: &str = "workspace.get";
    pub const WORKSPACE_LIST: &str = "workspace.list";
    pub const WORKSPACE_CLOSE: &str = "workspace.close";
    pub const TAB_CREATE: &str = "tab.create";
    pub const TAB_RENAME: &str = "tab.rename";
    pub const PANE_GET: &str = "pane.get";
    pub const PANE_LIST: &str = "pane.list";
    pub const PANE_CLOSE: &str = "pane.close";
    pub const PANE_SEND_TEXT: &str = "pane.send_text";
    pub const PANE_SEND_KEYS: &str = "pane.send_keys";
    pub const PANE_READ: &str = "pane.read";
    pub const PANE_SCROLL: &str = "pane.scroll";
    pub const LAYOUT_EXPORT: &str = "layout.export";
    pub const EVENTS_SUBSCRIBE: &str = "events.subscribe";
}

#[derive(Serialize)]
pub(crate) struct Request<'a, P: Serialize> {
    pub id: &'a str,
    pub method: &'a str,
    pub params: P,
}

pub(crate) fn encode<P: Serialize>(id: &str, method: &str, params: P) -> Result<String> {
    let mut line = serde_json::to_string(&Request { id, method, params })
        .map_err(|e| Error::Protocol(format!("encoding {method}: {e}")))?;
    line.push('\n');
    Ok(line)
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<ErrorBody>,
}

/// Decode one reply line into its `result` object.
///
/// herdr answers unparseable requests with `"id": ""`, so an error with an
/// empty ID is accepted for any request.
pub(crate) fn decode_reply(line: &str, expected_id: &str) -> Result<Value> {
    let reply: Reply = serde_json::from_str(line)
        .map_err(|e| Error::Protocol(format!("{e}: {}", truncate(line))))?;
    let id = reply.id.unwrap_or_default();
    if let Some(err) = reply.error
        && (id == expected_id || id.is_empty())
    {
        return Err(Error::Server {
            code: err.code,
            message: err.message,
        });
    }
    if id != expected_id {
        return Err(Error::Protocol(format!(
            "reply for `{id}` while waiting for `{expected_id}`"
        )));
    }
    reply
        .result
        .ok_or_else(|| Error::Protocol(format!("reply without result: {}", truncate(line))))
}

/// Pull `field` out of a result object and parse it.
pub(crate) fn field<T: serde::de::DeserializeOwned>(result: &Value, field: &str) -> Result<T> {
    let v = result
        .get(field)
        .ok_or_else(|| Error::Protocol(format!("result has no `{field}`: {}", short(result))))?;
    serde_json::from_value(v.clone())
        .map_err(|e| Error::Protocol(format!("parsing `{field}`: {e}")))
}

pub(crate) fn short(v: &Value) -> String {
    truncate(&v.to_string()).to_owned()
}

fn truncate(s: &str) -> &str {
    match s.char_indices().nth(300) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_one_line() {
        let line = encode(
            "7",
            method::PANE_GET,
            serde_json::json!({"pane_id": "w1:p1"}),
        )
        .unwrap();
        assert_eq!(
            line,
            "{\"id\":\"7\",\"method\":\"pane.get\",\"params\":{\"pane_id\":\"w1:p1\"}}\n"
        );
    }

    #[test]
    fn decodes_success() {
        let v = decode_reply(r#"{"id":"1","result":{"type":"ok"}}"#, "1").unwrap();
        assert_eq!(v["type"], "ok");
    }

    #[test]
    fn decodes_server_error() {
        let e = decode_reply(
            r#"{"id":"7","error":{"code":"pane_not_found","message":"pane w9:p9 not found"}}"#,
            "7",
        )
        .unwrap_err();
        assert!(e.is_not_found());
        assert_eq!(e.code(), Some("pane_not_found"));
    }

    #[test]
    fn accepts_error_with_empty_id() {
        let e = decode_reply(
            r#"{"id":"","error":{"code":"invalid_request","message":"unknown variant"}}"#,
            "9",
        )
        .unwrap_err();
        assert_eq!(e.code(), Some("invalid_request"));
    }

    #[test]
    fn rejects_mismatched_id() {
        let e = decode_reply(r#"{"id":"2","result":{"type":"ok"}}"#, "1").unwrap_err();
        assert!(matches!(e, Error::Protocol(_)));
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(
            decode_reply("not json", "1").unwrap_err(),
            Error::Protocol(_)
        ));
    }
}
