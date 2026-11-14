//! herdr's saved machines (ADR 0011): the list comes from herdr's catalogue,
//! connections go through herdr's own `remote-api-bridge` over SSH, and
//! terminal tiles attach with `ssh -tt … herdr terminal attach`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::attach::AttachCommand;
use crate::client::{Backoff, Client, ClientOptions};
use crate::env::clean_process_env;
use crate::error::{Error, Result};
use crate::server::Herdr;
use crate::transport::BridgeCommand;
use crate::types::{PaneId, TerminalId};

/// One saved machine, as `herdr machine list --json` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MachineProfile {
    /// herdr's opaque profile ID (32 hex characters).
    pub id: String,
    pub label: String,
    /// The SSH target (`host`, `user@host`, an `~/.ssh/config` alias).
    pub target: String,
    /// The herdr session on that machine.
    pub session: String,
    pub enabled: bool,
}

/// Parses `herdr machine list --json`.
pub fn parse_machine_list(json: &str) -> Result<Vec<MachineProfile>> {
    let json = json.trim();
    if json.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(json)
        .map_err(|e| Error::Protocol(format!("`herdr machine list --json`: {e}")))
}

impl Herdr {
    /// herdr's saved machines. Only reads herdr's local catalogue; it never
    /// connects (`herdr machine status` would).
    pub async fn machines(&self) -> Result<Vec<MachineProfile>> {
        let out = self
            .cli(
                &crate::Session::default_session(),
                &["machine", "list", "--json"],
            )
            .await?;
        parse_machine_list(&out)
    }
}

/// How to run SSH for remote machines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ssh {
    /// `ssh`, or a stand-in in tests.
    pub program: PathBuf,
    /// Directory for the control sockets that multiplex every connection to
    /// a machine over one SSH session. Must be short (Unix socket paths are
    /// limited to ~104 bytes and ssh adds a suffix).
    pub control_dir: PathBuf,
}

impl Default for Ssh {
    fn default() -> Self {
        // SAFETY: getuid has no preconditions and can't fail.
        let uid = unsafe { libc_getuid() };
        Self {
            program: "ssh".into(),
            control_dir: PathBuf::from(format!("/tmp/azj-{uid}-ssh")),
        }
    }
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

impl Ssh {
    /// Options for every SSH Azulejo runs: herdr's non-interactive set plus
    /// connection sharing.
    pub fn options(&self) -> Vec<String> {
        let mut out = vec!["-C".to_owned()];
        for opt in [
            "BatchMode=yes",
            "NumberOfPasswordPrompts=0",
            "StrictHostKeyChecking=yes",
            "ConnectTimeout=10",
            "ConnectionAttempts=1",
            "ServerAliveInterval=15",
            "ServerAliveCountMax=4",
            "ControlMaster=auto",
            "ControlPersist=600",
        ] {
            out.push("-o".into());
            out.push(opt.into());
        }
        out.push("-o".into());
        out.push(format!(
            "ControlPath={}",
            self.control_dir.join("%C").to_string_lossy()
        ));
        out
    }

    /// Creates the control socket directory (0700).
    pub fn prepare(&self) -> std::io::Result<()> {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.control_dir)?;
        std::fs::set_permissions(&self.control_dir, std::fs::Permissions::from_mode(0o700))
    }
}

/// A saved machine Azulejo talks to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteMachine {
    pub profile: MachineProfile,
    pub ssh: Ssh,
}

/// The shell code that finds herdr on the remote machine. GUI-less SSH
/// sessions get a minimal `PATH`, so the installer's `~/.local/bin` is tried
/// first, like herdr's own discovery.
const FIND_HERDR: &str = r#"h=""; for c in "$HOME/.local/bin/herdr" "$(command -v herdr 2>/dev/null)" /opt/homebrew/bin/herdr /usr/local/bin/herdr /usr/bin/herdr; do if [ -n "$c" ] && [ -x "$c" ]; then h="$c"; break; fi; done; if [ -z "$h" ]; then echo "no usable herdr on this machine" >&2; exit 127; fi"#;

