//! Finding the herdr binary, locating a session's socket, and starting a
//! server with a clean environment.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::client::{Backoff, Client, ClientOptions};
use crate::env::clean_process_env;
use crate::error::{Error, Result};
use crate::types::ServerInfo;

/// Which herdr session to talk to. Azulejo uses herdr's default session by
/// default, shared with the user's own herdr TUI (ADR 0008).
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

/// Whether [`Herdr::connect_or_start`] found the server running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerOrigin {
    Running,
    Started,
}

/// Dev profile only: an absolute path to the herdr binary to run instead of
/// the one on `PATH`, e.g. a local herdr build (`apps/macos/dev.sh` passes it).
pub const BIN_ENV: &str = "AZJ_HERDR_BIN";
/// Dev profile only: an absolute path to a herdr `config.toml` that every
/// herdr process Azulejo starts reads instead of herdr's own default (handed
/// over as herdr's `HERDR_CONFIG_PATH`).
pub const CONFIG_ENV: &str = "AZJ_HERDR_CONFIG";
/// What marks a dev profile run (`apps/macos/dev.sh` sets it to `dev`). The
/// overrides above are ignored without it, so an installed app never picks
/// them up from a stray environment.
const PROFILE_ENV: &str = "AZULEJO_PROFILE";
/// herdr's own variable for its config file.
const HERDR_CONFIG_PATH: &str = "HERDR_CONFIG_PATH";

/// The herdr executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Herdr {
    bin: PathBuf,
    /// A herdr config file to use instead of herdr's default (dev profile).
    config: Option<PathBuf>,
}

impl Herdr {
    pub fn with_path(bin: impl Into<PathBuf>) -> Self {
        Self {
            bin: bin.into(),
            config: None,
        }
    }

    /// Look on `PATH`, then in the usual install locations. GUI apps on
    /// macOS get a minimal `PATH`, so `~/.local/bin` (herdr's installer
    /// default) is checked explicitly.
    ///
    /// On the dev profile, [`BIN_ENV`] wins over all of that and
    /// [`CONFIG_ENV`] picks herdr's config file. A bad path in either is an
    /// error, never a quiet fall back to the stock herdr.
    pub fn locate() -> Result<Self> {
        Self::locate_with(|name| std::env::var_os(name))
    }

    /// [`Herdr::locate`], reading the environment through `var`.
    fn locate_with(var: impl Fn(&str) -> Option<OsString>) -> Result<Self> {
        let set = |name: &str| var(name).filter(|v| !v.is_empty());
        if var(PROFILE_ENV).as_deref() != Some(OsStr::new("dev")) {
            return Self::search(&var);
        }
        let config = set(CONFIG_ENV)
            .map(|p| checked_path(CONFIG_ENV, p, false))
            .transpose()?;
        let herdr = match set(BIN_ENV) {
            Some(p) => Self::with_path(checked_path(BIN_ENV, p, true)?),
            None => Self::search(&var)?,
        };
        Ok(Self { config, ..herdr })
    }

