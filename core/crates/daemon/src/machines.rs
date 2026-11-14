//! herdr's saved machines (ADR 0011). herdr owns the list; this keeps the
//! core's links in step with it: read `herdr machine list --json` at start,
//! on config reloads and every minute, then connect added machines,
//! disconnect removed ones and show disabled ones as off. The config only
//! hides machines or keeps them from connecting.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread;
use std::time::Duration;

use azulejo_config::HerdrConfig;
use azulejo_engine::{Core, LogFn};

use crate::watch::SharedConfig;
use azulejo_herdr::{Herdr, MachineProfile, RemoteMachine, Ssh, parse_machine_list};
use azulejo_protocol::{MachineInfo, MachineState};

/// How often herdr's list is read again. It's a local file read.
const POLL: Duration = Duration::from_secs(60);

/// Where machine profiles come from.
#[derive(Debug, Clone)]
pub enum Source {
    /// `herdr machine list --json` (reads herdr's catalogue, never connects).
    Herdr(Herdr),
    /// A file in that format: tests and dev profiles.
    File(PathBuf),
}

impl Source {
    fn read(&self) -> Result<Vec<MachineProfile>, String> {
        match self {
            Source::File(path) => std::fs::read_to_string(path)
                .map_err(|e| format!("{}: {e}", path.display()))
                .and_then(|text| parse_machine_list(&text).map_err(|e| e.to_string())),
            Source::Herdr(herdr) => tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
                .and_then(|rt| rt.block_on(herdr.machines()).map_err(|e| e.to_string())),
        }
    }
}

/// What a machine is to the core right now.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Shown {
    Connected(MachineProfile),
    Off(MachineProfile),
}

/// The remote machines terminal tiles can attach to, by herdr ID.
pub type Registry = Arc<Mutex<HashMap<String, RemoteMachine>>>;

/// Keeps the core's machines in step with herdr's list until dropped.
pub struct MachineSync {
    wake: mpsc::Sender<Wake>,
    thread: Option<thread::JoinHandle<()>>,
}

enum Wake {
    /// Read the list again now (the config changed).
    Reread,
    Stop,
}

impl MachineSync {
    /// Starts following herdr's list. A config reload (`config.reloaded`)
    /// re-reads it at once with the new `[herdr.machine.*]` settings.
    pub fn start(
        core: &Arc<Core>,
        source: Source,
        ssh: Ssh,
        config: SharedConfig,
        registry: Registry,
        log: LogFn,
    ) -> MachineSync {
        let (wake, rx) = mpsc::channel();
        {
            let wake = wake.clone();
            let events = core.subscribe();
            let _ = thread::Builder::new()
                .name("azulejo-machines-reload".into())
                .spawn(move || {
                    while let Ok(event) = events.recv() {
                        if matches!(event, azulejo_protocol::Event::ConfigReloaded(_))
                            && wake.send(Wake::Reread).is_err()
                        {
                            return;
                        }
                    }
                });
        }
        let core = Arc::downgrade(core);
        let thread = thread::Builder::new()
            .name("azulejo-machines".into())
            .spawn(move || run(core, source, ssh, config, registry, log, rx))
            .expect("spawn machines thread");
        MachineSync {
            wake,
            thread: Some(thread),
        }
    }
}

