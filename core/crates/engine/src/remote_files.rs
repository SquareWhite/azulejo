//! A remote machine's files for the files panel: one shell
//! command over the machine's shared SSH connection (the same `ssh -T …
//! <target> -- <script>` and control socket the herdr bridge uses)
//! lists one or more folders.
//!
//! Every run happens off the engine's thread, can be cancelled (the panel
//! moved on) and gives up after [`TIMEOUT`], so a slow or unreachable host
//! only ever shows as a loading or error state.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use azulejo_herdr::{AttachCommand, SSH_FAILED, shell_quote};

/// How long one remote command may take before it counts as failed. A new
/// SSH connection may take up to `ConnectTimeout` (10 s) on its own.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(15);

/// Most entries one remote folder listing returns.
pub(crate) const MAX_ENTRIES: usize = 20_000;

/// One entry of a remote folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteEntry {
    pub name: String,
    /// A folder, or a symlink to one.
    pub dir: bool,
    pub symlink: bool,
}

/// A remote folder's entries, hidden ones included, in no order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RemoteDir {
    pub entries: Vec<RemoteEntry>,
    pub truncated: bool,
}

/// The listing script, for `/bin/sh` (the login shell may be zsh or fish,
/// so it gets the script as one quoted argument). `$1` is the most entries
/// per folder, the rest are folders. Output, NUL-separated records of
/// `<tag>\t<text>`: `D` starts a folder, then `d`/`f` (folder or not; `l`
/// first for a symlink) with a name, `E` with why the folder can't be
/// listed, or `T` when it was cut. Only shell builtins run per entry.
const LIST: &str = concat!(
    r#"m=$1; shift; for d in "$@"; do printf 'D\t%s\0' "$d"; "#,
    r#"if [ ! -e "$d" ]; then printf 'E\tno such folder\0'; continue; fi; "#,
    r#"if [ ! -d "$d" ]; then printf 'E\tnot a folder\0'; continue; fi; "#,
    r#"if ! cd -- "$d" 2>/dev/null; then printf 'E\tpermission denied\0'; continue; fi; "#,
    r#"n=0; for f in * .[!.]* ..?*; do "#,
    r#"if [ ! -e "$f" ] && [ ! -L "$f" ]; then continue; fi; "#,
    r#"n=$((n+1)); if [ "$n" -gt "$m" ]; then printf 'T\t\0'; break; fi; "#,
    r#"t=f; if [ -d "$f" ]; then t=d; fi; if [ -L "$f" ]; then t="l$t"; fi; "#,
    r#"printf '%s\t%s\0' "$t" "$f"; done; cd /; done"#,
);

/// The remote command that lists `dirs`.
pub(crate) fn list_script(dirs: &[PathBuf], max: usize) -> String {
    let mut s = format!("exec /bin/sh -c {} azj-ls {max}", shell_quote(LIST));
    for d in dirs {
        s.push(' ');
        s.push_str(&shell_quote(&d.to_string_lossy()));
    }
    s
}

/// A listing run's result: each folder's entries or why not, or why the
/// run itself failed.
pub(crate) type Listed = Result<Vec<(PathBuf, Result<RemoteDir, String>)>, RunError>;

/// Splits the listing script's output by folder. Folders the output never
/// reached (it was cut short) come back as errors.
pub(crate) fn parse_listing(
    out: &[u8],
    dirs: &[PathBuf],
) -> Vec<(PathBuf, Result<RemoteDir, String>)> {
    let mut results: Vec<(PathBuf, Result<RemoteDir, String>)> = Vec::new();
    for record in out.split(|b| *b == 0) {
        let text = String::from_utf8_lossy(record);
        let Some((tag, rest)) = text.split_once('\t') else {
            continue;
        };
        let current = results.last_mut();
        match (tag, current) {
            ("D", _) => results.push((PathBuf::from(rest), Ok(RemoteDir::default()))),
            ("E", Some(last)) => last.1 = Err(rest.to_owned()),
            ("T", Some((_, Ok(dir)))) => dir.truncated = true,
            ("d" | "f" | "ld" | "lf", Some((_, Ok(dir)))) if !rest.is_empty() => {
                dir.entries.push(RemoteEntry {
                    name: rest.to_owned(),
                    dir: tag.ends_with('d'),
                    symlink: tag.starts_with('l'),
                });
            }
            _ => {}
        }
    }
    for d in dirs {
        if !results.iter().any(|(p, _)| p == d) {
            results.push((d.clone(), Err("the listing was cut short".into())));
        }
    }
    results.retain(|(p, _)| dirs.contains(p));
    results
}

