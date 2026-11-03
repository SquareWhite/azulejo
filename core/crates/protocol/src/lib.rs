//! Commands, events and errors shared by the shells, the API server and the CLI.

use serde::{Deserialize, Serialize};

/// Version of the public API schema.
pub const API_VERSION: u32 = 1;

/// A request sent to the core, by a shell or through the socket API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Command {
    #[serde(rename = "core.ping")]
    Ping,
}

/// Something that happened in the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum Event {
    #[serde(rename = "core.pong")]
    Pong { api_version: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_round_trips_through_json() {
        let json = serde_json::to_string(&Command::Ping).unwrap();
        assert_eq!(json, r#"{"method":"core.ping"}"#);
        assert_eq!(
            serde_json::from_str::<Command>(&json).unwrap(),
            Command::Ping
        );
    }
}
