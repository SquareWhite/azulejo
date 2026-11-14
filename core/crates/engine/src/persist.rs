//! Saving and loading the session.
//!
//! The state file is `{"version": N, "session": {…}}`, written atomically
//! (temp file, fsync, rename, fsync the directory) so a crash mid-write leaves
//! the previous file intact. [`Saver`] debounces writes: a burst of changes
//! costs one write, 250 ms after the last one.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use azulejo_model::Session;
use serde_json::{Value, json};

/// Version of the state file format. Bump it and extend [`migrate`] when the
/// saved shape changes incompatibly.
pub const STATE_VERSION: u32 = 1;

/// How long the session must stay unchanged before it's written.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(250);

/// Where the session lives: `~/Library/Application Support/Azulejo/state.json`
/// on macOS, `$XDG_STATE_HOME/azulejo/state.json` (default
/// `~/.local/state/azulejo/state.json`) elsewhere.
pub fn default_state_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Azulejo/state.json")
    } else {
        let base = std::env::var_os("XDG_STATE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/state"));
        base.join("azulejo/state.json")
    }
}

/// What [`load`] found.
#[derive(Debug)]
pub enum Loaded {
    /// No state file yet.
    Missing,
    Session(Session),
    /// The file was unreadable or invalid. It was moved to `backup` so the
    /// next save doesn't overwrite it; start fresh.
    Corrupt {
        error: String,
        backup: PathBuf,
    },
}

/// Reads the state file at `path`.
pub fn load(path: &Path) -> io::Result<Loaded> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Loaded::Missing),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            return quarantine(path, format!("not UTF-8: {e}"));
        }
        Err(e) => return Err(e),
    };
    match parse(&text) {
        Ok(session) => Ok(Loaded::Session(session)),
        Err(error) => quarantine(path, error),
    }
}

fn parse(text: &str) -> Result<Session, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("bad JSON: {e}"))?;
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .ok_or("missing `version`")? as u32;
    let value = migrate(version, value)?;
    let session = value.get("session").cloned().ok_or("missing `session`")?;
    let session: Session =
        serde_json::from_value(session).map_err(|e| format!("bad session: {e}"))?;
    session
        .check_invariants()
        .map_err(|e| format!("broken session: {e}"))?;
    Ok(session)
}

/// Upgrades a state file from `version` to [`STATE_VERSION`], one step at a
/// time. Each future format change adds an arm.
fn migrate(version: u32, value: Value) -> Result<Value, String> {
    match version {
        STATE_VERSION => Ok(value),
        v if v > STATE_VERSION => Err(format!(
            "state file version {v} is newer than this build ({STATE_VERSION})"
        )),
        v => Err(format!("no migration from state file version {v}")),
    }
}

/// Moves a bad state file aside as `<name>.bak`, replacing an older backup.
fn quarantine(path: &Path, error: String) -> io::Result<Loaded> {
    let backup = path.with_extension("json.bak");
    fs::rename(path, &backup)?;
    Ok(Loaded::Corrupt { error, backup })
}

/// Writes `session` to `path` atomically, creating the directory if needed.
pub fn save(path: &Path, session: &Session) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let text = serde_json::to_vec_pretty(&json!({
        "version": STATE_VERSION,
        "session": session,
    }))
    .map_err(io::Error::other)?;

    let tmp = path.with_extension("json.tmp");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        file.write_all(&text)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    // Make the rename itself durable.
    File::open(dir)?.sync_all()?;
    Ok(())
}

enum SaverMsg {
    Save(Box<Session>),
    Flush(mpsc::Sender<()>),
}