impl RemoteMachine {
    pub fn new(profile: MachineProfile, ssh: Ssh) -> Self {
        Self { profile, ssh }
    }

    fn session_args(&self) -> String {
        format!("--session {}", shell_quote(&self.profile.session))
    }

    /// The remote command for the API bridge: find herdr, check it speaks
    /// `herdr-api-bridge-v1`, then become the bridge.
    pub fn bridge_script(&self) -> String {
        let s = self.session_args();
        format!(
            r#"{FIND_HERDR}; cap=$("$h" {s} remote-api-bridge --check </dev/null 2>/dev/null); if [ "$cap" != herdr-api-bridge-v1 ]; then echo "herdr on this machine is too old for remote-api-bridge" >&2; exit 78; fi; exec "$h" {s} remote-api-bridge"#
        )
    }

    /// The remote command a terminal tile runs.
    pub fn attach_script(&self, terminal: &TerminalId, takeover: bool) -> String {
        let s = self.session_args();
        let t = shell_quote(terminal.as_str());
        let takeover = if takeover { " --takeover" } else { "" };
        format!(r#"{FIND_HERDR}; exec "$h" {s} terminal attach {t}{takeover}"#)
    }

    fn ssh_args(&self, tty: &str, script: String) -> Vec<String> {
        let mut args = self.ssh.options();
        args.push(tty.into());
        args.push(self.profile.target.clone());
        args.push("--".into());
        args.push(script);
        args
    }

    fn env() -> Vec<(OsString, OsString)> {
        // SSH needs the agent socket and HOME; the clean set keeps both.
        clean_process_env(&[])
    }

    /// The bridge every API connection to this machine spawns.
    pub fn bridge(&self) -> BridgeCommand {
        BridgeCommand {
            program: self.ssh.program.clone(),
            args: self.ssh_args("-T", self.bridge_script()),
            env: Self::env(),
            label: format!("machine {}", self.profile.label),
        }
    }

    /// A client for this machine. Connections are SSH channels, so retry
    /// slower than for a local socket.
    pub fn client(&self) -> Client {
        Client::bridged(
            self.bridge(),
            ClientOptions {
                request_timeout: std::time::Duration::from_secs(15),
                connect: Backoff {
                    initial: std::time::Duration::from_secs(1),
                    max: std::time::Duration::from_secs(30),
                    give_up_after: std::time::Duration::ZERO,
                },
            },
        )
    }

    /// `ssh -tt … herdr terminal attach <id> [--takeover]` for a tile's PTY.
    /// ssh exits 255 when it couldn't reach the machine.
    pub fn attach(&self, terminal: &TerminalId, takeover: bool) -> AttachCommand {
        AttachCommand {
            program: self.ssh.program.clone(),
            args: self.ssh_args("-tt", self.attach_script(terminal, takeover)),
            env: Self::env(),
        }
    }

    /// The read-only view (`terminal session observe`) over SSH.
    pub fn observe(&self, pane: &PaneId, cols: u16, rows: u16) -> AttachCommand {
        let s = self.session_args();
        let script = format!(
            r#"{FIND_HERDR}; exec "$h" {s} terminal session observe {} --cols {cols} --rows {rows}"#,
            shell_quote(pane.as_str())
        );
        AttachCommand {
            program: self.ssh.program.clone(),
            args: self.ssh_args("-T", script),
            env: Self::env(),
        }
    }

    pub fn control_dir(&self) -> &Path {
        &self.ssh.control_dir
    }
}

/// Exit code ssh uses for its own failures (unreachable, auth).
pub const SSH_FAILED: i32 = 255;

/// Single-quotes `s` for a POSIX shell.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '@'))
    {
        return s.to_owned();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As herdr v0.9.5's `MachineListRow` serialises it.
    const FIXTURE: &str = r#"[
  {
    "id": "0123456789abcdef0123456789abcdef",
    "label": "build-server",
    "target": "build-host",
    "session": "default",
    "enabled": true,
    "selected": false
  },
  {
    "id": "fedcba9876543210fedcba9876543210",
    "label": "dev-server",
    "target": "user@dev-server",
    "session": "work",
    "enabled": false,
    "selected": true
  }
]"#;

