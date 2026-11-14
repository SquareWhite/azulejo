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

use azulejo_config::{Action, Chord, KeyOutcome, KeyRouter, Target};
use azulejo_daemon::{App, HerdrChoice, Options};
use azulejo_herdr::Session as HerdrSession;
use azulejo_protocol::{Command, Event, TileKind};
use serde_json::{Value, json};

mod terminal;
pub use terminal::{azj_core_attach_exit, azj_core_terminal_command};

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
    /// Prefix state between chords (see `azj_core_key`).
    keys: Mutex<KeyRouter>,
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
            keys: Mutex::new(KeyRouter::default()),
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

/// Runs a JSON command and returns its result as JSON: `{"ok":true,"reply":…}`
/// or `{"ok":false,"error":{"code","message"}}`. Its events still arrive
/// through the callback. Free the result with `azj_string_free`; NULL only on a
/// bad handle.
///
/// # Safety
/// `core` must come from `azj_core_new*`; `command_json` must be a valid C
/// string or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_execute(
    core: *const AzjCore,
    command_json: *const c_char,
) -> *mut c_char {
    let Some(app) = (unsafe { core.as_ref() }).and_then(|c| c.app.as_ref()) else {
        return std::ptr::null_mut();
    };
    let result = match unsafe { arg(command_json) }
        .ok_or_else(|| failure("parse_error", "command isn't a UTF-8 C string"))
        .and_then(|text| Command::from_json(text).map_err(|e| json!({"ok": false, "error": e})))
    {
        Err(failed) => failed,
        Ok(command) => match app.core().execute(command) {
            Ok(outcome) => json!({"ok": true, "reply": outcome.reply}),
            Err(e) => json!({"ok": false, "error": e}),
        },
    };
    out(result)
}

/// Routes one key chord through the user's keymap, keeping prefix state
/// between calls. `request_json` is `{"chord":"alt+h","tile":"terminal"}`
/// (`tile` is the focused tile's kind, or null), or `{"reset":true}` to drop
/// an armed prefix. Returns, as JSON, one of:
///
/// - `{"outcome":"action","action":"focus_tile_left","index":null,
///   "method":"layout.focus","command":{…}}`: run it. `method` is null for
///   shell-local actions; `command` is null when the shell has to build the
///   command from session state (switch to tab N, renames, cycling).
/// - `{"outcome":"prefix_armed"}`, `{"outcome":"prefix_cancelled"}`: consume
///   the chord.
/// - `{"outcome":"unhandled"}`: pass the chord to the focused tile.
/// - `{"outcome":"error","message":…}`: a bad request.
///
/// Free the result with `azj_string_free`; NULL only on a bad handle.
///
/// # Safety
/// `core` must come from `azj_core_new*`; `request_json` must be a valid C
/// string or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_key(
    core: *const AzjCore,
    request_json: *const c_char,
) -> *mut c_char {
    let Some(core) = (unsafe { core.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let Some(app) = &core.app else {
        return std::ptr::null_mut();
    };
    let result = match unsafe { arg(request_json) }.map(serde_json::from_str::<Value>) {
        Some(Ok(request)) => {
            let mut router = core.keys.lock().unwrap_or_else(|e| e.into_inner());
            route_key(&mut router, app, &request)
        }
        _ => json!({"outcome": "error", "message": "request must be a JSON object"}),
    };
    out(result)
}

fn route_key(router: &mut KeyRouter, app: &App, request: &Value) -> Value {
    if request["reset"] == Value::Bool(true) {
        router.reset();
        return json!({"outcome": "reset"});
    }
    let Some(text) = request["chord"].as_str() else {
        return json!({"outcome": "error", "message": "`chord` must be a string"});
    };
    let chord = match Chord::parse(text) {
        Ok(chord) => chord,
        Err(e) => return json!({"outcome": "error", "message": e}),
    };
    let tile = match &request["tile"] {
        Value::Null => None,
        v => match serde_json::from_value::<TileKind>(v.clone()) {
            Ok(kind) => Some(kind),
            Err(e) => return json!({"outcome": "error", "message": format!("`tile`: {e}")}),
        },
    };
    match router.handle(&app.config().keymap, chord, tile) {
        KeyOutcome::Action(action) => action_json(action),
        KeyOutcome::PrefixArmed => json!({"outcome": "prefix_armed"}),
        KeyOutcome::PrefixCancelled => json!({"outcome": "prefix_cancelled"}),
        KeyOutcome::Unhandled => json!({"outcome": "unhandled"}),
    }
}

fn action_json(action: Action) -> Value {
    let method = match action.target() {
        Target::Shell => Value::Null,
        Target::Method(m) => m.into(),
    };
    let command = action
        .command()
        .and_then(|c| serde_json::to_value(c).ok())
        .unwrap_or(Value::Null);
    json!({
        "outcome": "action",
        "action": action.name(),
        "index": action.index(),
        "method": method,
        "command": command,
    })
}

/// Looks an action up by its config name, for menus and the palette:
/// `{"action":"split_right"}` or `{"action":"switch_tab","index":3}`. Returns
/// the same `{"outcome":"action",…}` JSON `azj_core_key` does, or
/// `{"outcome":"error","message":…}` for an unknown name. Free with
/// `azj_string_free`; NULL only on a bad handle.
///
/// # Safety
/// `core` must come from `azj_core_new*`; `request_json` must be a valid C
/// string or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_action(
    core: *const AzjCore,
    request_json: *const c_char,
) -> *mut c_char {
    if (unsafe { core.as_ref() })
        .and_then(|c| c.app.as_ref())
        .is_none()
    {
        return std::ptr::null_mut();
    }
    let request: Value = unsafe { arg(request_json) }
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or(Value::Null);
    let index = request["index"].as_u64().and_then(|i| u8::try_from(i).ok());
    let result = request["action"]
        .as_str()
        .and_then(|name| Action::from_name(name, index))
        .map_or_else(
            || json!({"outcome": "error", "message": "unknown action"}),
            action_json,
        );
    out(result)
}

/// The global keymap (no tile-specific overrides) as JSON, for menus and the
/// palette: `{"platform":"macos","prefix":"cmd+b"|null,"bindings":
/// [{"action":"split_right","index":null,"chord":"ctrl+cmd+v"},…]}`. Chords use
/// the same syntax `azj_core_key` takes. Free with `azj_string_free`.
///
/// # Safety
/// `core` must come from `azj_core_new*` and not be freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_keymap(core: *const AzjCore) -> *mut c_char {
    let Some(app) = (unsafe { core.as_ref() }).and_then(|c| c.app.as_ref()) else {
        return std::ptr::null_mut();
    };
    let config = app.config();
    let keymap = &config.keymap;
    let platform = keymap.platform();
    let bindings: Vec<Value> = keymap
        .bindings(None)
        .map(|(chord, action)| {
            json!({
                "action": action.name(),
                "index": action.index(),
                "chord": chord.display(platform),
            })
        })
        .collect();
    let prefix = keymap
        .prefix_enabled()
        .then(|| keymap.prefix_key())
        .flatten()
        .map(|c| c.display(platform));
    out(json!({"platform": platform.name(), "prefix": prefix, "bindings": bindings}))
}

