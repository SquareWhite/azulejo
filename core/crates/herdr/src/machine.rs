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

/// The shell code that picks the remote herdr and leaves its path in `$h`.
///
/// The order follows herdr's own lookup for saved machines
/// (`remote_binary_candidates` in herdr's `src/remote/attach.rs`): `PATH`
/// first, then `~/.local/bin` (where herdr installs itself on a remote), then
/// Homebrew, mise and Nix. Like herdr, it takes the first one whose
/// `remote-api-bridge --check` prints `herdr-api-bridge-v1`, so a stale copy
/// early in the list can't hide a good one. The bridge, `terminal attach` and
/// `terminal session observe` all run this, so they always pick the same
/// binary. It costs one `--check` per candidate tried, no SSH round trips.
///
/// It runs in the remote user's login shell, often zsh, so no globs (zsh
/// fails the whole command on one that matches nothing): mise's `latest`
/// symlink stands in for herdr's exact-version mise paths.
///
/// On failure it prints one line (the bridge reports its last stderr line and
/// the sidebar shows it as the machine's tooltip) that
/// [`crate::needs_attention`] recognises: exit 127 when there's no herdr at
/// all, 78 with every herdr it found and its version when none has the
/// bridge. `remote-api-bridge` came with `herdr --machine` in 0.9.1.
const FIND_HERDR: &str = concat!(
    r#"h=""; seen="|"; bad=""; "#,
    r#"for c in "$(command -v herdr 2>/dev/null)" "$HOME/.local/bin/herdr" "#,
    r#"/opt/homebrew/bin/herdr /usr/local/bin/herdr /home/linuxbrew/.linuxbrew/bin/herdr "#,
    r#""$HOME/.local/share/mise/installs/herdr/latest/bin/herdr" "#,
    r#""$HOME/.local/share/mise/installs/herdr/latest/herdr" "#,
    r#""$HOME/.local/share/mise/installs/github-ogulcancelik-herdr/latest/herdr" "#,
    r#""$HOME/.nix-profile/bin/herdr" "/etc/profiles/per-user/${USER:-}/bin/herdr" "#,
    r#"/nix/var/nix/profiles/default/bin/herdr /run/current-system/sw/bin/herdr /usr/bin/herdr; do "#,
    // Absolute paths only, never mise's shim (herdr skips it too), each once.
    r#"case $c in */mise/shims/herdr) continue;; /*) ;; *) continue;; esac; "#,
    r#"case $seen in *"|$c|"*) continue;; esac; seen="$seen$c|"; "#,
    r#"[ -f "$c" ] && [ -x "$c" ] || continue; "#,
    r#"case $("$c" remote-api-bridge --check </dev/null 2>/dev/null) in *herdr-api-bridge-v1*) h=$c; break;; esac; "#,
    // Not this one: note it for the error as `~/.local/bin/herdr 0.8.2`.
    r#"v=$("$c" --version </dev/null 2>/dev/null | head -n 1); v=${v#herdr }; "#,
    r#"d=$c; if [ -n "$HOME" ]; then case $c in "$HOME"/*) d="~${c#"$HOME"}";; esac; fi; "#,
    r#"bad="$bad${bad:+, }$d ${v:-(no version)}"; "#,
    r#"done; "#,
    r#"if [ -z "$h" ]; then "#,
    r#"if [ -z "$bad" ]; then echo "no usable herdr on this machine (looked on PATH, in ~/.local/bin, Homebrew, mise and Nix)" >&2; exit 127; fi; "#,
    r#"echo "herdr on this machine is too old for Azulejo: $bad; needs herdr 0.9.1+ (remote-api-bridge)" >&2; exit 78; "#,
    r#"fi"#,
);

impl RemoteMachine {
    pub fn new(profile: MachineProfile, ssh: Ssh) -> Self {
        Self { profile, ssh }
    }

    fn session_args(&self) -> String {
        format!("--session {}", shell_quote(&self.profile.session))
    }

