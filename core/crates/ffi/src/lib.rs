//! C ABI for the macOS shell. Commands and events cross as JSON strings.
//!
//! `azj_core_new` boots the whole core ([`azulejo_daemon::App`]): the user's
//! config, the saved session, herdr and the API server. Commands from the shell
//! and from `azj` land on the same engine, and the shell sees every resulting
//! event, whoever caused it.
//!
//! Threading rules:
//! - A core handle is not thread-safe. Call every `azj_core_*` function on one
//!   thread (the shell's main thread).
//! - The event callback runs on a background thread, never inside
//!   `azj_core_send`. Hop to the main thread (`DispatchQueue.main` in Swift)
//!   before touching UI.
//! - The event JSON pointer is only valid during the callback; copy it.
//! - After `azj_core_free` returns, the callback is never called again.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use azulejo_daemon::{App, HerdrChoice, Options};
use azulejo_herdr::Session as HerdrSession;
use azulejo_protocol::{Command, Event};
use serde_json::Value;

/// Called once per event with a NUL-terminated JSON string, valid only for the call.
/// Runs on a background thread; see the crate docs for the threading rules.
pub type AzjEventCallback = extern "C" fn(context: *mut c_void, event_json: *const c_char);

#[derive(Clone, Copy)]
struct Listener {
    callback: AzjEventCallback,
    context: *mut c_void,
}

// SAFETY: the context pointer is only handed back to the shell's callback,
// which the threading rules require to be callable from any thread.
unsafe impl Send for Listener {}

/// Opaque handle to a running core.
pub struct AzjCore {
    app: Option<App>,
    listener: Arc<Mutex<Option<Listener>>>,
    stop: Arc<AtomicBool>,
    forwarder: Option<JoinHandle<()>>,
}

impl AzjCore {
    fn start(opts: Options) -> Option<Box<AzjCore>> {
        let app = match App::start(opts) {
            Ok(app) => app,
            Err(e) => {
                eprintln!("azulejo: {e}");
                return None;
            }
        };
        let listener: Arc<Mutex<Option<Listener>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let events = app.core().subscribe();
        let forwarder = {
            let listener = listener.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("azulejo-ffi-events".into())
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        match events.recv_timeout(Duration::from_millis(50)) {
                            Ok(event) => deliver(&listener, &event),
                            Err(RecvTimeoutError::Timeout) => {}
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    }
                })
                .ok()?
        };
        Some(Box::new(AzjCore {
            app: Some(app),
            listener,
            stop,
            forwarder: Some(forwarder),
        }))
    }
}

impl Drop for AzjCore {
    fn drop(&mut self) {
        // Stop delivering first, so no callback runs after free returns.
        self.stop.store(true, Ordering::Release);
        if let Some(forwarder) = self.forwarder.take() {
            let _ = forwarder.join();
        }
        if let Some(app) = self.app.take() {
            app.shutdown();
        }
    }
}

fn deliver(listener: &Mutex<Option<Listener>>, event: &Event) {
    // Holding the lock while calling back keeps set_event_callback from
    // swapping the context out from under a running callback.
    let guard = listener.lock().unwrap_or_else(|e| e.into_inner());
    if let (Some(l), Ok(json)) = (*guard, serde_json::to_string(event)) {
        let json = CString::new(json).expect("JSON never contains NUL");
        (l.callback)(l.context, json.as_ptr());
    }
}

/// Boots the core with the real config, saved state, herdr and API socket. If
/// the socket is taken (another instance is running) it starts without the API
/// and logs why. Never returns NULL.
#[unsafe(no_mangle)]
pub extern "C" fn azj_core_new() -> *mut AzjCore {
    let core = AzjCore::start(Options::default())
        .or_else(|| {
            // Defaults never require the API, so this only happens if a thread
            // couldn't be spawned. Fall back to an in-memory core.
            AzjCore::start(Options {
                state_path: None,
                socket_path: None,
                herdr: HerdrChoice::Off,
                ..Options::default()
            })
        })
        .expect("an in-memory core always starts");
    Box::into_raw(core)
}

