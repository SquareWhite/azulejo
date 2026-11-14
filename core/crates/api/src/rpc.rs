//! JSON-RPC 2.0 framing: request parsing, server-level methods, responses.

use azulejo_protocol::{API_VERSION, Command, Error, ErrorCode};
use serde_json::{Map, Value, json};

use crate::Dispatcher;

/// Methods the server answers itself, without the engine.
pub const SERVER_METHODS: &[&str] = &["core.version", "events.subscribe"];

/// One parsed request.
struct Call {
    /// `None` for a notification, which gets no response.
    id: Option<Value>,
    method: String,
    params: Option<Map<String, Value>>,
}

/// Handles one line of input: a request or a batch. Returns the response line
/// (without its newline), or `None` when nothing needs answering.
pub async fn handle_text(text: &str, dispatcher: &dyn Dispatcher) -> Option<String> {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(e) => {
            let err = Error::new(ErrorCode::ParseError, format!("invalid JSON: {e}"));
            return Some(error_response(Value::Null, &err).to_string());
        }
    };
    match value {
        Value::Array(batch) => {
            if batch.is_empty() {
                let err = Error::new(ErrorCode::InvalidRequest, "empty batch");
                return Some(error_response(Value::Null, &err).to_string());
            }
            let mut responses = Vec::new();
            for item in batch {
                if let Some(response) = handle_value(item, dispatcher).await {
                    responses.push(response);
                }
            }
            (!responses.is_empty()).then(|| Value::Array(responses).to_string())
        }
        value => handle_value(value, dispatcher)
            .await
            .map(|response| response.to_string()),
    }
}

async fn handle_value(value: Value, dispatcher: &dyn Dispatcher) -> Option<Value> {
    let call = match parse_call(value) {
        Ok(call) => call,
        Err((id, err)) => return Some(error_response(id, &err)),
    };
    let result = run(&call, dispatcher).await;
    let id = call.id?;
    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(err) => error_response(id, &err),
    })
}

fn parse_call(value: Value) -> Result<Call, (Value, Error)> {
    let invalid = |id: Value, msg: &str| (id, Error::new(ErrorCode::InvalidRequest, msg));
    let Value::Object(mut map) = value else {
        return Err(invalid(Value::Null, "a request must be a JSON object"));
    };
    let id = match map.remove("id") {
        None => None,
        Some(id @ (Value::Null | Value::String(_) | Value::Number(_))) => Some(id),
        Some(_) => {
            return Err(invalid(
                Value::Null,
                "`id` must be a string, number or null",
            ));
        }
    };
    let reply_id = id.clone().unwrap_or(Value::Null);
    match map.get("jsonrpc") {
        Some(Value::String(v)) if v == "2.0" => {}
        _ => return Err(invalid(reply_id, "`jsonrpc` must be \"2.0\"")),
    }
    let method = match map.remove("method") {
        Some(Value::String(m)) => m,
        _ => return Err(invalid(reply_id, "`method` must be a string")),
    };
    let params = match map.remove("params") {
        None | Some(Value::Null) => None,
        Some(Value::Object(p)) => Some(p),
        Some(_) => {
            let err = Error::new(
                ErrorCode::InvalidParams,
                "`params` must be an object; positional params aren't supported",
            );
            return Err((reply_id, err));
        }
    };
    Ok(Call { id, method, params })
}

async fn run(call: &Call, dispatcher: &dyn Dispatcher) -> Result<Value, Error> {
    match call.method.as_str() {
        "core.version" => {
            let mut version = dispatcher.info();
            version.insert("type".into(), "version".into());
            version.insert("api_version".into(), API_VERSION.into());
            version.insert("version".into(), env!("CARGO_PKG_VERSION").into());
            Ok(Value::Object(version))
        }
        "events.subscribe" => Err(Error::new(
            ErrorCode::Unsupported,
            "event streaming isn't available yet (PLAN step 2.8)",
        )),
        method => {
            let mut request = Map::new();
            request.insert("method".into(), Value::String(method.to_owned()));
            // Leave empty params out so commands without params parse too.
            if let Some(params) = call.params.as_ref().filter(|p| !p.is_empty()) {
                request.insert("params".into(), Value::Object(params.clone()));
            }
            let command = Command::from_json(&Value::Object(request).to_string())?;
            let reply = dispatcher.dispatch(command).await?;
            serde_json::to_value(reply).map_err(|e| {
                Error::new(ErrorCode::Internal, format!("reply didn't serialise: {e}"))
            })
        }
    }
}

