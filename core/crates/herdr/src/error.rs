use std::path::PathBuf;
use std::time::Duration;

/// Everything that can go wrong talking to herdr.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The socket never accepted a connection within the retry budget.
    #[error("cannot connect to herdr at {path}: {source}")]
    Connect {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// I/O failed after the connection was up.
    #[error("herdr socket I/O: {0}")]
    Io(#[from] std::io::Error),
    /// No reply within the request timeout.
    #[error("herdr did not reply to `{method}` within {timeout:?}")]
    Timeout { method: String, timeout: Duration },
    /// herdr closed the connection before replying.
    #[error("herdr closed the connection before replying to `{method}`")]
    Closed { method: String },
    /// The reply was not the JSON we expected.
    #[error("unexpected reply from herdr: {0}")]
    Protocol(String),
    /// herdr answered with an error body.
    #[error("herdr error {code}: {message}")]
    Server { code: String, message: String },
    /// The server speaks an older protocol than Azulejo supports.
    #[error("herdr protocol {found} (herdr {version}) is older than the minimum {min}")]
    Version {
        found: u32,
        min: u32,
        version: String,
    },
    /// The herdr binary could not be found or run.
    #[error("herdr binary: {0}")]
    Binary(String),
    /// A bridge to a remote machine ended without a reply (SSH failed, or
    /// no usable herdr there). `message` is its last stderr line.
    #[error("{label}: {message}")]
    Bridge { label: String, message: String },
    /// A herdr CLI command failed.
    #[error("`herdr {command}` failed: {stderr}")]
    Cli { command: String, stderr: String },
}

impl Error {
    /// herdr's error code, when the server sent one (e.g. `pane_not_found`).
    pub fn code(&self) -> Option<&str> {
        match self {
            Error::Server { code, .. } => Some(code),
            _ => None,
        }
    }

    /// True for `*_not_found` server errors.
    pub fn is_not_found(&self) -> bool {
        self.code().is_some_and(|c| c.ends_with("_not_found"))
    }

    /// True when herdr isn't reachable at all (not running, or went away).
    pub fn is_unreachable(&self) -> bool {
        matches!(
            self,
            Error::Connect { .. } | Error::Closed { .. } | Error::Bridge { .. }
        )
    }

    /// A remote machine that won't come back by retrying: it wants a
    /// password or a host key confirmed, or has no usable herdr. The user
    /// fixes it in herdr (`herdr machine reconnect <label>`).
    pub fn needs_attention(&self) -> bool {
        let Error::Bridge { message, .. } = self else {
            return false;
        };
        needs_attention(message)
    }
}

/// See [`Error::needs_attention`]; works on any message, e.g. a stream's
/// disconnect reason.
pub fn needs_attention(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    [
        "permission denied",
        "host key verification failed",
        "remote host identification has changed",
        "no matching host key",
        "could not resolve hostname",
        "no usable herdr",
        "too old",
    ]
    .iter()
    .any(|needle| m.contains(needle))
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