/// The current `[animation]` settings, as JSON: `{"enabled":true,
/// "duration_ms":200,"curve":"ease_out","tab_switch":"fade",
/// "dim_inactive":false,"reduce_motion":"system"}`. Follows config reloads.
/// Free with `azj_string_free`; NULL on a bad handle.
///
/// # Safety
/// `core` must come from `azj_core_new*` and not be freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_animation(core: *const AzjCore) -> *mut c_char {
    let Some(app) = (unsafe { core.as_ref() }).and_then(|c| c.app.as_ref()) else {
        return std::ptr::null_mut();
    };
    let a = app.config().animation;
    out(json!({
        "enabled": a.enabled,
        "duration_ms": a.duration_ms,
        "curve": a.curve.name(),
        "style": a.style.name(),
        "tab_switch": a.tab_switch.name(),
        "dim_inactive": a.dim_inactive,
        "reduce_motion": a.reduce_motion.name(),
    }))
}

fn failure(code: &str, message: &str) -> Value {
    json!({"ok": false, "error": {"code": code, "message": message}})
}

/// # Safety
/// `s` must be NULL or a valid C string.
unsafe fn arg<'a>(s: *const c_char) -> Option<&'a str> {
    if s.is_null() {
        return None;
    }
    (unsafe { CStr::from_ptr(s) }).to_str().ok()
}

