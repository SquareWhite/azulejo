//! How a [`Client`](crate::Client) reaches a herdr server: the local Unix
//! socket, or a bridge process whose stdin and stdout carry the same NDJSON
//! (herdr's own `remote-api-bridge` over SSH, ADR 0011). Either way one
//! connection carries one request and its reply, or one event subscription.

use std::ffi::OsString;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};
use tokio::net::UnixStream;
use tokio::process::{Child, ChildStdin, ChildStdout};

use crate::error::{Error, Result};

/// Where a client's connections go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    /// A herdr server on this machine.
    Socket(PathBuf),
    /// A process that bridges stdio to a herdr server's API; one process per
    /// connection.
    Bridge(BridgeCommand),
}

/// A process to spawn per connection; its stdin and stdout are the
/// connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// The complete environment (spawned with a cleared environment).
    pub env: Vec<(OsString, OsString)>,
    /// Shown in errors, e.g. "machine dev-server".
    pub label: String,
}

impl BridgeCommand {
    /// `program` followed by `args`.
    pub fn argv(&self) -> Vec<String> {
        std::iter::once(self.program.to_string_lossy().into_owned())
            .chain(self.args.iter().cloned())
            .collect()
    }
}

/// One open connection.
pub(crate) struct Conn {
    pub read: Box<dyn AsyncRead + Send + Unpin>,
    pub write: Box<dyn AsyncWrite + Send + Unpin>,
    /// The bridge process, killed when the connection is dropped.
    pub guard: Option<BridgeGuard>,
}

impl Conn {
    pub(crate) fn unix(stream: UnixStream) -> Self {
        let (read, write) = stream.into_split();
        Self {
            read: Box::new(read),
            write: Box::new(write),
            guard: None,
        }
    }

    /// Why the bridge ended, if this is a bridge that wrote to stderr.
    pub(crate) fn bridge_error(&self) -> Option<Error> {
        self.guard.as_ref().map(BridgeGuard::error)
    }
}

/// Keeps a bridge process alive and remembers what it said on stderr.
pub(crate) struct BridgeGuard {
    _child: Child,
    stderr: Arc<Mutex<String>>,
    label: String,
}

impl BridgeGuard {
    pub(crate) fn error(&self) -> Error {
        // Give the stderr reader a moment to catch the last line.
        let message = self.stderr.lock().map(|s| s.clone()).unwrap_or_default();
        Error::Bridge {
            label: self.label.clone(),
            message: last_line(&message),
        }
    }
}

fn last_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("the connection closed")
        .to_owned()
}

/// Starts a bridge process. Its stderr is kept (bounded) for error messages.
pub(crate) fn spawn_bridge(cmd: &BridgeCommand) -> Result<Conn> {
    let mut child = tokio::process::Command::new(&cmd.program)
        .env_clear()
        .envs(cmd.env.iter().cloned())
        .args(&cmd.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::Binary(format!("{}: {e}", cmd.program.display())))?;
    let stdin: ChildStdin = child.stdin.take().expect("piped stdin");
    let stdout: ChildStdout = child.stdout.take().expect("piped stdout");
    let stderr_text = Arc::new(Mutex::new(String::new()));
    if let Some(mut stderr) = child.stderr.take() {
        let sink = stderr_text.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stderr.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                if let Ok(mut s) = sink.lock()
                    && s.len() < 16 * 1024
                {
                    s.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            }
        });
    }
    Ok(Conn {
        read: Box::new(stdout),
        write: Box::new(stdin),
        guard: Some(BridgeGuard {
            _child: child,
            stderr: stderr_text,
            label: cmd.label.clone(),
        }),
    })
}

/// An [`AsyncRead`] that also holds the bridge guard, so the process lives as
/// long as the reader.
pub(crate) struct GuardedRead {
    pub inner: Box<dyn AsyncRead + Send + Unpin>,
    pub guard: Option<BridgeGuard>,
}

impl GuardedRead {
    pub(crate) fn bridge_error(&self) -> Option<Error> {
        self.guard.as_ref().map(BridgeGuard::error)
    }
}

impl AsyncRead for GuardedRead {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn a_bridge_carries_a_line_each_way() {
        let cmd = BridgeCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "read l; printf '%s!\\n' \"$l\"".into()],
            env: vec![],
            label: "test".into(),
        };
        let mut conn = spawn_bridge(&cmd).unwrap();
        conn.write.write_all(b"hi\n").await.unwrap();
        let mut line = String::new();
        BufReader::new(&mut conn.read)
            .read_line(&mut line)
            .await
            .unwrap();
        assert_eq!(line, "hi!\n");
    }

    #[tokio::test]
    async fn a_failing_bridge_reports_its_last_stderr_line() {
        let cmd = BridgeCommand {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "echo 'ssh: connect to host x port 22: Connection refused' >&2; exit 255".into(),
            ],
            env: vec![],
            label: "machine x".into(),
        };
        let mut conn = spawn_bridge(&cmd).unwrap();
        let mut line = String::new();
        let n = BufReader::new(&mut conn.read)
            .read_line(&mut line)
            .await
            .unwrap();
        assert_eq!(n, 0);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let err = conn.bridge_error().unwrap();
        assert!(err.is_unreachable());
        assert!(err.to_string().contains("Connection refused"), "{err}");
        assert!(err.to_string().contains("machine x"), "{err}");
    }
}
