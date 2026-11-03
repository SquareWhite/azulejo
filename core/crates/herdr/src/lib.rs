//! herdr adapter. No other crate may use herdr method names or JSON shapes.

use std::path::PathBuf;

/// Minimum herdr socket protocol version Azulejo supports.
pub const MIN_PROTOCOL: u32 = 22;

/// Default herdr socket path, honouring `HERDR_SOCKET_PATH`.
pub fn default_socket_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("HERDR_SOCKET_PATH") {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/herdr/herdr.sock"))
}

/// Arguments for the child process a terminal tile runs to view a herdr terminal.
pub fn attach_argv(terminal_id: &str) -> Vec<String> {
    vec![
        "herdr".into(),
        "terminal".into(),
        "attach".into(),
        terminal_id.into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_argv_targets_the_terminal() {
        assert_eq!(attach_argv("term_1").last().unwrap(), "term_1");
    }
}