/// Boots the core with `options_json` overriding the defaults; NULL on bad
/// options or when the API is required and its socket is taken. Keys (all
/// optional; `null` turns the thing off):
///
/// - `state_path`, `socket_path`, `config_path`: paths
/// - `herdr`: `false` for no herdr
/// - `herdr_session`: a session name, or `"default"` for herdr's own
/// - `api_required`: fail instead of running without the API
///
/// # Safety
/// `options_json` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_new_with_options(options_json: *const c_char) -> *mut AzjCore {
    if options_json.is_null() {
        return std::ptr::null_mut();
    }
    let Ok(text) = (unsafe { CStr::from_ptr(options_json) }).to_str() else {
        return std::ptr::null_mut();
    };
    match parse_options(text) {
        Ok(opts) => AzjCore::start(opts).map_or(std::ptr::null_mut(), Box::into_raw),
        Err(e) => {
            eprintln!("azulejo: bad core options: {e}");
            std::ptr::null_mut()
        }
    }
}

fn parse_options(text: &str) -> Result<Options, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let Value::Object(map) = value else {
        return Err("options must be a JSON object".into());
    };
    let mut opts = Options::default();
    let path = |v: &Value, key: &str| -> Result<Option<PathBuf>, String> {
        match v {
            Value::Null => Ok(None),
            Value::String(s) => Ok(Some(PathBuf::from(s))),
            _ => Err(format!("`{key}` must be a path or null")),
        }
    };
    for (key, v) in &map {
        match key.as_str() {
            "state_path" => opts.state_path = path(v, key)?,
            "socket_path" => opts.socket_path = path(v, key)?,
            "config_path" => opts.config_path = path(v, key)?,
            "api_required" => {
                opts.api_required = v.as_bool().ok_or("`api_required` must be a bool")?
            }
            "herdr" => match v {
                Value::Bool(false) | Value::Null => opts.herdr = HerdrChoice::Off,
                Value::Bool(true) => {}
                _ => return Err("`herdr` must be a bool".into()),
            },
            "herdr_session" => match v {
                Value::Null => opts.herdr = HerdrChoice::Off,
                Value::String(s) if s == "default" => {
                    opts.herdr = HerdrChoice::Session(HerdrSession::default_session())
                }
                Value::String(s) => opts.herdr = HerdrChoice::Session(HerdrSession::named(s)),
                _ => return Err("`herdr_session` must be a string or null".into()),
            },
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    Ok(opts)
}

/// # Safety
/// `core` must come from `azj_core_new*` and not be freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_set_event_callback(
    core: *mut AzjCore,
    callback: AzjEventCallback,
    context: *mut c_void,
) {
    let Some(core) = (unsafe { core.as_mut() }) else {
        return;
    };
    *core.listener.lock().unwrap_or_else(|e| e.into_inner()) = Some(Listener { callback, context });
}

/// Sends a JSON command; its events arrive through the callback. Returns 0 on
/// success, -1 on a bad handle or a command that doesn't parse, and -2 when the
/// core refuses the command (nothing changes and no events fire). Replies don't
/// cross the C ABI yet (PLAN step 1.6).
///
/// # Safety
/// `core` must come from `azj_core_new*`; `command_json` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_send(core: *mut AzjCore, command_json: *const c_char) -> i32 {
    let Some(core) = (unsafe { core.as_ref() }) else {
        return -1;
    };
    let Some(app) = &core.app else {
        return -1;
    };
    if command_json.is_null() {
        return -1;
    }
    let Ok(text) = (unsafe { CStr::from_ptr(command_json) }).to_str() else {
        return -1;
    };
    let Ok(command) = Command::from_json(text) else {
        return -1;
    };
    match app.core().execute(command) {
        Ok(_) => 0,
        Err(_) => -2,
    }
}