/// Why a remote command gave nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RunError {
    /// Stopped on purpose (the panel moved on); say nothing.
    Cancelled,
    TimedOut,
    /// ssh itself failed (exit 255): unreachable, refused, needs a password.
    Unreachable(String),
    /// The remote command failed; its last line of stderr.
    Failed(String),
}

impl RunError {
    /// What a panel shows, with the machine's name.
    pub(crate) fn message(&self, label: &str) -> String {
        match self {
            RunError::Cancelled => "cancelled".into(),
            RunError::TimedOut => format!("{label} didn't answer within {} s", TIMEOUT.as_secs()),
            RunError::Unreachable(why) if why.is_empty() => format!("can't reach {label}"),
            RunError::Unreachable(why) => format!("can't reach {label}: {why}"),
            RunError::Failed(why) => why.clone(),
        }
    }
}

/// Runs `cmd` (an `ssh` command line) and returns its stdout. Gives up after
/// `timeout`, or as soon as `cancel` is set, killing the process.
pub(crate) fn run(
    cmd: &AttachCommand,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Vec<u8>, RunError> {
    let mut child = Command::new(&cmd.program)
        .args(&cmd.args)
        .env_clear()
        .envs(cmd.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| RunError::Failed(format!("can't run {}: {e}", cmd.program.display())))?;
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(RunError::Failed("no pipes to the remote command".into()));
    };
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("azulejo-remote-read".into())
        .spawn(move || {
            let errors = std::thread::spawn(move || {
                let mut err = Vec::new();
                let _ = stderr.read_to_end(&mut err);
                err
            });
            let mut out = Vec::new();
            let _ = stdout.read_to_end(&mut out);
            let err = errors.join().unwrap_or_default();
            let _ = tx.send((out, err));
        });
    if spawned.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(RunError::Failed("couldn't start a reader thread".into()));
    }
    let deadline = Instant::now() + timeout;
    let (out, err) = loop {
        match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(got) => break got,
            Err(mpsc::RecvTimeoutError::Disconnected) => break (Vec::new(), Vec::new()),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let why = if cancel.load(Ordering::Relaxed) {
                    RunError::Cancelled
                } else if Instant::now() >= deadline {
                    RunError::TimedOut
                } else {
                    continue;
                };
                let _ = child.kill();
                let _ = child.wait();
                return Err(why);
            }
        }
    };
    let status = child
        .wait()
        .map_err(|e| RunError::Failed(format!("waiting for ssh: {e}")))?;
    if cancel.load(Ordering::Relaxed) {
        return Err(RunError::Cancelled);
    }
    if status.success() {
        return Ok(out);
    }
    let last = String::from_utf8_lossy(&err)
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or_default()
        .to_owned();
    Err(match status.code() {
        Some(SSH_FAILED) => RunError::Unreachable(last),
        Some(code) if last.is_empty() => RunError::Failed(format!("exit status {code}")),
        None if last.is_empty() => RunError::Failed("killed".into()),
        _ => RunError::Failed(last),
    })
}