    fn machine() -> RemoteMachine {
        RemoteMachine::new(
            parse_machine_list(FIXTURE).unwrap().remove(0),
            Ssh {
                program: "ssh".into(),
                control_dir: "/tmp/azj-501-ssh".into(),
            },
        )
    }

    #[test]
    fn parses_herdrs_machine_list() {
        let list = parse_machine_list(FIXTURE).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].label, "build-server");
        assert_eq!(list[0].target, "build-host");
        assert!(list[0].enabled);
        assert_eq!(list[1].session, "work");
        assert!(!list[1].enabled);
        assert!(parse_machine_list("").unwrap().is_empty());
        assert!(parse_machine_list("[]").unwrap().is_empty());
        assert!(parse_machine_list("nope").is_err());
    }

    #[test]
    fn bridge_runs_herdrs_remote_api_bridge_over_ssh() {
        let b = machine().bridge();
        assert_eq!(b.program, PathBuf::from("ssh"));
        let args = &b.args;
        let target = args.iter().position(|a| a == "build-host").unwrap();
        assert_eq!(args[target - 1], "-T");
        assert_eq!(args[target + 1], "--");
        let script = &args[target + 2];
        assert!(script.contains("remote-api-bridge --check"));
        assert!(script.ends_with(r#"exec "$h" --session default remote-api-bridge"#));
        for opt in [
            "BatchMode=yes",
            "StrictHostKeyChecking=yes",
            "ControlMaster=auto",
            "ControlPersist=600",
            "ControlPath=/tmp/azj-501-ssh/%C",
        ] {
            assert!(args.iter().any(|a| a == opt), "missing {opt}");
        }
        assert!(
            b.env
                .iter()
                .all(|(k, _)| !k.to_string_lossy().starts_with("HERDR_"))
        );
    }

    #[test]
    fn attach_uses_a_tty_and_puts_takeover_after_the_id() {
        let cmd = machine().attach(&"term_1".into(), true);
        let target = cmd.args.iter().position(|a| a == "build-host").unwrap();
        assert_eq!(cmd.args[target - 1], "-tt");
        assert!(
            cmd.args[target + 2]
                .ends_with(r#"exec "$h" --session default terminal attach term_1 --takeover"#)
        );
        let plain = machine().attach(&"term_1".into(), false);
        assert!(
            plain
                .args
                .last()
                .unwrap()
                .ends_with("terminal attach term_1")
        );
    }

    #[test]
    fn observe_runs_without_a_tty() {
        let cmd = machine().observe(&"w1:p2".into(), 80, 24);
        let script = cmd.args.last().unwrap();
        assert!(script.ends_with("terminal session observe w1:p2 --cols 80 --rows 24"));
        assert!(cmd.args.iter().any(|a| a == "-T"));
    }

    #[test]
    fn quotes_for_the_remote_shell() {
        assert_eq!(shell_quote("work"), "work");
        assert_eq!(shell_quote("my session"), "'my session'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
        let mut m = machine();
        m.profile.session = "a b; rm -rf ~".into();
        assert!(m.bridge_script().contains("--session 'a b; rm -rf ~'"));
    }

    #[test]
    fn the_scripts_run_in_a_posix_shell() {
        // A machine without herdr says so on stderr and exits 127.
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(machine().bridge_script())
            .env_clear()
            .env("HOME", "/nonexistent")
            .env("PATH", "/nonexistent")
            .output()
            .unwrap();
        // /usr/bin/herdr etc. don't exist on CI or this Mac.
        if out.status.code() == Some(127) {
            assert!(String::from_utf8_lossy(&out.stderr).contains("no usable herdr"));
        }
    }
}
