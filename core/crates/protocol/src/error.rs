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
        }
    }
}

/// A failed command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
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
        let code = match &e {
            e if e.is_not_found() => ErrorCode::NotFound,
            ModelError::InvalidValue(_) => ErrorCode::InvalidParams,
            _ => ErrorCode::InvalidState,
        };
        Error::new(code, e.to_string())
    }
}