fn out(value: Value) -> *mut c_char {
    CString::new(value.to_string()).map_or(std::ptr::null_mut(), CString::into_raw)
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

/// Reads the config file again now (the `reload_config` action), as if it
/// had been saved. The `config.reloaded` event also goes to the callback.
/// Returns that payload as JSON, `{"applied":false,"diagnostics":[]}` with no
/// `path` when there's no config file, or NULL on a bad handle.
///
/// # Safety
/// `core` must come from `azj_core_new*` and not be freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_reload_config(core: *const AzjCore) -> *mut c_char {
    let Some(app) = (unsafe { core.as_ref() }).and_then(|c| c.app.as_ref()) else {
        return std::ptr::null_mut();
    };
    let value = match app.reload_config() {
        Some(event) => serde_json::to_value(event).unwrap_or(Value::Null),
        None => serde_json::json!({ "applied": false, "diagnostics": [] }),
    };
    out(value)
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

    unsafe fn take(s: *mut c_char) -> Value {
        assert!(!s.is_null());
        let text = unsafe { CStr::from_ptr(s) }.to_str().unwrap().to_owned();
        unsafe { azj_string_free(s) };
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn execute_returns_replies_and_errors() {
        unsafe {
            let core = azj_core_new_with_options(IN_MEMORY.as_ptr());
            let ok = take(azj_core_execute(
                core,
                c"{\"method\":\"session.get\"}".as_ptr(),
            ));
            assert_eq!(ok["ok"], true);
            assert_eq!(ok["reply"]["type"], "session");
            assert_eq!(
                ok["reply"]["session"]["windows"].as_array().unwrap().len(),
                1
            );

            let refused = take(azj_core_execute(
                core,
                c"{\"method\":\"layout.focus\",\"params\":{\"direction\":\"left\"}}".as_ptr(),
            ));
            assert_eq!(refused["ok"], false);
            assert!(refused["error"]["code"].is_string());

            let bad = take(azj_core_execute(core, c"nope".as_ptr()));
            assert_eq!(bad["error"]["code"], "parse_error");
            let null = take(azj_core_execute(core, std::ptr::null()));
            assert_eq!(null["ok"], false);
            azj_core_free(core);
        }
    }

    #[test]
    fn keys_route_through_the_keymap() {
        unsafe {
            let core = azj_core_new_with_options(IN_MEMORY.as_ptr());
            let key = |req: &CStr| take(azj_core_key(core, req.as_ptr()));

            let focus = key(c"{\"chord\":\"alt+h\",\"tile\":\"terminal\"}");
            assert_eq!(focus["outcome"], "action");
            assert_eq!(focus["action"], "focus_tile_left");
            assert_eq!(focus["method"], "layout.focus");
            assert_eq!(focus["command"]["method"], "layout.focus");

            // Indexed actions need session state: no command.
            let tab = key(c"{\"chord\":\"ctrl+3\",\"tile\":null}");
            assert_eq!(tab["action"], "switch_tab");
            assert_eq!(tab["index"], 3);
            assert_eq!(tab["command"], Value::Null);

            // cmd+a toggles the sidebar only in terminal tiles.
            if cfg!(target_os = "macos") {
                let term = key(c"{\"chord\":\"cmd+a\",\"tile\":\"terminal\"}");
                assert_eq!(term["action"], "toggle_sidebar");
                assert_eq!(term["method"], Value::Null);
                let browser = key(c"{\"chord\":\"cmd+a\",\"tile\":\"browser\"}");
                assert_eq!(browser["outcome"], "unhandled");
            }

            assert_eq!(key(c"{\"chord\":\"x\"}")["outcome"], "unhandled");
            assert_eq!(key(c"{\"chord\":\"cmd+bogus\"}")["outcome"], "error");
            assert_eq!(
                key(c"{\"tile\":\"lamp\",\"chord\":\"x\"}")["outcome"],
                "error"
            );
            assert_eq!(key(c"{\"reset\":true}")["outcome"], "reset");

            let action = |req: &CStr| take(azj_core_action(core, req.as_ptr()));
            let split = action(c"{\"action\":\"split_auto\"}");
            assert_eq!(split["command"]["method"], "layout.split_auto");
            assert_eq!(
                action(c"{\"action\":\"switch_tab\",\"index\":2}")["index"],
                2
            );
            assert_eq!(action(c"{\"action\":\"nope\"}")["outcome"], "error");

            let keymap = take(azj_core_keymap(core));
            let bindings = keymap["bindings"].as_array().unwrap();
            assert!(bindings.iter().any(|b| b["action"] == "split_auto"));
            assert_eq!(
                keymap["prefix"],
                Value::Null,
                "prefix mode is off by default"
            );

            let animation = take(azj_core_animation(core));
            assert_eq!(animation["enabled"], true);
            assert_eq!(animation["duration_ms"], 200);
            assert_eq!(animation["curve"], "ease_out");
            assert_eq!(animation["style"], "crop");
            assert_eq!(animation["tab_switch"], "fade");
            assert_eq!(animation["reduce_motion"], "system");
            azj_core_free(core);
        }
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
