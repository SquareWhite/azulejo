//! The child process a terminal tile runs: `herdr terminal attach`, and how
//! to read its exit.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::client::Client;
use crate::server::{Herdr, Session};
use crate::types::{PaneId, TerminalId};

/// A command for a shell to spawn inside the tile's PTY. The PTY must have a
/// non-zero size before spawning, or attach fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// The complete environment for the child (spawn with a cleared env).
    pub env: Vec<(OsString, OsString)>,
}

impl AttachCommand {
    /// `program` followed by `args`.
    pub fn argv(&self) -> Vec<String> {
        std::iter::once(self.program.to_string_lossy().into_owned())
            .chain(self.args.iter().cloned())
            .collect()
    }

    /// Wrap the command so its stderr goes to `stderr_file` instead of the
    /// terminal. herdr prints its exit reason on stderr; the shell reads the
    /// file after exit and passes it to [`AttachExit::classify`].
    pub fn with_stderr_file(mut self, stderr_file: &Path) -> Self {
        let mut args = vec![
            "-c".to_owned(),
            r#"exec "$0" "$@" 2>>"$AZJ_ATTACH_STDERR""#.to_owned(),
            self.program.to_string_lossy().into_owned(),
        ];
        args.append(&mut self.args);
        self.env.push((
            "AZJ_ATTACH_STDERR".into(),
            stderr_file.as_os_str().to_owned(),
        ));
        Self {
            program: "/bin/sh".into(),
            args,
            env: self.env,
        }
    }
}

impl Herdr {
    /// `herdr [--session S] terminal attach <terminal-id> [--takeover]`.
    /// `--takeover` must follow the ID; herdr rejects it before.
    pub fn attach(
        &self,
        session: &Session,
        terminal: &TerminalId,
        takeover: bool,
    ) -> AttachCommand {
        let mut args = session.args();
        args.extend(["terminal".into(), "attach".into(), terminal.to_string()]);
        if takeover {
            args.push("--takeover".into());
        }
        AttachCommand {
            program: self.path().to_owned(),
            args,
            env: self.env(),
        }
    }

    /// Read-only view at the tile's size, for when another client owns the
    /// pane. Prints NDJSON `terminal.frame` records, not raw terminal output.
    pub fn observe(&self, session: &Session, pane: &PaneId, cols: u16, rows: u16) -> AttachCommand {
        let mut args = session.args();
        args.extend([
            "terminal".into(),
            "session".into(),
            "observe".into(),
            pane.to_string(),
            "--cols".into(),
            cols.to_string(),
            "--rows".into(),
            rows.to_string(),
        ]);
        AttachCommand {
            program: self.path().to_owned(),
            args,
            env: self.env(),
        }
    }
}

/// Why an attach process ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachExit {
    /// The user pressed the detach key (`ctrl+b q`). Probably an accident
    /// inside a tile; reattaching silently is reasonable.
    Detached,
    /// Another client attached with `--takeover`. Show the tile read-only
    /// with "Take back"; never retake automatically.
    TakenOver,
    /// Another client already owns the terminal and we didn't ask to take
    /// over. Show the tile read-only with "Take over".
    AlreadyAttached,
    /// The pane closed or its process exited.
    TerminalGone,
    /// The herdr server is stopping or gone.
    ServerStopping,
    /// Anything else, including death by signal (`code: None`).
    Failed { code: Option<i32>, message: String },
}

impl AttachExit {
    /// Classify from the exit code and the captured stderr (see
    /// [`AttachCommand::with_stderr_file`]). Messages from herdr 0.9.1;
    /// note herdr prefixes several with "server shut down" even when the
    /// server is still running.
    pub fn classify(code: Option<i32>, stderr: Option<&str>) -> Self {
        if code == Some(0) {
            return AttachExit::Detached;
        }
        let msg = stderr.map(str::trim).unwrap_or_default();
        if msg.contains("taken over") {
            AttachExit::TakenOver
        } else if msg.contains("already has an attached client") {
            AttachExit::AlreadyAttached
        } else if msg.contains("attach ended") && msg.contains("not found") {
            AttachExit::TerminalGone
        } else if msg.contains("server is shutting down") {
            AttachExit::ServerStopping
        } else {
            AttachExit::Failed {
                code,
                message: msg.lines().last().unwrap_or_default().to_owned(),
            }
        }
    }
}

