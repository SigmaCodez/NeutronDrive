//! Versioned policy plus a separately invoked Linux process runtime.
//! Policy calls stay deterministic; backend calls explicitly own guarded I/O.
pub mod appimage;
mod autostart;
mod backend;
mod cache;
mod computer;
mod computer_setup;
mod control;
mod control_server;
mod daemon;
mod daemon_control;
mod daemon_job;
mod desktop_bookmarks;
mod desktop_bus;
mod desktop_runtime;
mod desktop_service;
mod drive;
mod event_ancestry;
mod event_route;
mod event_runtime;
mod event_source;
mod events;
mod filesystem;
mod fuse;
mod host;
mod inotify;
mod job_roots;
mod jobs;
mod localization;
mod metadata_cache;
mod mount_manager;
mod ondemand_fs;
mod ondemand_handles;
mod ondemand_io;
mod ondemand_namespace;
mod ondemand_replace;
mod ondemand_snapshot;
mod ondemand_tree;
mod ondemand_write;
mod policy;
mod portals;
mod process;
mod service_controller;
mod storage;
mod sync_execute;
mod sync_plan;
mod sync_proof;
mod sync_remote;
mod sync_state;
mod trash_journal;
mod trash_location;
mod trash_move;
mod trash_restore;
mod update_policy;
mod update_runtime;
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

/// Safe Rust entry point shared by native executables and the temporary C ABI.
/// Callback failures cancel and reap a running SDK process before returning.
pub fn runtime_request(
    bytes: &[u8],
    callback: &mut impl FnMut(serde_json::Value) -> i32,
) -> serde_json::Value {
    if bytes.is_empty() || bytes.len() > MAX_INPUT {
        return serde_json::json!({"ok":false,"error":"Invalid backend request"});
    }
    let request: BackendRequest = match serde_json::from_slice(bytes) {
        Ok(request) => request,
        Err(_) => return serde_json::json!({"ok":false,"error":"Invalid backend request"}),
    };
    let result = match request {
        BackendRequest::Process(request) => process::run(request, callback),
        BackendRequest::Value(request) => backend::dispatch(request, callback),
    };
    match result {
        Ok(value) => serde_json::json!({"ok":true,"result":value}),
        Err(error) => {
            serde_json::json!({"ok":false,"error":error.message,"cancelled":error.cancelled})
        }
    }
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
        let mut notify = |value: serde_json::Value| {
            let event = value.to_string();
            // SAFETY: caller owns callback/context; event lives through call.
            unsafe { callback.unwrap()(event.as_ptr(), event.len(), context) }
        };
        runtime_request(bytes, &mut notify)
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

/// Safe policy entry point for native callers; retains the C ABI's reply envelope.
pub fn policy_request(bytes: &[u8]) -> serde_json::Value {
    if bytes.is_empty() || bytes.len() > MAX_INPUT {
        return serde_json::json!({"ok":false,"error":"Invalid Rust core request length"});
    }
    match policy::dispatch(bytes) {
        Ok(value) => serde_json::json!({"ok":true,"result":value}),
        Err(error) => serde_json::json!({"ok":false,"error":error}),
    }
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
        policy_request(bytes)
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
    fn safe_policy_reply_matches_ffi() {
        for bytes in [
            br#"{"version":1,"op":"info"}"#.as_slice(),
            br#"{"version":99,"op":"info"}"#.as_slice(),
            b"invalid".as_slice(),
            b"".as_slice(),
        ] {
            let pointer = unsafe { neutrondrive_core_call(bytes.as_ptr(), bytes.len()) };
            let ffi: serde_json::Value =
                serde_json::from_slice(unsafe { CStr::from_ptr(pointer) }.to_bytes()).unwrap();
            unsafe { neutrondrive_core_free(pointer) };
            assert_eq!(policy_request(bytes), ffi);
        }
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