impl Drop for MachineSync {
    fn drop(&mut self) {
        let _ = self.wake.send(Wake::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(
    core: Weak<Core>,
    source: Source,
    ssh: Ssh,
    config: SharedConfig,
    registry: Registry,
    log: LogFn,
    rx: mpsc::Receiver<Wake>,
) {
    if let Err(e) = ssh.prepare() {
        log(format!(
            "machines: can't create {}: {e}",
            ssh.control_dir.display()
        ));
    }
    let mut shown: HashMap<String, Shown> = HashMap::new();
    let mut last_error: Option<String> = None;
    loop {
        let Some(core) = core.upgrade() else {
            return;
        };
        match source.read() {
            Ok(profiles) => {
                last_error = None;
                let herdr = match config.read() {
                    Ok(c) => c.herdr.clone(),
                    Err(poisoned) => poisoned.into_inner().herdr.clone(),
                };
                let want = plan(&profiles, &herdr);
                apply(&core, &ssh, &registry, &mut shown, want);
            }
            Err(e) => {
                // Keep what's connected; say so once.
                if last_error.as_deref() != Some(e.as_str()) {
                    log(format!("machines: reading herdr's machine list: {e}"));
                    last_error = Some(e);
                }
            }
        }
        drop(core);
        match rx.recv_timeout(POLL) {
            Ok(Wake::Reread) => {}
            Ok(Wake::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

/// What the core should show for herdr's list under `config`, by ID.
fn plan(profiles: &[MachineProfile], config: &HerdrConfig) -> HashMap<String, Shown> {
    profiles
        .iter()
        .filter_map(|p| {
            let prefs = config.prefs(&p.id, &p.label);
            if prefs.hidden {
                return None;
            }
            let shown = if p.enabled && !prefs.no_connect {
                Shown::Connected(p.clone())
            } else {
                Shown::Off(p.clone())
            };
            Some((p.id.clone(), shown))
        })
        .collect()
}

fn apply(
    core: &Core,
    ssh: &Ssh,
    registry: &Registry,
    shown: &mut HashMap<String, Shown>,
    want: HashMap<String, Shown>,
) {
    for id in shown
        .keys()
        .filter(|id| !want.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>()
    {
        shown.remove(&id);
        if let Ok(mut r) = registry.lock() {
            r.remove(&id);
        }
        core.remove_machine(id);
    }
    for (id, next) in want {
        if shown.get(&id) == Some(&next) {
            continue;
        }
        match &next {
            Shown::Connected(profile) => {
                let machine = RemoteMachine::new(profile.clone(), ssh.clone());
                let client = machine.client();
                if let Ok(mut r) = registry.lock() {
                    r.insert(id.clone(), machine);
                }
                core.add_machine(id.clone(), profile.label.clone(), client);
            }
            Shown::Off(profile) => {
                if let Ok(mut r) = registry.lock() {
                    r.remove(&id);
                }
                core.show_machine(MachineInfo {
                    id: id.clone(),
                    label: profile.label.clone(),
                    state: MachineState::Off,
                    message: Some(if profile.enabled {
                        "not connected (`connect = false` in Azulejo's config)".into()
                    } else {
                        "disabled in herdr (`herdr machine enable`)".into()
                    }),
                });
            }
        }
        shown.insert(id, next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use azulejo_config::MachinePrefs;

    fn profile(id: &str, label: &str, enabled: bool) -> MachineProfile {
        MachineProfile {
            id: id.into(),
            label: label.into(),
            target: format!("{label}-host"),
            session: "default".into(),
            enabled,
        }
    }

    #[test]
    fn plan_follows_herdr_and_the_presentation_prefs() {
        let profiles = [
            profile("a", "build-server", true),
            profile("b", "dev-server", true),
            profile("c", "old", false),
            profile("d", "secret", true),
        ];
        let mut config = HerdrConfig::default();
        config.machine.insert(
            "dev-server".into(),
            MachinePrefs {
                no_connect: true,
                ..Default::default()
            },
        );
        config.machine.insert(
            "d".into(),
            MachinePrefs {
                hidden: true,
                ..Default::default()
            },
        );
        let plan = plan(&profiles, &config);
        assert!(matches!(plan["a"], Shown::Connected(_)));
        assert!(
            matches!(plan["b"], Shown::Off(_)),
            "connect = false by label"
        );
        assert!(matches!(plan["c"], Shown::Off(_)), "disabled in herdr");
        assert!(!plan.contains_key("d"), "hidden by ID");
    }
}