    /// The remote command for the API bridge: find a herdr that speaks
    /// `herdr-api-bridge-v1`, then become the bridge.
    pub fn bridge_script(&self) -> String {
        let s = self.session_args();
        format!(r#"{FIND_HERDR}; exec "$h" {s} remote-api-bridge"#)
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
    fn every_script_picks_herdr_the_same_way_herdr_does() {
        let m = machine();
        let scripts = [
            m.bridge_script(),
            m.attach_script(&"term_1".into(), false),
            m.observe(&"w1:p2".into(), 80, 24)
                .args
                .last()
                .unwrap()
                .clone(),
        ];
        for script in &scripts {
            assert!(script.starts_with(FIND_HERDR), "{script}");
            // One check, inside the shared lookup: nothing else picks a binary.
            assert_eq!(script.matches("remote-api-bridge --check").count(), 1);
        }
        // herdr's order: PATH, its install dir, then the package managers.
        let at = |needle: &str| FIND_HERDR.find(needle).unwrap();
        assert!(at("command -v herdr") < at(r#""$HOME/.local/bin/herdr""#));
        assert!(at(r#""$HOME/.local/bin/herdr""#) < at("/opt/homebrew/bin/herdr"));
        assert!(at("/opt/homebrew/bin/herdr") < at("/usr/bin/herdr"));
        for path in [
            "/home/linuxbrew/.linuxbrew/bin/herdr",
            "/.nix-profile/bin/herdr",
            "/mise/installs/herdr/",
            "/run/current-system/sw/bin/herdr",
        ] {
            assert!(FIND_HERDR.contains(path), "missing {path}");
        }
        assert!(!FIND_HERDR.contains('\n'));
    }

    /// The fixed system paths the lookup tries. If one holds a real herdr
    /// on the test machine, the "nothing qualifies" cases can't be staged.
    fn system_herdr() -> Option<&'static str> {
        [
            "/opt/homebrew/bin/herdr",
            "/usr/local/bin/herdr",
            "/home/linuxbrew/.linuxbrew/bin/herdr",
            "/nix/var/nix/profiles/default/bin/herdr",
            "/run/current-system/sw/bin/herdr",
            "/usr/bin/herdr",
        ]
        .into_iter()
        .find(|p| Path::new(p).exists())
    }

    /// A fake remote: `$HOME` and a `PATH` dir in a temp dir, to hold stub
    /// herdrs, reached through scripts/ssh-standin.sh.
    struct FakeRemote {
        dir: tempfile::TempDir,
    }

    impl FakeRemote {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("home/.local/bin")).unwrap();
            std::fs::create_dir_all(dir.path().join("bin")).unwrap();
            Self { dir }
        }

        fn home(&self) -> PathBuf {
            self.dir.path().join("home")
        }

        fn path_dir(&self) -> PathBuf {
            self.dir.path().join("bin")
        }

        /// A stub herdr at `at`: prints `name` and its arguments, reports
        /// `version`, and passes `remote-api-bridge --check` if `bridge`.
        fn herdr(&self, at: &Path, name: &str, version: &str, bridge: bool) {
            use std::os::unix::fs::PermissionsExt;
            let check = if bridge {
                "echo herdr-api-bridge-v1"
            } else {
                "echo \"error: unrecognized subcommand 'remote-api-bridge'\" >&2; exit 2"
            };
            std::fs::write(
                at,
                format!(
                    "#!/bin/sh\ncase \"$*\" in\n\"remote-api-bridge --check\") {check} ;;\n--version) echo 'herdr {version}' ;;\n*) echo \"{name} $*\" ;;\nesac\n"
                ),
            )
            .unwrap();
            std::fs::set_permissions(at, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        /// Runs ssh `args` on this fake remote: through the ssh stand-in
        /// for `STANDIN`, else as `<shell> -c <script>`, like sshd hands the
        /// script to the user's login shell.
        fn run(&self, shell: &str, args: &[String]) -> (Option<i32>, String, String) {
            let mut cmd = if shell == STANDIN {
                let mut c = std::process::Command::new("/bin/sh");
                c.arg(
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/ssh-standin.sh"),
                )
                .args(args);
                c
            } else {
                let mut c = std::process::Command::new(shell);
                c.arg("-c").arg(args.last().unwrap());
                c
            };
            let out = cmd
                .env_clear()
                .env("HOME", self.home())
                .env(
                    "PATH",
                    format!("{}:/usr/bin:/bin", self.path_dir().display()),
                )
                .output()
                .unwrap();
            (
                out.status.code(),
                String::from_utf8_lossy(&out.stdout).into_owned(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
            )
        }
    }

    /// Runs the script through the ssh stand-in.
    const STANDIN: &str = "ssh-standin";

    /// The stand-in plus every login shell on this machine the script must
    /// survive (sshd runs it with the remote user's shell, often zsh).
    fn shells() -> Vec<&'static str> {
        let mut out = vec![STANDIN];
        out.extend(
            [
                "/bin/sh",
                "/bin/bash",
                "/bin/zsh",
                "/bin/dash",
                "/usr/bin/dash",
            ]
            .into_iter()
            .filter(|s| Path::new(s).exists()),
        );
        out
    }

    #[test]
    fn stale_herdrs_early_in_line_are_skipped_for_one_with_the_bridge() {
        let fake = FakeRemote::new();
        // PATH and ~/.local/bin hold old herdrs; the good one is in mise.
        fake.herdr(&fake.path_dir().join("herdr"), "old", "0.8.2", false);
        fake.herdr(
            &fake.home().join(".local/bin/herdr"),
            "older",
            "0.8.0",
            false,
        );
        let mise = fake
            .home()
            .join(".local/share/mise/installs/herdr/latest/bin");
        std::fs::create_dir_all(&mise).unwrap();
        fake.herdr(&mise.join("herdr"), "new", "0.9.5", true);
        let m = machine();

        for shell in shells() {
            let (code, out, err) = fake.run(shell, &m.bridge().args);
            assert_eq!(code, Some(0), "{shell}: {err}");
            assert_eq!(
                out.trim(),
                "new --session default remote-api-bridge",
                "{shell}"
            );

            // Tiles and the observe view use that same binary.
            let (code, out, err) = fake.run(shell, &m.attach(&"term_1".into(), true).args);
            assert_eq!(code, Some(0), "{shell}: {err}");
            assert_eq!(
                out.trim(),
                "new --session default terminal attach term_1 --takeover",
                "{shell}"
            );
            let (code, out, err) = fake.run(shell, &m.observe(&"w1:p2".into(), 80, 24).args);
            assert_eq!(code, Some(0), "{shell}: {err}");
            assert_eq!(
                out.trim(),
                "new --session default terminal session observe w1:p2 --cols 80 --rows 24",
                "{shell}"
            );
        }
    }

    #[test]
    fn when_several_herdrs_qualify_path_wins_as_in_herdr() {
        let fake = FakeRemote::new();
        fake.herdr(&fake.path_dir().join("herdr"), "on-path", "0.9.5", true);
        fake.herdr(
            &fake.home().join(".local/bin/herdr"),
            "installed",
            "0.9.5",
            true,
        );
        for shell in shells() {
            let (code, out, err) = fake.run(shell, &machine().bridge().args);
            assert_eq!(code, Some(0), "{shell}: {err}");
            assert!(out.starts_with("on-path "), "{shell}: {out}");
        }
    }

    #[test]
    fn with_no_herdr_that_has_the_bridge_it_lists_what_it_tried() {
        if let Some(p) = system_herdr() {
            eprintln!("skipped: {p} exists on this machine");
            return;
        }
        let fake = FakeRemote::new();
        fake.herdr(&fake.path_dir().join("herdr"), "old", "0.9.0", false);
        fake.herdr(
            &fake.home().join(".local/bin/herdr"),
            "older",
            "0.8.2",
            false,
        );
        for shell in shells() {
            let (code, out, err) = fake.run(shell, &machine().bridge().args);
            assert_eq!(code, Some(78), "{shell}: {out}");
            assert!(out.is_empty(), "{shell}: {out}");
            assert_eq!(
                err.trim(),
                format!(
                    "herdr on this machine is too old for Azulejo: {}/herdr 0.9.0, \
                     ~/.local/bin/herdr 0.8.2; needs herdr 0.9.1+ (remote-api-bridge)",
                    fake.path_dir().display()
                ),
                "{shell}"
            );
            assert!(crate::needs_attention(&err));
        }

        // On PATH and in ~/.local/bin at once: listed once. A herdr that
        // can't say its version still shows up.
        let fake = FakeRemote::new();
        let at = fake.home().join(".local/bin/herdr");
        fake.herdr(&at, "older", "0.8.2", false);
        let stub = std::fs::read_to_string(&at).unwrap();
        std::fs::write(&at, stub.replace("echo 'herdr 0.8.2'", "exit 1")).unwrap();
        let mut args = machine().bridge().args;
        let script = args.pop().unwrap();
        args.push(format!("PATH=\"$HOME/.local/bin:$PATH\"; {script}"));
        for shell in shells() {
            let (code, _, err) = fake.run(shell, &args);
            assert_eq!(code, Some(78), "{shell}: {err}");
            assert!(
                err.contains("Azulejo: ~/.local/bin/herdr (no version); needs"),
                "{shell}: {err}"
            );
        }
    }

    #[test]
    fn with_no_herdr_at_all_it_says_so() {
        if let Some(p) = system_herdr() {
            eprintln!("skipped: {p} exists on this machine");
            return;
        }
        let fake = FakeRemote::new();
        // Neither a non-executable file nor a directory counts.
        std::fs::write(fake.path_dir().join("herdr"), "").unwrap();
        std::fs::create_dir(fake.home().join(".local/bin/herdr")).unwrap();
        for shell in shells() {
            let (code, _, err) = fake.run(shell, &machine().attach(&"term_1".into(), false).args);
            assert_eq!(code, Some(127), "{shell}: {err}");
            assert!(
                err.trim().starts_with("no usable herdr on this machine"),
                "{shell}: {err}"
            );
            assert_eq!(err.trim().lines().count(), 1, "{shell}: {err}");
            assert!(crate::needs_attention(&err));
        }
    }
}