    fn search(var: &impl Fn(&str) -> Option<OsString>) -> Result<Self> {
        let mut candidates: Vec<PathBuf> = var("PATH")
            .map(|p| std::env::split_paths(&p).map(|d| d.join("herdr")).collect())
            .unwrap_or_default();
        if let Some(home) = var("HOME") {
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

    /// The herdr config file every herdr process gets, when it isn't herdr's
    /// default.
    pub fn config(&self) -> Option<&Path> {
        self.config.as_deref()
    }

    /// Environment for every herdr process Azulejo starts.
    pub fn env(&self) -> Vec<(OsString, OsString)> {
        let mut env = clean_process_env(&[]);
        if let Some(config) = &self.config {
            env.push((HERDR_CONFIG_PATH.into(), config.into()));
        }
        env
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
    /// it isn't running. A running server is used as it is, never restarted:
    /// restarting herdr kills every process in it. The server is detached from Azulejo (own process
    /// group), so it outlives the app: that's what keeps sessions alive.
    pub async fn ensure_server(
        &self,
        session: &Session,
        opts: ClientOptions,
    ) -> Result<(Client, ServerInfo)> {
        self.connect_or_start(session, opts)
            .await
            .map(|(client, info, _)| (client, info))
    }

    /// [`Herdr::ensure_server`], also saying whether the server was already
    /// running. One that was just started has fresh terminals in every pane
    /// it restored.
    pub async fn connect_or_start(
        &self,
        session: &Session,
        opts: ClientOptions,
    ) -> Result<(Client, ServerInfo, ServerOrigin)> {
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
            Ok(info) => return Ok((Client::new(socket, opts), info, ServerOrigin::Running)),
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
        Ok((Client::new(socket, opts), info, ServerOrigin::Started))
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

/// An override path from the variable `name`: absolute, there and a file
/// (executable, when `exec`).
fn checked_path(name: &str, value: OsString, exec: bool) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = PathBuf::from(value);
    let bad = |why: &str| Error::Binary(format!("{name}={}: {why}", path.display()));
    if !path.is_absolute() {
        return Err(bad("not an absolute path"));
    }
    let meta = std::fs::metadata(&path).map_err(|e| bad(&e.to_string()))?;
    if !meta.is_file() {
        return Err(bad("not a file"));
    }
    if exec && meta.permissions().mode() & 0o111 == 0 {
        return Err(bad("not executable"));
    }
    Ok(path)
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

    /// A fake environment for [`Herdr::locate_with`].
    fn env_of(vars: &[(&str, &Path)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let vars: Vec<(String, OsString)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.as_os_str().to_owned()))
            .collect();
        move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    /// A temp dir with `stock/herdr` (found on `PATH`), an executable
    /// `fork/herdr`, a non-executable `plain` and a `herdr.toml`.
    fn fixture() -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        for exe in ["stock/herdr", "fork/herdr"] {
            let p = dir.path().join(exe);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(dir.path().join("plain"), "").unwrap();
        std::fs::write(dir.path().join("herdr.toml"), "").unwrap();
        dir
    }

    fn config_var(herdr: &Herdr) -> Option<OsString> {
        herdr
            .env()
            .into_iter()
            .find(|(k, _)| k == "HERDR_CONFIG_PATH")
            .map(|(_, v)| v)
    }

    #[test]
    fn dev_profile_honours_the_overrides() {
        let dir = fixture();
        let d = dir.path();
        let herdr = Herdr::locate_with(env_of(&[
            ("AZULEJO_PROFILE", Path::new("dev")),
            ("PATH", &d.join("stock")),
            (BIN_ENV, &d.join("fork/herdr")),
            (CONFIG_ENV, &d.join("herdr.toml")),
        ]))
        .unwrap();
        assert_eq!(herdr.path(), d.join("fork/herdr"));
        assert_eq!(herdr.config(), Some(d.join("herdr.toml").as_path()));
        assert_eq!(config_var(&herdr), Some(d.join("herdr.toml").into()));
        // The attach argv runs the same binary with the same config.
        let attach = herdr.attach(&Session::named("azj-dev-x"), &"term_1".into(), false);
        assert_eq!(attach.program, d.join("fork/herdr"));
        assert!(attach.env.iter().any(|(k, _)| k == "HERDR_CONFIG_PATH"));
    }

    #[test]
    fn dev_profile_config_alone_keeps_the_stock_binary() {
        let dir = fixture();
        let d = dir.path();
        let herdr = Herdr::locate_with(env_of(&[
            ("AZULEJO_PROFILE", Path::new("dev")),
            ("PATH", &d.join("stock")),
            (CONFIG_ENV, &d.join("herdr.toml")),
        ]))
        .unwrap();
        assert_eq!(herdr.path(), d.join("stock/herdr"));
        assert_eq!(herdr.config(), Some(d.join("herdr.toml").as_path()));
    }

    #[test]
    fn overrides_are_ignored_outside_the_dev_profile() {
        let dir = fixture();
        let d = dir.path();
        let (stock, fork, toml) = (d.join("stock"), d.join("fork/herdr"), d.join("herdr.toml"));
        for profile in [None, Some(""), Some("release"), Some("Dev")] {
            let mut vars: Vec<(&str, &Path)> =
                vec![("PATH", &stock), (BIN_ENV, &fork), (CONFIG_ENV, &toml)];
            if let Some(p) = profile {
                vars.push(("AZULEJO_PROFILE", Path::new(p)));
            }
            let herdr = Herdr::locate_with(env_of(&vars)).unwrap();
            assert_eq!(herdr.path(), stock.join("herdr"), "profile {profile:?}");
            assert_eq!(herdr.config(), None);
            assert_eq!(config_var(&herdr), None);
        }
    }

    #[test]
    fn bad_override_paths_are_errors_not_fallbacks() {
        let dir = fixture();
        let d = dir.path();
        let cases = [
            (BIN_ENV, d.join("missing"), "AZJ_HERDR_BIN="),
            (BIN_ENV, d.join("plain"), "not executable"),
            (BIN_ENV, d.join("fork"), "not a file"),
            (BIN_ENV, PathBuf::from("fork/herdr"), "not an absolute path"),
            (CONFIG_ENV, d.join("missing.toml"), "AZJ_HERDR_CONFIG="),
            (CONFIG_ENV, d.to_path_buf(), "not a file"),
            (
                CONFIG_ENV,
                PathBuf::from("herdr.toml"),
                "not an absolute path",
            ),
        ];
        for (name, value, want) in cases {
            let err = Herdr::locate_with(env_of(&[
                ("AZULEJO_PROFILE", Path::new("dev")),
                ("PATH", &d.join("stock")),
                (name, &value),
            ]))
            .unwrap_err()
            .to_string();
            assert!(err.contains(want), "{name}={}: {err}", value.display());
        }
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