/// `path` with `.` and `..` worked out by name, as an absolute path: remote
/// paths can't be resolved on this machine.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::from("/");
    for c in path.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::ParentDir => {
                out.pop();
            }
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a script the way the stand-in does: through `/bin/sh -c`.
    fn sh(script: &str) -> Vec<u8> {
        let out = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    }

    fn names(dir: &RemoteDir) -> Vec<String> {
        let mut v: Vec<String> = dir
            .entries
            .iter()
            .map(|e| match (e.symlink, e.dir) {
                (true, true) => format!("{}@/", e.name),
                (true, false) => format!("{}@", e.name),
                (false, true) => format!("{}/", e.name),
                (false, false) => e.name.clone(),
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn lists_folders_with_odd_names_hidden_entries_and_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a b");
        std::fs::create_dir_all(a.join("sub")).unwrap();
        std::fs::write(a.join("it's.txt"), "x").unwrap();
        std::fs::write(a.join(".env"), "x").unwrap();
        std::fs::write(a.join("..odd"), "x").unwrap();
        std::fs::write(a.join("line\nbreak"), "x").unwrap();
        std::fs::write(a.join("*"), "x").unwrap();
        std::os::unix::fs::symlink(a.join("sub"), a.join("to-sub")).unwrap();
        std::os::unix::fs::symlink(a.join("gone"), a.join("dangling")).unwrap();
        let empty = tmp.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        let missing = tmp.path().join("missing");
        let file = a.join("it's.txt");
        let dirs = vec![a.clone(), empty.clone(), missing.clone(), file.clone()];
        let out = sh(&list_script(&dirs, 100));
        let got = parse_listing(&out, &dirs);
        assert_eq!(got.len(), 4);
        assert_eq!(got[0].0, a);
        assert_eq!(
            names(got[0].1.as_ref().unwrap()),
            vec![
                "*",
                "..odd",
                ".env",
                "dangling@",
                "it's.txt",
                "line\nbreak",
                "sub/",
                "to-sub@/"
            ]
        );
        assert_eq!(names(got[1].1.as_ref().unwrap()), Vec::<String>::new());
        assert_eq!(got[2].1, Err("no such folder".into()));
        assert_eq!(got[3].1, Err("not a folder".into()));
    }

    #[test]
    fn cuts_long_folders() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(tmp.path().join(format!("f{i}")), "").unwrap();
        }
        let dirs = vec![tmp.path().to_path_buf()];
        let got = parse_listing(&sh(&list_script(&dirs, 3)), &dirs);
        let dir = got[0].1.as_ref().unwrap();
        assert_eq!(dir.entries.len(), 3);
        assert!(dir.truncated);
    }

    #[test]
    fn output_cut_short_fails_the_rest() {
        let dirs = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let got = parse_listing(b"D\t/a\0f\tx\0", &dirs);
        assert_eq!(got[0].1.as_ref().unwrap().entries.len(), 1);
        assert_eq!(got[1].1, Err("the listing was cut short".into()));
    }

    fn standin(dir: &Path) -> PathBuf {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/ssh-standin.sh");
        let dst = dir.join("ssh");
        std::fs::copy(src, &dst).unwrap();
        dst
    }

    fn cmd(ssh: &Path, script: String) -> AttachCommand {
        AttachCommand {
            program: ssh.to_path_buf(),
            args: vec!["-T".into(), "standin-host".into(), "--".into(), script],
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        }
    }

    #[test]
    fn runs_through_ssh_and_reports_an_unreachable_host() {
        let tmp = tempfile::tempdir().unwrap();
        let ssh = standin(tmp.path());
        let dirs = vec![tmp.path().to_path_buf()];
        let no = AtomicBool::new(false);
        let out = run(&cmd(&ssh, list_script(&dirs, 10)), &no, TIMEOUT).unwrap();
        let got = parse_listing(&out, &dirs);
        assert!(names(got[0].1.as_ref().unwrap()).contains(&"ssh".to_owned()));

        std::fs::write(tmp.path().join("ssh.fail"), "").unwrap();
        let err = run(&cmd(&ssh, list_script(&dirs, 10)), &no, TIMEOUT).unwrap_err();
        assert!(matches!(&err, RunError::Unreachable(why) if why.contains("Connection refused")));
        assert!(err.message("box").starts_with("can't reach box: "));
    }

    #[test]
    fn slow_hosts_time_out_and_cancelled_runs_stop_at_once() {
        let tmp = tempfile::tempdir().unwrap();
        let ssh = standin(tmp.path());
        let no = AtomicBool::new(false);
        let started = Instant::now();
        let err = run(
            &cmd(&ssh, "sleep 5".into()),
            &no,
            Duration::from_millis(200),
        )
        .unwrap_err();
        assert_eq!(err, RunError::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));

        let yes = AtomicBool::new(true);
        let started = Instant::now();
        let err = run(&cmd(&ssh, "sleep 5".into()), &yes, TIMEOUT).unwrap_err();
        assert_eq!(err, RunError::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn normalizes_by_name() {
        assert_eq!(
            normalize(Path::new("/srv/app/../x/./y")),
            PathBuf::from("/srv/x/y")
        );
        assert_eq!(normalize(Path::new("/..")), PathBuf::from("/"));
    }
}
