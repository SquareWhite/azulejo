//! C ABI for the macOS shell. Commands and events cross as JSON strings.
//!
//! Threading rules:
//! - A core handle is not thread-safe. Call every `azj_core_*` function on one
//!   thread (the shell's main thread).
//! - Today the callback runs synchronously inside `azj_core_send`, on the caller's
//!   thread. Once the herdr adapter and API server exist, events will also arrive
//!   from background threads, so shells must not assume the main thread: hop to it
//!   (`DispatchQueue.main` in Swift) before touching UI.
//! - The event JSON pointer is only valid during the callback; copy it.

use std::ffi::{CStr, CString, c_char, c_void};

use azulejo_engine::Engine;
use azulejo_protocol::Command;

/// Called once per event with a NUL-terminated JSON string, valid only for the call.
/// May run on any thread; see the crate docs for the threading rules.
pub type AzjEventCallback = extern "C" fn(context: *mut c_void, event_json: *const c_char);

/// Opaque handle to a running core.
pub struct AzjCore {
    engine: Engine,
    callback: Option<AzjEventCallback>,
    context: *mut c_void,
}

#[unsafe(no_mangle)]
pub extern "C" fn azj_core_new() -> *mut AzjCore {
    Box::into_raw(Box::new(AzjCore {
        engine: Engine::new(),
        callback: None,
        context: std::ptr::null_mut(),
    }))
}

/// # Safety
/// `core` must come from `azj_core_new` and not be freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_set_event_callback(
    core: *mut AzjCore,
    callback: AzjEventCallback,
    context: *mut c_void,
) {
    let Some(core) = (unsafe { core.as_mut() }) else {
        return;
    };
    core.callback = Some(callback);
    core.context = context;
}

/// Sends a JSON command. Returns 0 on success, -1 on a bad handle or invalid JSON.
///
/// # Safety
/// `core` must come from `azj_core_new`; `command_json` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_send(core: *mut AzjCore, command_json: *const c_char) -> i32 {
    let Some(core) = (unsafe { core.as_mut() }) else {
        return -1;
    };
    if command_json.is_null() {
        return -1;
    }
    let Ok(text) = (unsafe { CStr::from_ptr(command_json) }).to_str() else {
        return -1;
    };
    let Ok(command) = serde_json::from_str::<Command>(text) else {
        return -1;
    };
    for event in core.engine.handle(command) {
        if let (Some(callback), Ok(json)) = (core.callback, serde_json::to_string(&event)) {
            let json = CString::new(json).expect("JSON never contains NUL");
            callback(core.context, json.as_ptr());
        }
    }
    0
}

/// # Safety
/// `core` must come from `azj_core_new` and not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azj_core_free(core: *mut AzjCore) {
    if !core.is_null() {
        drop(unsafe { Box::from_raw(core) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static RECEIVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    extern "C" fn record(_: *mut c_void, json: *const c_char) {
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        RECEIVED.lock().unwrap().push(text);
    }

    #[test]
    fn ping_round_trips_through_the_c_abi() {
        unsafe {
            let core = azj_core_new();
            azj_core_set_event_callback(core, record, std::ptr::null_mut());
            assert_eq!(
                azj_core_send(core, c"{\"method\":\"core.ping\"}".as_ptr()),
                0
            );
            assert_eq!(azj_core_send(core, c"not json".as_ptr()), -1);
            assert_eq!(
                azj_core_send(
                    core,
                    c"{\"method\":\"layout.split\",\"params\":{\"direction\":\"right\"}}".as_ptr()
                ),
                0
            );
            azj_core_free(core);
        }
        let received = RECEIVED.lock().unwrap();
        assert_eq!(
            received[0],
            r#"{"event":"core.pong","data":{"api_version":1}}"#
        );
        assert_eq!(
            received[1],
            r#"{"event":"layout.changed","data":{"tree":{"type":"split","axis":"horizontal","ratio":0.5,"a":{"type":"leaf","id":"t1"},"b":{"type":"leaf","id":"t2"}},"focused":"t2"}}"#
        );
    }
}
