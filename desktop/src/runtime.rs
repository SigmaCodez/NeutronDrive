//! Shared, typed failure handling for the native UI's Rust core calls.
use serde_json::Value;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub cancelled: bool,
}
pub fn call(mut request: Value, callback: &mut impl FnMut(Value) -> i32) -> Result<Value, Failure> {
    request["version"] = 1.into();
    let result = neutrondrive_core::runtime_request(request.to_string().as_bytes(), callback);
    if result["ok"] == true {
        Ok(result["result"].clone())
    } else {
        Err(Failure {
            message: result["error"]
                .as_str()
                .unwrap_or("Native operation failed")
                .into(),
            cancelled: result["cancelled"].as_bool().unwrap_or(false),
        })
    }
}
pub fn value(request: Value) -> Result<Value, Failure> {
    call(request, &mut |_| 0)
}

/// Disk-backed browsing runs exclusively on workers, never the Qt thread.
pub fn browse(
    executable: String,
    operation: Value,
    fresh: bool,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value, Failure> {
    neutrondrive_core::browse_cache::run(executable, operation, fresh, callback).map_err(|e| {
        Failure {
            message: e.message,
            cancelled: e.cancelled,
        }
    })
}