fn error_response(id: Value, err: &Error) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": err.code.rpc_code(),
            "message": err.message,
            "data": {"code": err.code},
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DispatchFuture, spawn_engine};
    use azulejo_engine::Engine;
    use azulejo_protocol::Reply;

    async fn call(text: &str) -> Option<Value> {
        let (tx, _thread) = spawn_engine(Engine::new(), |_| {});
        handle_text(text, &tx)
            .await
            .map(|s| serde_json::from_str(&s).unwrap())
    }

    fn error_code(response: &Value) -> (i64, String) {
        let err = &response["error"];
        (
            err["code"].as_i64().unwrap(),
            err["data"]["code"].as_str().unwrap().to_owned(),
        )
    }

    #[tokio::test]
    async fn ping_round_trips() {
        let r = call(r#"{"jsonrpc":"2.0","id":7,"method":"core.ping"}"#)
            .await
            .unwrap();
        assert_eq!(r["id"], 7);
        assert_eq!(r["result"]["type"], "pong");
        assert_eq!(r["result"]["api_version"], API_VERSION);
    }

    #[tokio::test]
    async fn empty_params_are_fine_for_unit_commands() {
        let r = call(r#"{"jsonrpc":"2.0","id":1,"method":"core.ping","params":{}}"#)
            .await
            .unwrap();
        assert_eq!(r["result"]["type"], "pong");
    }

    #[tokio::test]
    async fn version_is_answered_by_the_server() {
        let r = call(r#"{"jsonrpc":"2.0","id":"v","method":"core.version"}"#)
            .await
            .unwrap();
        assert_eq!(r["id"], "v");
        assert_eq!(r["result"]["api_version"], 1);
    }

    #[tokio::test]
    async fn subscribe_is_unsupported_for_now() {
        let r = call(r#"{"jsonrpc":"2.0","id":1,"method":"events.subscribe"}"#)
            .await
            .unwrap();
        assert_eq!(error_code(&r), (-32003, "unsupported".into()));
    }

    #[tokio::test]
    async fn bad_input_maps_to_json_rpc_errors() {
        let cases = [
            ("{not json", -32700, "parse_error"),
            ("[]", -32600, "invalid_request"),
            ("42", -32600, "invalid_request"),
            (
                r#"{"id":1,"method":"core.ping"}"#,
                -32600,
                "invalid_request",
            ),
            (r#"{"jsonrpc":"2.0","id":1}"#, -32600, "invalid_request"),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#,
                -32601,
                "method_not_found",
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"layout.split","params":[1]}"#,
                -32602,
                "invalid_params",
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"layout.split","params":{"direction":"sideways"}}"#,
                -32602,
                "invalid_params",
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"tile.get","params":{"tile":"tile_01JBX3K6Z8V9W0Q1R2S3T4V5W6"}}"#,
                -32001,
                "not_found",
            ),
        ];
        for (text, code, name) in cases {
            let r = call(text).await.unwrap();
            assert_eq!(error_code(&r), (code, name.to_owned()), "{text}");
        }
    }

    #[tokio::test]
    async fn notifications_get_no_response() {
        assert!(
            call(r#"{"jsonrpc":"2.0","method":"core.ping"}"#)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn batches_answer_every_request_but_notifications() {
        let r = call(
            r#"[{"jsonrpc":"2.0","id":1,"method":"core.ping"},
                {"jsonrpc":"2.0","method":"core.ping"},
                {"jsonrpc":"2.0","id":2,"method":"nope"}]"#,
        )
        .await
        .unwrap();
        let r = r.as_array().unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0]["result"]["type"], "pong");
        assert_eq!(r[1]["error"]["code"], -32601);

        assert!(
            call(r#"[{"jsonrpc":"2.0","method":"core.ping"}]"#)
                .await
                .is_none()
        );
    }

    struct Broken;
    impl Dispatcher for Broken {
        fn dispatch(&self, _: Command) -> DispatchFuture {
            Box::pin(async { Ok::<Reply, Error>(Reply::Ok) })
        }
    }

    #[tokio::test]
    async fn version_carries_the_dispatchers_info() {
        let info = Map::from_iter([("herdr_socket".to_owned(), Value::from("/tmp/herdr.sock"))]);
        let dispatcher = crate::WithInfo::new(Broken, info);
        let r = handle_text(
            r#"{"jsonrpc":"2.0","id":1,"method":"core.version"}"#,
            &dispatcher,
        )
        .await
        .unwrap();
        let r: Value = serde_json::from_str(&r).unwrap();
        assert_eq!(r["result"]["herdr_socket"], "/tmp/herdr.sock");
        assert_eq!(r["result"]["api_version"], 1);
    }

    #[tokio::test]
    async fn any_dispatcher_works() {
        let r = handle_text(r#"{"jsonrpc":"2.0","id":1,"method":"core.ping"}"#, &Broken)
            .await
            .unwrap();
        let r: Value = serde_json::from_str(&r).unwrap();
        assert_eq!(r["result"]["type"], "ok");
    }
}
