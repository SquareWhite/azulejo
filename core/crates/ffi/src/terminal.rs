//! What a terminal tile runs, and how to read its end (ADRs 0001, 0003, 0006).
//!
//! The shell's terminal surface (libghostty on macOS) runs one child process:
//! `herdr terminal attach <terminal-id>` to drive the pane, or a read-only
//! `herdr terminal session observe` piped through a frame decoder when another
//! client owns it. The shell can't see the child's exit status reliably
//! (libghostty's macOS `login` wrapper loses it), so the command writes its
//! status and stderr to files the shell names; when the child is gone the
//! shell reads them back through `azj_core_attach_exit`.

use std::ffi::c_char;
use std::path::{Path, PathBuf};
use std::time::Duration;

use azulejo_daemon::HerdrInfo;
use azulejo_herdr::{AttachExit, Backoff, Client, ClientOptions, PaneId, TerminalId};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AzjCore, arg, failure, out};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    /// Attach without `--takeover`; fails if another client owns the pane.
    Attach,
    /// Attach with `--takeover`, replacing whoever owns it.
    Takeover,
    /// Watch read-only at the tile's size.
    Observe,
}

#[derive(Debug, Deserialize)]
struct CommandRequest {
    mode: Mode,
    terminal_id: String,
    #[serde(default)]
    pane_id: Option<String>,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
    /// herdr's stderr goes here (it holds the exit reason).
    stderr_file: PathBuf,
    /// The child's exit status is written here when it ends.
    status_file: PathBuf,
    /// Observe only: the program that turns NDJSON frames into terminal output.
    #[serde(default)]
    decoder: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct ExitRequest {
    terminal_id: String,
    /// The exit status, if the shell found one.
    #[serde(default)]
    status: Option<i32>,
    #[serde(default)]
    stderr: Option<String>,
}

/// Builds the command a terminal tile's surface runs. `request_json`:
///
/// `{"mode":"attach"|"takeover"|"observe","terminal_id":"term_…",
///   "pane_id":"w1:p1","cols":120,"rows":40,"stderr_file":"/…/err",
///   "status_file":"/…/status","decoder":"/…/azj-observe"}`
///
/// (`pane_id`, `cols`, `rows` and `decoder` are for observe.) Returns
/// `{"ok":true,"command":"…","argv":[…]}`, where `command` is one shell
/// command line (libghostty runs its command through a shell) and `argv` the
/// herdr invocation inside it, or `{"ok":false,"error":{"code","message"}}`
/// with code `unavailable` when the core runs without herdr. The command
/// drops inherited `HERDR_*`, `CLAUDE*` and `AZULEJO_*` variables, so the
/// attach client can't follow the user's own herdr socket. Free with
/// `azj_string_free`; NULL only on a bad handle.
///
/// # Safety
/// `core` must come from `azj_core_new*`; `request_json` must be a valid C
/// string or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_terminal_command(
    core: *const AzjCore,
    request_json: *const c_char,
) -> *mut c_char {
    let Some(app) = (unsafe { core.as_ref() }).and_then(|c| c.app.as_ref()) else {
        return std::ptr::null_mut();
    };
    let request: CommandRequest = match unsafe { arg(request_json) }
        .ok_or("request isn't a UTF-8 C string".to_owned())
        .and_then(|text| serde_json::from_str(text).map_err(|e| e.to_string()))
    {
        Ok(request) => request,
        Err(e) => return out(failure("invalid_params", &e)),
    };
    let Some(herdr) = app.herdr() else {
        return out(failure("unavailable", "the core runs without herdr"));
    };
    let inherited: Vec<String> = std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .collect();
    let terminfo = bundle_terminfo();
    out(
        match terminal_command(herdr, &request, &inherited, terminfo.as_deref()) {
            Ok((command, argv)) => json!({"ok": true, "command": command, "argv": argv}),
            Err(message) => failure("invalid_params", &message),
        },
    )
}