/// Writes the session on a background thread, debounced by [`SAVE_DEBOUNCE`].
/// Dropping it writes anything pending first.
pub struct Saver {
    tx: Option<mpsc::Sender<SaverMsg>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Saver {
    /// `on_error` is called (on the saver thread) when a write fails; the
    /// next change retries.
    pub fn new(path: PathBuf, on_error: impl Fn(io::Error) + Send + 'static) -> Self {
        Self::with_debounce(path, SAVE_DEBOUNCE, on_error)
    }

    pub fn with_debounce(
        path: PathBuf,
        debounce: Duration,
        on_error: impl Fn(io::Error) + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("azulejo-saver".into())
            .spawn(move || run_saver(&path, debounce, &rx, &on_error))
            .expect("spawn saver thread");
        Self {
            tx: Some(tx),
            thread: Some(thread),
        }
    }

    /// Queues `session` for writing. Later calls within the debounce window
    /// replace it.
    pub fn schedule(&self, session: Session) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(SaverMsg::Save(Box::new(session)));
        }
    }

    /// Writes anything pending now and waits for it.
    pub fn flush(&self) {
        if let Some(tx) = &self.tx {
            let (done, wait) = mpsc::channel();
            if tx.send(SaverMsg::Flush(done)).is_ok() {
                let _ = wait.recv();
            }
        }
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        // Closing the channel makes the thread write what's pending and exit.
        self.tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_saver(
    path: &Path,
    debounce: Duration,
    rx: &mpsc::Receiver<SaverMsg>,
    on_error: &dyn Fn(io::Error),
) {
    let mut pending: Option<Box<Session>> = None;
    let write = |pending: &mut Option<Box<Session>>| {
        if let Some(session) = pending.take()
            && let Err(e) = save(path, &session)
        {
            on_error(e);
        }
    };
    loop {
        let msg = if pending.is_some() {
            match rx.recv_timeout(debounce) {
                Ok(msg) => msg,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    write(&mut pending);
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(msg) => msg,
                Err(_) => break,
            }
        };
        match msg {
            SaverMsg::Save(session) => pending = Some(session),
            SaverMsg::Flush(done) => {
                write(&mut pending);
                let _ = done.send(());
            }
        }
    }
    write(&mut pending);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;
    use azulejo_model::Direction;
    use azulejo_protocol::{Command, LayoutSplit};

    fn sample() -> Session {
        let mut engine = Engine::new();
        engine
            .execute(Command::LayoutSplit(LayoutSplit {
                tile: None,
                direction: Direction::Right,
                content: None,
            }))
            .unwrap();
        engine.session().clone()
    }

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/state.json");
        let session = sample();
        save(&path, &session).unwrap();
        let Loaded::Session(loaded) = load(&path).unwrap() else {
            panic!("expected a session");
        };
        assert_eq!(loaded, session);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            load(&dir.path().join("state.json")).unwrap(),
            Loaded::Missing
        ));
    }

    #[test]
    fn corrupt_file_is_kept_as_bak() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        for bad in [
            "{not json",
            r#"{"session": {}}"#,
            r#"{"version": 99, "session": {}}"#,
            r#"{"version": 1, "session": {"windows": 3}}"#,
        ] {
            fs::write(&path, bad).unwrap();
            let Loaded::Corrupt { backup, error } = load(&path).unwrap() else {
                panic!("expected corrupt for {bad}");
            };
            assert!(!error.is_empty());
            assert_eq!(fs::read_to_string(&backup).unwrap(), bad);
            assert!(!path.exists());
        }
    }

    #[test]
    fn broken_invariants_count_as_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut session = sample();
        // Point focus at a tile that doesn't exist.
        session.windows[0].workspaces[0].tabs[0].focused = azulejo_model::TileId::new();
        save(&path, &session).unwrap();
        assert!(matches!(load(&path).unwrap(), Loaded::Corrupt { .. }));
    }

    #[test]
    fn stale_temp_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let session = sample();
        save(&path, &session).unwrap();
        // A crash mid-write leaves a half-written temp file behind.
        fs::write(path.with_extension("json.tmp"), "{\"version\": 1, \"sess").unwrap();
        let Loaded::Session(loaded) = load(&path).unwrap() else {
            panic!("expected a session");
        };
        assert_eq!(loaded, session);
        // The next save replaces it.
        save(&path, &session).unwrap();
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn saver_debounces_bursts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let saver = Saver::with_debounce(path.clone(), Duration::from_millis(100), |e| {
            panic!("save failed: {e}")
        });
        let first = Session::new();
        let last = sample();
        saver.schedule(first);
        saver.schedule(last.clone());
        // Nothing written inside the window.
        thread::sleep(Duration::from_millis(30));
        assert!(!path.exists());
        thread::sleep(Duration::from_millis(250));
        let Loaded::Session(loaded) = load(&path).unwrap() else {
            panic!("expected a session");
        };
        assert_eq!(loaded, last);
    }

    #[test]
    fn saver_writes_pending_on_drop_and_flush() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let session = sample();
        {
            let saver = Saver::with_debounce(path.clone(), Duration::from_secs(60), |e| {
                panic!("save failed: {e}")
            });
            saver.schedule(Session::new());
            saver.flush();
            assert!(matches!(load(&path).unwrap(), Loaded::Session(_)));
            saver.schedule(session.clone());
        }
        let Loaded::Session(loaded) = load(&path).unwrap() else {
            panic!("expected a session");
        };
        assert_eq!(loaded, session);
    }
}