/// Where things live, as JSON: the API socket, state and config paths, and
/// the herdr session, socket and binary (the same fields `core.version`
/// reports). Free the result with `azj_string_free`. NULL on a bad handle.
///
/// # Safety
/// `core` must come from `azj_core_new*` and not be freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_info(core: *const AzjCore) -> *mut c_char {
    let Some(app) = (unsafe { core.as_ref() }).and_then(|c| c.app.as_ref()) else {
        return std::ptr::null_mut();
    };
    let json = Value::Object(app.info()).to_string();
    CString::new(json).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Frees a string returned by this library.
///
/// # Safety
/// `s` must come from this library and not be freed twice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// Stops the API server, saves state and stops the core. herdr panes keep
/// running. No callback runs after this returns.
///
/// # Safety
/// `core` must come from `azj_core_new*` and not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_free(core: *mut AzjCore) {
    if !core.is_null() {
        drop(unsafe { Box::from_raw(core) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Instant;

    static RECEIVED: Mutex<Option<mpsc::Sender<String>>> = Mutex::new(None);

    extern "C" fn record(_: *mut c_void, json: *const c_char) {
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        if let Some(tx) = RECEIVED.lock().unwrap().as_ref() {
            let _ = tx.send(text);
        }
    }

    const IN_MEMORY: &CStr =
        c"{\"state_path\":null,\"socket_path\":null,\"config_path\":null,\"herdr\":false}";

    #[test]
    fn commands_and_events_cross_the_c_abi() {
        let (tx, rx) = mpsc::channel();
        *RECEIVED.lock().unwrap() = Some(tx);
        unsafe {
            let core = azj_core_new_with_options(IN_MEMORY.as_ptr());
            assert!(!core.is_null());
            azj_core_set_event_callback(core, record, std::ptr::null_mut());
            assert_eq!(
                azj_core_send(core, c"{\"method\":\"core.ping\"}".as_ptr()),
                0
            );
            assert_eq!(azj_core_send(core, c"not json".as_ptr()), -1);
            assert_eq!(
                azj_core_send(core, c"{\"method\":\"tile.explode\"}".as_ptr()),
                -1
            );
            // A lone tile has no neighbour to the left: refused.
            assert_eq!(
                azj_core_send(
                    core,
                    c"{\"method\":\"layout.focus\",\"params\":{\"direction\":\"left\"}}".as_ptr()
                ),
                -2
            );
            assert_eq!(
                azj_core_send(
                    core,
                    c"{\"method\":\"layout.split\",\"params\":{\"direction\":\"right\"}}".as_ptr()
                ),
                0
            );

            // The split's events arrive on the forwarder thread.
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut events: Vec<Value> = Vec::new();
            while events.len() < 3 && Instant::now() < deadline {
                if let Ok(json) = rx.recv_timeout(Duration::from_millis(100)) {
                    events.push(serde_json::from_str(&json).unwrap());
                }
            }
            let names: Vec<&str> = events
                .iter()
                .map(|e| e["event"].as_str().unwrap())
                .collect();
            assert_eq!(names, ["tile.created", "layout.changed", "focus.changed"]);
            let tree = &events[1]["data"]["tree"];
            assert_eq!(tree["type"], "split");
            assert_eq!(tree["axis"], "horizontal");
            assert_eq!(tree["b"]["id"], events[1]["data"]["focused"]);

            let info = azj_core_info(core);
            let text = CStr::from_ptr(info).to_str().unwrap().to_owned();
            azj_string_free(info);
            let info: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(info["herdr_session"], Value::Null);
            assert_eq!(info["socket"], Value::Null);

            azj_core_free(core);
        }
        // Nothing arrives after free.
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        *RECEIVED.lock().unwrap() = None;
    }

    #[test]
    fn bad_options_give_null() {
        unsafe {
            assert!(azj_core_new_with_options(c"[]".as_ptr()).is_null());
            assert!(azj_core_new_with_options(c"{\"bogus\":1}".as_ptr()).is_null());
            assert!(azj_core_new_with_options(c"{\"state_path\":3}".as_ptr()).is_null());
            assert!(azj_core_new_with_options(std::ptr::null()).is_null());
        }
    }
}
