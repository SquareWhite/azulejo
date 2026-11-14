use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use azulejo_model::ModelError;

/// Why a command failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request wasn't valid JSON.
    ParseError,
    /// Valid JSON, but not a request (e.g. no `method`).
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    /// A window, workspace, tab or tile doesn't exist.
    NotFound,
    /// The command doesn't apply right now (nothing focused, no neighbour that way…).
    InvalidState,
    /// Not available in this build or configuration.
    Unsupported,
    /// A service the command needs (herdr) isn't reachable right now.
    Unavailable,
    Internal,
}

impl ErrorCode {
    /// The JSON-RPC 2.0 error code for this error.
    pub fn rpc_code(self) -> i32 {
        match self {
            ErrorCode::ParseError => -32700,
            ErrorCode::InvalidRequest => -32600,
            ErrorCode::MethodNotFound => -32601,
            ErrorCode::InvalidParams => -32602,
            ErrorCode::Internal => -32603,
            ErrorCode::NotFound => -32001,
            ErrorCode::InvalidState => -32002,
            ErrorCode::Unsupported => -32003,
            ErrorCode::Unavailable => -32004,
        }
    }
}

/// A failed command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Error {
    pub code: ErrorCode,
    /// For people: shells show it as is, so it names no IDs.
    pub message: String,
    /// For programs: what the error is about, such as `{"tile": "tile_…",
    /// "direction": "right"}` or `{"param": "kind"}`. Over JSON-RPC these
    /// fields sit in the error's `data` beside `code`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Adds `data`'s fields (an object) to the error's data.
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        match (&mut self.data, data) {
            (Some(serde_json::Value::Object(have)), serde_json::Value::Object(more)) => {
                have.extend(more)
            }
            (slot, data) => *slot = Some(data),
        }
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

impl From<ModelError> for Error {
    fn from(e: ModelError) -> Self {
        use serde_json::json;
        let code = match &e {
            e if e.is_not_found() => ErrorCode::NotFound,
            ModelError::InvalidValue(_) => ErrorCode::InvalidParams,
            _ => ErrorCode::InvalidState,
        };
        let data = match &e {
            ModelError::WindowNotFound(id) => Some(json!({ "window": id })),
            ModelError::WorkspaceNotFound(id) => Some(json!({ "workspace": id })),
            ModelError::TabNotFound(id) => Some(json!({ "tab": id })),
            ModelError::TileNotFound(id) => Some(json!({ "tile": id })),
            ModelError::NoNeighbour { tile, direction }
            | ModelError::NoEdge { tile, direction } => {
                Some(json!({ "tile": tile, "direction": direction }))
            }
            ModelError::DifferentTabs { a, b } => Some(json!({ "tiles": [a, b] })),
            ModelError::NothingFocused | ModelError::InvalidValue(_) => None,
        };
        let error = Error::new(code, e.to_string());
        match data {
            Some(data) => error.with_data(data),
            None => error,
        }
    }
}