/// Reads why a terminal tile's child ended. `request_json`:
/// `{"terminal_id":"term_…","status":1,"stderr":"…"}` (status null when the
/// shell found none). When stderr doesn't say, asks herdr whether the
/// terminal still exists. Returns `{"exit":"detached"|"taken_over"|
/// "already_attached"|"terminal_gone"|"server_stopping"|"failed",
/// "message":"…"}`. Blocks for at most a couple of seconds. Free with
/// `azj_string_free`; NULL only on a bad handle.
///
/// # Safety
/// `core` must come from `azj_core_new*`; `request_json` must be a valid C
/// string or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_attach_exit(
    core: *const AzjCore,
    request_json: *const c_char,
) -> *mut c_char {
    let Some(app) = (unsafe { core.as_ref() }).and_then(|c| c.app.as_ref()) else {
        return std::ptr::null_mut();
    };
    let request: ExitRequest = match unsafe { arg(request_json) }
        .ok_or("request isn't a UTF-8 C string".to_owned())
        .and_then(|text| serde_json::from_str(text).map_err(|e| e.to_string()))
    {
        Ok(request) => request,
        Err(e) => return out(failure("invalid_params", &e)),
    };
    let exit = diagnose(app.herdr(), &request);
    out(exit_json(&exit))
}

fn diagnose(herdr: Option<&HerdrInfo>, request: &ExitRequest) -> AttachExit {
    let exit = AttachExit::classify(request.status, request.stderr.as_deref());
    let (Some(herdr), AttachExit::Failed { .. }) = (herdr, &exit) else {
        return exit;
    };
    // stderr didn't say (or the status went missing): ask herdr.
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return exit;
    };
    let client = Client::new(
        &herdr.socket,
        ClientOptions {
            request_timeout: Duration::from_secs(2),
            connect: Backoff {
                initial: Duration::from_millis(50),
                max: Duration::from_millis(200),
                give_up_after: Duration::from_millis(500),
            },
        },
    );
    let terminal = TerminalId(request.terminal_id.clone());
    if request.status == Some(1) {
        // herdr's own refusals exit 1; with the terminal still there, another
        // client has it (ADR 0003).
        return runtime.block_on(client.diagnose_attach_exit(
            Some(1),
            request.stderr.as_deref(),
            &terminal,
        ));
    }
    // Killed (a signal, or the status went missing with the wrapper): that's
    // never a take-over. Only ask whether the terminal and herdr are still
    // there; if so, the tile reattaches.
    match runtime.block_on(client.resolve_terminal(&terminal)) {
        Err(e) if e.is_unreachable() => AttachExit::ServerStopping,
        Ok(None) => AttachExit::TerminalGone,
        _ => exit,
    }
}

fn exit_json(exit: &AttachExit) -> Value {
    let (kind, message) = match exit {
        AttachExit::Detached => ("detached", String::new()),
        AttachExit::TakenOver => ("taken_over", String::new()),
        AttachExit::AlreadyAttached => ("already_attached", String::new()),
        AttachExit::TerminalGone => ("terminal_gone", String::new()),
        AttachExit::ServerStopping => ("server_stopping", String::new()),
        AttachExit::Failed { message, .. } => ("failed", message.clone()),
    };
    json!({"exit": kind, "message": message})
}

/// `inherited` is the app's environment variable names; `terminfo` the
/// bundle's own terminfo directory, when it ships one.
fn terminal_command(
    herdr: &HerdrInfo,
    request: &CommandRequest,
    inherited: &[String],
    terminfo: Option<&Path>,
) -> Result<(String, Vec<String>), String> {
    let terminal = &request.terminal_id;
    if terminal.is_empty() {
        return Err("terminal_id is empty".into());
    }
    let err = path_str(&request.stderr_file)?;
    let status = path_str(&request.status_file)?;
    let (script, mut words, attach) = match request.mode {
        Mode::Attach | Mode::Takeover => {
            let attach = herdr.attach(terminal, matches!(request.mode, Mode::Takeover));
            (ATTACH_SCRIPT, vec![err, status], attach)
        }
        Mode::Observe => {
            let pane = request
                .pane_id
                .as_deref()
                .filter(|p| !p.is_empty())
                .ok_or("observe needs pane_id")?;
            let decoder = request
                .decoder
                .as_deref()
                .ok_or("observe needs decoder")
                .and_then(|d| path_str(d).map_err(|_| "decoder isn't UTF-8"))?;
            // Never zero: herdr rejects it, and a hidden tile still has a size.
            let cols = request.cols.unwrap_or(80).max(2);
            let rows = request.rows.unwrap_or(24).max(1);
            let attach = herdr
                .herdr
                .observe(&herdr.session, &PaneId(pane.to_owned()), cols, rows);
            (OBSERVE_SCRIPT, vec![err, status, decoder], attach)
        }
    };
    let argv = attach.argv();
    let unset = inherited_to_unset(inherited);
    let mut line: Vec<String> = vec!["/usr/bin/env".into()];
    for name in unset {
        line.push("-u".into());
        line.push(name);
    }
    // Terminfo from the launching environment (e.g. kitty's) never reaches
    // the attach client; only the bundle's own does (ADR 0006).
    if let Some(dir) = terminfo {
        line.push(format!("TERMINFO={}", path_str(dir)?));
    }
    line.extend([
        "/bin/sh".into(),
        "-c".into(),
        script.into(),
        "azj-term".into(),
    ]);
    line.append(&mut words);
    line.extend(argv.iter().cloned());
    Ok((
        line.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" "),
        argv,
    ))
}

