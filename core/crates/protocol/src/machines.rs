//! herdr machines (ADR 0011): Local plus herdr's saved SSH machines, each
//! with its connection state, for the sidebar's machine groups.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The ID shells use for Local. Workspaces on Local have no `machine`.
pub const LOCAL_MACHINE: &str = "local";

/// A herdr machine and how Azulejo's link to it is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MachineInfo {
    /// `"local"`, or herdr's saved-machine ID (what `Workspace.machine` holds).
    pub id: String,
    /// What the sidebar shows: "Local", or the label saved in herdr.
    pub label: String,
    pub state: MachineState,
    /// Why it's reconnecting or needs attention, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl MachineInfo {
    pub fn is_local(&self) -> bool {
        self.id == LOCAL_MACHINE
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MachineState {
    /// Not up yet since Azulejo started.
    Connecting,
    Connected,
    /// The link dropped; retrying with backoff.
    Reconnecting,
    /// Retrying won't help (password prompt, host key, no herdr there):
    /// the user fixes it in herdr, e.g. `herdr machine reconnect <label>`.
    Error,
    /// Not connected on purpose: disabled in herdr or `connect = false`.
    Off,
}

/// `machines.list` params (none).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MachinesList {}