impl Client {
    /// Like [`AttachExit::classify`], but when stderr didn't say, ask herdr:
    /// unreachable → `ServerStopping`, terminal missing → `TerminalGone`,
    /// terminal still there after a non-zero exit → `TakenOver`.
    pub async fn diagnose_attach_exit(
        &self,
        code: Option<i32>,
        stderr: Option<&str>,
        terminal: &TerminalId,
    ) -> AttachExit {
        let exit = AttachExit::classify(code, stderr);
        if !matches!(exit, AttachExit::Failed { code: Some(1), .. }) {
            return exit;
        }
        match self.resolve_terminal(terminal).await {
            Err(e) if e.is_unreachable() => AttachExit::ServerStopping,
            Ok(None) => AttachExit::TerminalGone,
            Ok(Some(_)) => AttachExit::TakenOver,
            Err(_) => exit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn herdr() -> Herdr {
        Herdr::with_path("/usr/local/bin/herdr")
    }

    #[test]
    fn takeover_follows_the_terminal_id() {
        let cmd = herdr().attach(&Session::named("azj"), &"term_1".into(), true);
        assert_eq!(
            cmd.argv(),
            [
                "/usr/local/bin/herdr",
                "--session",
                "azj",
                "terminal",
                "attach",
                "term_1",
                "--takeover"
            ]
        );
    }

    #[test]
    fn attach_env_has_no_herdr_variables() {
        let cmd = herdr().attach(&Session::named("azj"), &"term_1".into(), false);
        assert!(
            cmd.env
                .iter()
                .all(|(k, _)| !k.to_string_lossy().starts_with("HERDR_"))
        );
        assert_eq!(cmd.argv().last().unwrap(), "term_1");
    }

    #[test]
    fn observe_takes_the_pane_and_size() {
        let cmd = herdr().observe(&Session::default_session(), &"w1:p2".into(), 120, 40);
        assert_eq!(
            cmd.args,
            [
                "terminal", "session", "observe", "w1:p2", "--cols", "120", "--rows", "40"
            ]
        );
    }

    #[test]
    fn stderr_wrapper_keeps_argv_intact() {
        let cmd = herdr()
            .attach(&Session::named("a b"), &"term_1".into(), false)
            .with_stderr_file(Path::new("/tmp/x.err"));
        assert_eq!(cmd.program, PathBuf::from("/bin/sh"));
        assert_eq!(cmd.args[2], "/usr/local/bin/herdr");
        assert_eq!(
            cmd.args[3..],
            ["--session", "a b", "terminal", "attach", "term_1"]
        );
        assert!(
            cmd.env
                .iter()
                .any(|(k, v)| k == "AZJ_ATTACH_STDERR" && v == "/tmp/x.err")
        );
    }

    #[test]
    fn classifies_exit_reasons() {
        use AttachExit::*;
        let c = AttachExit::classify;
        assert_eq!(c(Some(0), None), Detached);
        assert_eq!(
            c(
                Some(1),
                Some("herdr: server shut down: terminal attach taken over\n")
            ),
            TakenOver
        );
        assert_eq!(
            c(
                Some(1),
                Some(
                    "terminal attach failed: terminal term_1 already has an attached client; retry with --takeover"
                )
            ),
            AlreadyAttached
        );
        assert_eq!(
            c(
                Some(1),
                Some("herdr: server shut down: terminal attach ended: terminal term_1 not found")
            ),
            TerminalGone
        );
        assert_eq!(
            c(
                Some(1),
                Some("herdr: server shut down: server is shutting down")
            ),
            ServerStopping
        );
        assert_eq!(
            c(None, None),
            Failed {
                code: None,
                message: String::new()
            }
        );
    }
}