/// `$1` stderr file, `$2` status file, then the herdr argv. Never closes the
/// PTY's input (a stray EOF reaches the pane's shell; ADR 0006).
const ATTACH_SCRIPT: &str =
    r#"err=$1; st=$2; shift 2; rm -f "$st"; "$@" 2>>"$err"; printf '%s' "$?" >"$st""#;

/// `$1` stderr file, `$2` status file, `$3` frame decoder, then the herdr argv.
/// Keys typed into an observe tile go nowhere: the shell swallows them.
const OBSERVE_SCRIPT: &str = r#"err=$1; st=$2; dec=$3; shift 3; rm -f "$st"; set -o pipefail 2>/dev/null; "$@" 2>>"$err" | "$dec"; printf '%s' "$?" >"$st""#;

/// Variables the app inherited that the herdr child must not see: `HERDR_*`
/// would point the attach client at whatever herdr session launched the app
/// (the user's own), and `CLAUDE*` / `AZULEJO_*` / `AZJ_*` are an agent's or
/// a dev launcher's, not the terminal's. Everything else, and the TERM
/// variables libghostty sets for its child, stays.
/// `TERMINFO` and `TERMINFO_DIRS` are always dropped: whatever launched the
/// app (kitty, say) set them for its own terminal, not ours.
fn inherited_to_unset(inherited: &[String]) -> Vec<String> {
    let mut unset: Vec<String> = inherited
        .iter()
        .filter(|k| {
            ["HERDR_", "CLAUDE", "AZULEJO_", "AZJ_"]
                .iter()
                .any(|p| k.starts_with(p))
        })
        .cloned()
        .chain(["TERMINFO".to_owned(), "TERMINFO_DIRS".to_owned()])
        .collect();
    unset.sort();
    unset.dedup();
    unset
}

/// `Azulejo.app/Contents/Resources/terminfo` next to the running executable
/// (`Contents/MacOS/…`), if the bundle ships it.
fn bundle_terminfo() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?.join("Resources").join("terminfo");
    dir.is_dir().then_some(dir)
}

fn path_str(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{} isn't UTF-8", path.display()))
}

