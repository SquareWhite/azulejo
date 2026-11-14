//! A small blocking client for the socket API.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use azulejo_protocol::ErrorCode;
use serde_json::{Value, json};

/// How long a call waits for its response.
const TIMEOUT: Duration = Duration::from_secs(30);

/// An error reply from the server.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    /// JSON-RPC error code.
    pub code: i64,
    pub message: String,
    /// The protocol's error code, when the server sent one.
    pub kind: Option<ErrorCode>,
    /// The rest of the error's `data`: what it's about (`tile`, `param`…).
    pub data: serde_json::Map<String, Value>,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            Some(kind) => {
                let kind = serde_json::to_value(kind).unwrap_or_default();
                write!(f, "{}: {}", kind.as_str().unwrap_or("error"), self.message)?;
            }
            None => write!(f, "{} ({})", self.message, self.code)?,
        }
        // The message names no IDs; say which things it's about here.
        if !self.data.is_empty() {
            let about: Vec<String> = self
                .data
                .iter()
                .map(|(k, v)| match v {
                    Value::String(s) => format!("{k} {s}"),
                    other => format!("{k} {other}"),
                })
                .collect();
            write!(f, " ({})", about.join(", "))?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Nothing is listening at the socket path.
    #[error("Azulejo isn't running (nothing listening at {})", path.display())]
    NotRunning { path: PathBuf },
    #[error("can't connect to {}: {source}", path.display())]
    Connect { path: PathBuf, source: io::Error },
    #[error("socket error: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Rpc(RpcError),
    #[error("unexpected response from the server: {0}")]
    Protocol(String),
}

/// One connection to the server. Calls run one at a time.
#[derive(Debug)]
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
    /// Notifications that arrived while waiting for a response.
    notifications: VecDeque<Value>,
}

impl Client {
    pub fn connect(path: impl AsRef<Path>) -> Result<Client, ClientError> {
        let path = path.as_ref();
        let stream = UnixStream::connect(path).map_err(|source| match source.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => ClientError::NotRunning {
                path: path.to_owned(),
            },
            _ => ClientError::Connect {
                path: path.to_owned(),
                source,
            },
        })?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        Ok(Client {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
            next_id: 1,
            notifications: VecDeque::new(),
        })
    }

    /// Connects to [`socket_path`](crate::socket_path).
    pub fn connect_default() -> Result<Client, ClientError> {
        Client::connect(crate::socket_path())
    }

    /// Calls `method` and returns its `result`.
    pub fn call(&mut self, method: &str, params: Option<Value>) -> Result<Value, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        let mut request = json!({"jsonrpc": "2.0", "id": id, "method": method});
        if let Some(params) = params {
            request["params"] = params;
        }
        let mut line = request.to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;

        let mut response = loop {
            let message = self.read_message()?;
            // Event notifications (no `id`) may arrive before the response.
            if message.get("id").is_none() && message.get("method").is_some() {
                self.notifications.push_back(message);
                continue;
            }
            break message;
        };
        if response.get("id") != Some(&json!(id)) {
            return Err(ClientError::Protocol(format!(
                "expected a response to request {id}, got {response}"
            )));
        }
        if let Some(err) = response.get("error") {
            return Err(ClientError::Rpc(RpcError {
                code: err.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
                    .to_owned(),
                kind: err
                    .get("data")
                    .and_then(|d| d.get("code"))
                    .and_then(|c| serde_json::from_value(c.clone()).ok()),
                data: err
                    .get("data")
                    .and_then(Value::as_object)
                    .map(|d| {
                        let mut d = d.clone();
                        d.remove("code");
                        d
                    })
                    .unwrap_or_default(),
            }));
        }
        response
            .get_mut("result")
            .map(Value::take)
            .ok_or_else(|| ClientError::Protocol("response has no `result`".into()))
    }

    /// Starts an event stream on this connection (`events.subscribe`) and
    /// returns its subscription ID. Read events with
    /// [`Client::next_notification`].
    pub fn subscribe(&mut self, params: Option<Value>) -> Result<u64, ClientError> {
        let result = self.call("events.subscribe", params)?;
        result
            .get("subscription")
            .and_then(Value::as_u64)
            .ok_or_else(|| ClientError::Protocol(format!("no subscription in {result}")))
    }

    /// The next notification (`{"jsonrpc":"2.0","method":"events.event",
    /// "params":{…}}`), waiting at most `timeout` (`None` waits forever).
    /// Returns `Ok(None)` on timeout.
    pub fn next_notification(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<Option<Value>, ClientError> {
        if let Some(n) = self.notifications.pop_front() {
            return Ok(Some(n));
        }
        self.writer.set_read_timeout(timeout)?;
        let message = self.read_message();
        self.writer.set_read_timeout(Some(TIMEOUT))?;
        match message {
            Ok(message) => Ok(Some(message)),
            Err(ClientError::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    fn read_message(&mut self) -> Result<Value, ClientError> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(ClientError::Protocol("server closed the connection".into()));
        }
        serde_json::from_str(&line).map_err(|e| ClientError::Protocol(format!("invalid JSON: {e}")))
    }
}
