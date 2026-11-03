//! Finding the herdr binary, locating a session's socket, and starting a
//! server with a clean environment.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::client::{Backoff, Client, ClientOptions};
use crate::env::clean_process_env;
use crate::error::{Error, Result};
use crate::types::ServerInfo;

/// Which herdr session to talk to. Azulejo uses a named session so it never
/// shares a server with a herdr TUI the user runs by hand.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Session(Option<String>);

impl Session {
    pub fn named(name: impl Into<String>) -> Self {
        Self(Some(name.into()))
    }

    /// herdr's default session (`~/.config/herdr/herdr.sock`).
    pub fn default_session() -> Self {
        Self(None)
    }

    pub fn name(&self) -> Option<&str> {
        self.0.as_deref()
    }

    /// CLI arguments that select this session; they go before the subcommand.
    pub fn args(&self) -> Vec<String> {
        match &self.0 {
            Some(name) => vec!["--session".into(), name.clone()],
            None => Vec::new(),
        }
    }
}

/// The herdr executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Herdr {
    bin: PathBuf,
}

impl Herdr {
    pub fn with_path(bin: impl Into<PathBuf>) -> Self {
        Self { bin: bin.into() }
    }

    /// Look on `PATH`, then in the usual install locations. GUI apps on
    /// macOS get a minimal `PATH`, so `~/.local/bin` (herdr's installer
    /// default) is checked explicitly.
    pub fn locate() -> Result<Self> {
        let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).map(|d| d.join("herdr")).collect())
            .unwrap_or_default();
        if let Some(home) = std::env::var_os("HOME") {
            candidates.push(PathBuf::from(home).join(".local/bin/herdr"));
        }
        candidates.push("/opt/homebrew/bin/herdr".into());
        candidates.push("/usr/local/bin/herdr".into());
        candidates
            .into_iter()
            .find(|p| p.is_file())
            .map(Self::with_path)
            .ok_or_else(|| Error::Binary("herdr not found on PATH or in ~/.local/bin".into()))
    }

    pub fn path(&self) -> &Path {
        &self.bin
    }

    /// Environment for every herdr process Azulejo starts.
    pub fn env(&self) -> Vec<(OsString, OsString)> {
        clean_process_env(&[])
    }

    /// Run a herdr CLI command with a clean environment; returns stdout.
    pub async fn cli(&self, session: &Session, args: &[&str]) -> Result<String> {
        let out = tokio::process::Command::new(&self.bin)
            .env_clear()
            .envs(self.env())
            .args(session.args())
            .args(args)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|e| Error::Binary(format!("{}: {e}", self.bin.display())))?;
        if !out.status.success() {
            return Err(Error::Cli {
                command: args.join(" "),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// The API socket path for `session`, as herdr reports it.
    pub async fn socket_path(&self, session: &Session) -> Result<PathBuf> {
        let status = self.cli(session, &["status"]).await?;
        parse_status_socket(&status)
            .ok_or_else(|| Error::Protocol(format!("no server socket in `herdr status`: {status}")))
    }

    /// Return a client for `session`'s server, starting the server first if
    /// it isn't running. The server is detached from Azulejo (own process
    /// group), so it outlives the app: that's what keeps sessions alive.
    pub async fn ensure_server(
        &self,
        session: &Session,
        opts: ClientOptions,
    ) -> Result<(Client, ServerInfo)> {
        let socket = self.socket_path(session).await?;
        let probe = Client::new(
            &socket,
            ClientOptions {
                connect: Backoff {
                    give_up_after: Duration::ZERO,
                    ..opts.connect
                },
                ..opts
            },
        );
        match probe.ping().await {
            Ok(info) => return Ok((Client::new(socket, opts), info)),
            Err(e) if e.is_unreachable() => {}
            Err(e) => return Err(e),
        }

        self.spawn_server(session)?;
        let waiting = Client::new(
            &socket,
            ClientOptions {
                connect: Backoff {
                    give_up_after: opts.connect.give_up_after.max(Duration::from_secs(10)),
                    ..opts.connect
                },
                ..opts
            },
        );
        let info = waiting.ping().await?;
        Ok((Client::new(socket, opts), info))
    }

    fn spawn_server(&self, session: &Session) -> Result<()> {
        use std::os::unix::process::CommandExt;
        let mut child = std::process::Command::new(&self.bin)
            .env_clear()
            .envs(self.env())
            .args(session.args())
            .arg("server")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|e| Error::Binary(format!("starting herdr server: {e}")))?;
        // Reap it whenever it exits so it never lingers as a zombie.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    /// Delete a stopped named session's directory (tests and cleanup).
    pub async fn delete_session(&self, name: &str) -> Result<()> {
        self.cli(&Session::default_session(), &["session", "delete", name])
            .await
            .map(drop)
    }
}

fn parse_status_socket(status: &str) -> Option<PathBuf> {
    let server = status.split("\nserver:").nth(1)?;
    server
        .lines()
        .find_map(|l| l.trim().strip_prefix("socket:"))
        .map(|s| PathBuf::from(s.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_args_go_first() {
        assert_eq!(Session::named("azj").args(), ["--session", "azj"]);
        assert!(Session::default_session().args().is_empty());
    }

    #[test]
    fn parses_socket_from_status() {
        let status = "client:\n  version: 0.9.1\n  protocol: 22\n\nserver:\n  status: not running\n  socket: /Users/x/.config/herdr/sessions/azj/herdr.sock\n\nupdate:\n  restart_needed: no\n";
        assert_eq!(
            parse_status_socket(status),
            Some(PathBuf::from(
                "/Users/x/.config/herdr/sessions/azj/herdr.sock"
            ))
        );
    }
}