/// Single-quotes a word for `/bin/sh`.
fn quote(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c))
    {
        return word.to_owned();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use azulejo_herdr::{Herdr, Session};

    fn info() -> HerdrInfo {
        HerdrInfo {
            herdr: Herdr::with_path("/opt/bin/herdr"),
            session: Session::named("azj dev"),
            socket: "/tmp/herdr.sock".into(),
        }
    }

    fn request(mode: Mode) -> CommandRequest {
        CommandRequest {
            mode,
            terminal_id: "term_1".into(),
            pane_id: Some("w1:p2".into()),
            cols: Some(100),
            rows: Some(30),
            stderr_file: "/tmp/t err".into(),
            status_file: "/tmp/t.status".into(),
            decoder: Some("/Apps/Azulejo.app/Contents/MacOS/azj-observe".into()),
        }
    }

    /// What the app might inherit: the user's herdr session and an agent's vars.
    fn inherited() -> Vec<String> {
        ["HERDR_SOCKET_PATH", "CLAUDECODE", "PATH", "HOME"]
            .map(String::from)
            .to_vec()
    }

    /// Runs the command line through `/bin/sh -c` as libghostty would, with
    /// herdr swapped for `echo` so we can see the argv it got.
    fn run(line: &str) -> String {
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(line)
            .env("HERDR_SOCKET_PATH", "/should/not/leak")
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    }

    #[test]
    fn attach_line_round_trips_through_the_shell() {
        let (_, argv) = terminal_command(&info(), &request(Mode::Takeover), &[], None).unwrap();
        assert_eq!(
            argv,
            [
                "/opt/bin/herdr",
                "--session",
                "azj dev",
                "terminal",
                "attach",
                "term_1",
                "--takeover"
            ]
        );
        let dir = std::env::temp_dir().join(format!("azj-ffi-term-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let status = dir.join("status");
        let err = dir.join("err");
        let mut req = request(Mode::Attach);
        req.status_file = status.clone();
        req.stderr_file = err.clone();
        let (line, _) = terminal_command(&info(), &req, &inherited(), None).unwrap();
        // Swap the herdr binary for a probe that prints its argv and env.
        let probe = dir.join("herdr");
        std::fs::write(
            &probe,
            "#!/bin/sh\nprintf '%s|' \"$@\"; printf 'sock=%s' \"${HERDR_SOCKET_PATH-unset}\"; echo oops >&2; exit 7\n",
        )
        .unwrap();
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(&probe)
            .status()
            .unwrap();
        let line = line.replace("/opt/bin/herdr", probe.to_str().unwrap());
        let printed = run(&line);
        assert_eq!(
            printed,
            "--session|azj dev|terminal|attach|term_1|sock=unset"
        );
        assert_eq!(std::fs::read_to_string(&status).unwrap(), "7");
        assert_eq!(std::fs::read_to_string(&err).unwrap(), "oops\n");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(line.starts_with("/usr/bin/env"));
    }

    #[test]
    fn observe_pipes_through_the_decoder() {
        let (line, argv) = terminal_command(&info(), &request(Mode::Observe), &[], None).unwrap();
        assert_eq!(
            argv[3..],
            [
                "terminal", "session", "observe", "w1:p2", "--cols", "100", "--rows", "30"
            ]
        );
        assert!(line.contains("/Apps/Azulejo.app/Contents/MacOS/azj-observe"));
        let mut missing = request(Mode::Observe);
        missing.decoder = None;
        assert!(terminal_command(&info(), &missing, &[], None).is_err());
    }

    /// Terminfo from the launching terminal never reaches the attach client;
    /// the bundle's own does.
    #[test]
    fn launching_terminfo_is_replaced_by_the_bundles() {
        let dir = std::env::temp_dir().join(format!("azj-ffi-ti-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut req = request(Mode::Attach);
        req.status_file = dir.join("status");
        req.stderr_file = dir.join("err");
        let probe = dir.join("herdr");
        std::fs::write(
            &probe,
            "#!/bin/sh\nprintf 'ti=%s dirs=%s' \"${TERMINFO-unset}\" \"${TERMINFO_DIRS-unset}\"\n",
        )
        .unwrap();
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(&probe)
            .status()
            .unwrap();
        let shell = |line: String| {
            let line = line.replace("/opt/bin/herdr", probe.to_str().unwrap());
            let out = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(line)
                .env(
                    "TERMINFO",
                    "/Applications/kitty.app/Contents/Resources/kitty/terminfo",
                )
                .env("TERMINFO_DIRS", "/opt/kitty/terminfo")
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap()
        };
        let (line, _) = terminal_command(&info(), &req, &[], None).unwrap();
        assert_eq!(shell(line), "ti=unset dirs=unset");
        let bundle = Path::new("/Apps/Azulejo.app/Contents/Resources/terminfo");
        let (line, _) = terminal_command(&info(), &req, &[], Some(bundle)).unwrap();
        assert_eq!(
            shell(line),
            "ti=/Apps/Azulejo.app/Contents/Resources/terminfo dirs=unset"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("plain/path-1"), "plain/path-1");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn exits_without_herdr_classify_from_stderr_only() {
        let req = ExitRequest {
            terminal_id: "term_1".into(),
            status: Some(1),
            stderr: Some("herdr: server shut down: terminal attach taken over".into()),
        };
        assert_eq!(diagnose(None, &req), AttachExit::TakenOver);
        assert_eq!(exit_json(&AttachExit::TakenOver)["exit"], "taken_over");
        let unknown = ExitRequest {
            terminal_id: "term_1".into(),
            status: None,
            stderr: None,
        };
        assert!(matches!(
            diagnose(None, &unknown),
            AttachExit::Failed { .. }
        ));
    }
}
