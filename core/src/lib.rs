//! Versioned, deterministic reconciliation and daemon policy.
//! No filesystem, network, credentials or Python calls occur in this library.
//! The host executes guarded I/O and submits fresh observations for decisions.
mod cache;
mod events;
mod inotify;
mod policy;

#[cfg(test)]
use std::ffi::CStr;
use std::ffi::{c_char, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
const MAX_INPUT: usize = 64 * 1024 * 1024;

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
/// `value` must be an unreleased pointer returned by neutrondrive_core_call.
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
