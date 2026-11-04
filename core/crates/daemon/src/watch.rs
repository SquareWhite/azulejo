//! Config hot reload (F25b): watch the config file and the user themes
//! beside it (`themes/*.toml`), read them again on save, and tell every
//! listener with `config.reloaded`.
//!
//! A file with errors never replaces a working config: the previous one
//! stays and the event carries the diagnostics for the shell's banner.
//! Warnings alone don't block a reload.

use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, RwLock, Weak};
use std::thread;
use std::time::Duration;

use azulejo_config::{Config, Diagnostic, Platform, Severity, Source};
use azulejo_engine::{Core, LogFn};
use azulejo_protocol::{ConfigDiagnostic, ConfigReloaded, Event};
use notify::{RecursiveMode, Watcher};

/// The config everyone reads; swapped whole on reload.
pub type SharedConfig = Arc<RwLock<Arc<Config>>>;

/// Editors save in bursts (write, rename, chmod); wait this long for quiet.
const SETTLE: Duration = Duration::from_millis(100);

/// Watches until dropped.
pub struct ConfigWatch {
    _watcher: notify::RecommendedWatcher,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for ConfigWatch {
    fn drop(&mut self) {
        // Dropping the sender ends the thread's wait.
        self.stop.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Whether a changed path is the config file or a user theme beside it.
/// By name: FSEvents reports resolved paths (`/private/var/…` for
/// `/var/…`), and a stray match only costs a snapshot compare.
fn relevant(changed: &Path, config: &Path) -> bool {
    let themes = |p: &Path| p.file_name().is_some_and(|n| n == "themes");
    changed.file_name() == config.file_name()
        || themes(changed)
        || (changed.parent().is_some_and(themes)
            && changed.extension().is_some_and(|e| e == "toml"))
}

/// What a save has to change for a reload: the config file and every user
/// theme, as text.
fn snapshot(config: &Path) -> Option<String> {
    let mut out = std::fs::read_to_string(config).ok().unwrap_or_default();
    if let Some(dir) = config.parent().map(|d| d.join("themes"))
        && let Ok(entries) = std::fs::read_dir(&dir)
    {
        let mut files: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        files.sort();
        for file in files {
            out.push('\0');
            out.push_str(&file.display().to_string());
            out.push('\0');
            out.push_str(&std::fs::read_to_string(&file).unwrap_or_default());
        }
    }
    Some(out)
}

/// Starts watching `path` (and `themes/` beside it). Returns `None` (and
/// logs) when its directory doesn't exist or can't be watched; the config
/// then only changes on restart.
pub fn watch(
    path: &Path,
    platform: Platform,
    shared: SharedConfig,
    core: Weak<Core>,
    log: LogFn,
) -> Option<ConfigWatch> {
    let dir = path.parent()?.to_path_buf();
    let config = path.to_path_buf();
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher =
        match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res
                && event.paths.iter().any(|p| relevant(p, &config))
            {
                let _ = tx.send(());
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                log(format!("config: not watching for changes: {e}"));
                return None;
            }
        };
    // The directory, not the file: editors replace the file on save. Its
    // subfolders too, for `themes/` (which may not exist yet).
    if let Err(e) = watcher.watch(&dir, RecursiveMode::Recursive) {
        log(format!(
            "config: not watching {} for changes: {e}",
            dir.display()
        ));
        return None;
    }
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let path = path.to_path_buf();
    let thread = thread::Builder::new()
        .name("azulejo-config-watch".into())
        .spawn(move || run(&path, platform, &shared, &core, &log, &rx, &stop_rx))
        .ok()?;
    Some(ConfigWatch {
        _watcher: watcher,
        stop: Some(stop_tx),
        thread: Some(thread),
    })
}

fn run(
    path: &Path,
    platform: Platform,
    shared: &SharedConfig,
    core: &Weak<Core>,
    log: &LogFn,
    changes: &mpsc::Receiver<()>,
    stop: &mpsc::Receiver<()>,
) {
    let mut last = snapshot(path);
    loop {
        // Wake up for a change, or now and then to notice `stop`.
        match changes.recv_timeout(Duration::from_millis(250)) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) => {
                if matches!(stop.try_recv(), Err(mpsc::TryRecvError::Disconnected)) {
                    return;
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
        // Let the save settle, swallowing the rest of the burst.
        while changes.recv_timeout(SETTLE).is_ok() {}
        let text = snapshot(path);
        if text == last {
            continue; // touched, not changed (or a replayed event)
        }
        last = text;
        let Some(event) = reload(path, platform, shared, log) else {
            continue;
        };
        match core.upgrade() {
            Some(core) => {
                push_settings(&core, shared);
                core.broadcast(Event::ConfigReloaded(event));
            }
            None => return,
        }
    }
}

/// Hands the core the settings it acts on itself (`[terminal] on_exit`,
/// `[layout]`), from the config currently in force.
pub fn push_settings(core: &Core, shared: &SharedConfig) {
    if let Ok(config) = shared.read() {
        core.set_on_exit(config.terminal.on_exit);
        core.set_layout_prefs(config.layout.clone());
        core.set_sidebar_collapsed(config.sidebar.collapsed);
    }
}

/// Reads `path` again and swaps it in unless it has errors. `None` when the
/// lock is poisoned (a panic elsewhere); nothing changes then.
pub fn reload(
    path: &Path,
    platform: Platform,
    shared: &SharedConfig,
    log: &LogFn,
) -> Option<ConfigReloaded> {
    let fresh = azulejo_config::reload(path, platform);
    let applied = !fresh.has_errors();
    let mut diagnostics: Vec<ConfigDiagnostic> = fresh.diagnostics.iter().map(diagnostic).collect();
    {
        let mut current = shared.write().ok()?;
        // Per-machine settings (`[herdr.machine.*]`) apply live; the rest
        // of `[herdr]` picks connections made at start.
        let restart_only = |h: &azulejo_config::HerdrConfig| azulejo_config::HerdrConfig {
            machine: Default::default(),
            ..h.clone()
        };
        if applied && restart_only(&current.herdr) != restart_only(&fresh.herdr) {
            diagnostics.push(ConfigDiagnostic {
                severity: "warning".into(),
                message: "`[herdr]` changes apply after Azulejo restarts".into(),
                file: Some(path.display().to_string()),
                line: None,
                column: None,
            });
        }
        if applied {
            *current = Arc::new(fresh);
        }
    }
    log(format!(
        "config: {} {}{}",
        if applied {
            "reloaded"
        } else {
            "kept the previous config; errors in"
        },
        path.display(),
        if diagnostics.is_empty() {
            String::new()
        } else {
            format!(" ({} diagnostics)", diagnostics.len())
        }
    ));
    Some(ConfigReloaded {
        applied,
        path: Some(path.display().to_string()),
        diagnostics,
    })
}

/// A config diagnostic as the protocol carries it.
pub fn diagnostic(d: &Diagnostic) -> ConfigDiagnostic {
    ConfigDiagnostic {
        severity: match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
        .into(),
        message: d.message.clone(),
        file: match &d.source {
            Source::DefaultKeymap => Some("keymap.default.toml".into()),
            Source::User(Some(p)) | Source::Ghostty(p) | Source::Theme(p) => {
                Some(p.display().to_string())
            }
            Source::User(None) => None,
        },
        line: d.location.map(|l| l.line as u32),
        column: d.location.map(|l| l.column as u32),
    }
}
