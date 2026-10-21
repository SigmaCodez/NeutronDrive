//! Versioned policy plus a separately invoked Linux process runtime.
//! Policy calls stay deterministic; backend calls explicitly own guarded I/O.
mod backend;
mod cache;
mod event_route;
mod events;
mod inotify;
mod jobs;
mod policy;
mod process;
mod storage;
mod watcher;

#[cfg(test)]
use std::ffi::CStr;
use std::ffi::{c_char, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum BackendRequest {
    Process(process::Request),
    Value(backend::Request),
}

const MAX_INPUT: usize = 64 * 1024 * 1024;

/// The callback runs synchronously on the calling thread. Return nonzero to
/// cancel. Event bytes are borrowed only for the duration of the callback.
pub type BackendCallback = unsafe extern "C" fn(*const u8, usize, *mut std::ffi::c_void) -> i32;

/// # Safety
/// Input must be readable for `length` bytes. Callback/context must remain valid
/// until return and must not unwind. Free the reply with neutrondrive_core_free.
#[no_mangle]
pub unsafe extern "C" fn neutrondrive_backend_call(
    input: *const u8,
    length: usize,
    callback: Option<BackendCallback>,
    context: *mut std::ffi::c_void,
) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if input.is_null() || length == 0 || length > MAX_INPUT || callback.is_none() {
            return serde_json::json!({"ok":false,"error":"Invalid backend request"});
        }
        let bytes = unsafe { std::slice::from_raw_parts(input, length) };
        let request: BackendRequest = match serde_json::from_slice(bytes) {
            Ok(request) => request,
            Err(_) => return serde_json::json!({"ok":false,"error":"Invalid backend request"}),
        };
        let mut notify = |value: serde_json::Value| {
            let event = value.to_string();
            // SAFETY: caller owns callback/context; event lives through call.
            unsafe { callback.unwrap()(event.as_ptr(), event.len(), context) }
        };
        let result = match request {
            BackendRequest::Process(request) => process::run(request, &mut notify),
            BackendRequest::Value(request) => backend::dispatch(request, &mut notify),
        };
        match result {
            Ok(value) => serde_json::json!({"ok":true,"result":value}),
            Err(error) => {
                serde_json::json!({"ok":false,"error":error.message,"cancelled":error.cancelled})
            }
        }
    }));
    let reply = result.unwrap_or_else(
        |_| serde_json::json!({"ok":false,"error":"Backend runtime panicked; operation stopped"}),
    );
    CString::new(reply.to_string())
        .expect("JSON has no raw NUL")
        .into_raw()
}

#[no_mangle]
pub extern "C" fn neutrondrive_core_abi() -> u32 {
    1
}

/// # Safety
/// `input` must point to `length` readable bytes for this call. The returned
/// owned string must be released exactly once with neutrondrive_core_free.
#[no_mangle]
pub unsafe extern "C" fn neutrondrive_core_call(input: *const u8, length: usize) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if input.is_null() || length == 0 || length > MAX_INPUT {
            return serde_json::json!({"ok":false,"error":"Invalid Rust core request length"});
        }
        // SAFETY: the ABI caller promises a live immutable buffer of length.
        let bytes = unsafe { std::slice::from_raw_parts(input, length) };
        match policy::dispatch(bytes) {
            Ok(value) => serde_json::json!({"ok":true,"result":value}),
            Err(error) => serde_json::json!({"ok":false,"error":error}),
        }
    }));
    let reply = result.unwrap_or_else(
        |_| serde_json::json!({"ok":false,"error":"Rust core panicked; operation stopped"}),
    );
    // JSON serialization escapes all embedded NULs.
    CString::new(reply.to_string())
        .expect("JSON has no raw NUL")
        .into_raw()
}

/// # Safety
/// `value` must be an unreleased pointer returned by neutrondrive_core_call
/// or neutrondrive_backend_call.
#[no_mangle]
pub unsafe extern "C" fn neutrondrive_core_free(value: *mut c_char) {
    if !value.is_null() {
        // SAFETY: ownership is transferred back by the ABI caller.
        drop(unsafe { CString::from_raw(value) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    unsafe extern "C" fn ignore_event(_: *const u8, _: usize, _: *mut std::ffi::c_void) -> i32 {
        0
    }

    #[test]
    fn backend_abi_rejects_invalid_buffers_and_missing_callbacks() {
        for length in [0, 1, MAX_INPUT + 1] {
            let reply = unsafe {
                neutrondrive_backend_call(
                    std::ptr::null(),
                    length,
                    Some(ignore_event),
                    std::ptr::null_mut(),
                )
            };
            let value: serde_json::Value =
                serde_json::from_slice(unsafe { CStr::from_ptr(reply) }.to_bytes()).unwrap();
            assert_eq!(value["ok"], false);
            unsafe { neutrondrive_core_free(reply) };
        }
        let request = br#"{"version":1,"op":"validate_remote","path":"/my-files"}"#;
        let reply = unsafe {
            neutrondrive_backend_call(request.as_ptr(), request.len(), None, std::ptr::null_mut())
        };
        let value: serde_json::Value =
            serde_json::from_slice(unsafe { CStr::from_ptr(reply) }.to_bytes()).unwrap();
        assert_eq!(value["ok"], false);
        unsafe { neutrondrive_core_free(reply) };
    }

    #[test]
    fn backend_abi_value_roundtrip_and_invalid_schema_are_owned_replies() {
        for request in [
            br#"{"version":1,"op":"validate_remote","path":"/my-files"}"#.as_slice(),
            b"{}",
            b"{",
            br#"{"version":1,"op":"validate_remote","path":"/my-files","extra":1}"#,
        ] {
            let reply = unsafe {
                neutrondrive_backend_call(
                    request.as_ptr(),
                    request.len(),
                    Some(ignore_event),
                    std::ptr::null_mut(),
                )
            };
            let value: serde_json::Value =
                serde_json::from_slice(unsafe { CStr::from_ptr(reply) }.to_bytes()).unwrap();
            assert_eq!(
                value["ok"],
                request == br#"{"version":1,"op":"validate_remote","path":"/my-files"}"#
            );
            unsafe { neutrondrive_core_free(reply) };
        }
    }
    #[test]
    fn abi_rejects_null_and_oversize_without_dereferencing() {
        for length in [0, 1, MAX_INPUT + 1] {
            let reply = unsafe { neutrondrive_core_call(std::ptr::null(), length) };
            let value: serde_json::Value =
                serde_json::from_slice(unsafe { CStr::from_ptr(reply) }.to_bytes()).unwrap();
            assert_eq!(value["ok"], false);
            unsafe { neutrondrive_core_free(reply) };
        }
    }
    #[test]
    fn abi_roundtrip_and_invalid_json_fail_closed() {
        for request in [br#"{"version":1,"op":"info"}"#.as_slice(), b"{", b"{}"] {
            let reply = unsafe { neutrondrive_core_call(request.as_ptr(), request.len()) };
            let value: serde_json::Value =
                serde_json::from_slice(unsafe { CStr::from_ptr(reply) }.to_bytes()).unwrap();
            assert_eq!(value["ok"], request.starts_with(b"{\"version\""));
            unsafe { neutrondrive_core_free(reply) };
        }
    }
}
