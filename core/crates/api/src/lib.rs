//! JSON-RPC server over a Unix socket.

use std::path::PathBuf;

/// Directory that holds the Azulejo socket for this platform.
pub fn socket_dir() -> PathBuf {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("TMPDIR")
    } else {
        std::env::var_os("XDG_RUNTIME_DIR")
    };
    base.map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("azulejo")
}

pub fn socket_path() -> PathBuf {
    socket_dir().join("azulejo.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_lives_in_azulejo_dir() {
        assert!(socket_path().ends_with("azulejo/azulejo.sock"));
    }
}
