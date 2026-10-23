//! Local journal adaptation for the native UI; recovery remains in the Rust core.
use crate::runtime::{self, Failure};
use serde_json::{json, Value};
use std::path::Path;
fn failure(message: &str) -> Failure {
    Failure {
        message: message.into(),
        cancelled: false,
    }
}
pub fn run(operation: &Value, callback: &mut impl FnMut(Value) -> i32) -> Result<Value, Failure> {
    let root = runtime::call(json!({"op":"data_dir"}), callback)?;
    let state = Path::new(
        root.as_str()
            .ok_or_else(|| failure("Invalid state directory"))?,
    )
    .join("local-trash");
    run_at(&state, operation, callback)
}
fn run_at(
    state: &Path,
    operation: &Value,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value, Failure> {
    match operation["action"].as_str().unwrap_or("") {
        "list" => {
            if !state.exists() {
                return Ok(json!([]));
            }
            let values = runtime::call(json!({"op":"trash_list","state":state}), callback)?;
            let records = values
                .as_array()
                .ok_or_else(|| failure("Invalid trash list"))?;
            Ok(json!(records
                .iter()
                .map(|record| json!({"state":state,"record":record}))
                .collect::<Vec<_>>()))
        }
        "restore" => {
            if operation["state"] != json!(state) {
                return Err(failure("Unrecognized trash journal"));
            }
            let result = runtime::call(
                json!({"op":"trash_restore","state":state,"identifier":operation["id"],"destination":null}),
                callback,
            )?;
            restored(result)
        }
        _ => Err(failure("Unknown trash action")),
    }
}
fn restored(result: Value) -> Result<Value, Failure> {
    if result["ok"] != true {
        return Err(Failure {
            message: result["error"]
                .as_str()
                .unwrap_or("Restore did not complete")
                .into(),
            cancelled: result["cancelled"] == true,
        });
    }
    Ok(result["record"].clone())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_or_cancelled_restore_is_never_reported_as_success() {
        for cancelled in [false, true] {
            let result=restored(json!({"ok":false,"error":"original data preserved","cancelled":cancelled,"record":{"status":"recovery"}})).unwrap_err();
            assert_eq!(result.message, "original data preserved");
            assert_eq!(result.cancelled, cancelled);
        }
        assert!(restored(json!({"record":{}})).is_err());
        assert_eq!(
            restored(json!({"ok":true,"record":{"status":"restored"}})).unwrap(),
            json!({"status":"restored"})
        );
    }
    #[test]
    fn native_restore_preserves_conflicting_files_and_recovers_after_retry() {
        use std::{
            fs,
            os::{fd::AsRawFd, unix::fs::DirBuilderExt},
        };
        struct Temp(std::path::PathBuf);
        impl Drop for Temp {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let temp = Temp(std::env::temp_dir().join(format!("nd-qt-trash-{}", std::process::id())));
        fs::DirBuilder::new().mode(0o700).create(&temp.0).unwrap();
        let root = temp.0.join("local");
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let state = temp.0.join("state");
        assert_eq!(
            run_at(&state, &json!({"action":"list"}), &mut |_| 0).unwrap(),
            json!([])
        );
        runtime::value(json!({"op":"trash_init","state":state})).unwrap();
        fs::write(root.join("data"), b"original content").unwrap();
        let fd = fs::File::open(&root).unwrap();
        let snapshot =
            runtime::value(json!({"op":"trash_snapshot","parent":fd.as_raw_fd(),"name":"data"}))
                .unwrap();
        let moved=runtime::value(json!({"op":"trash_move","request":{"state":state,"data_home":temp.0.join("xdg"),"desktop":false,
            "root_fd":fd.as_raw_fd(),"root":root,"relative":"data","expected":snapshot[""],"expected_tree":null,"identifier":null}})).unwrap();
        assert_eq!(moved["ok"], true, "{moved}");
        let record = &moved["record"];
        let payload = Path::new(record["trash_path"].as_str().unwrap());
        assert!(!root.join("data").exists());
        let operation = json!({"action":"restore","state":state,"id":record["id"]});
        assert!(run_at(
            &state,
            &json!({"action":"restore","state":"/wrong/journal","id":record["id"]}),
            &mut |_| 0
        )
        .is_err());
        assert!(
            run_at(&state, &operation, &mut |_| 1)
                .unwrap_err()
                .cancelled
        );
        assert_eq!(fs::read(payload).unwrap(), b"original content");
        fs::write(root.join("data"), b"existing content").unwrap();
        assert!(run_at(&state, &operation, &mut |_| 0).is_err());
        assert_eq!(fs::read(payload).unwrap(), b"original content");
        assert_eq!(fs::read(root.join("data")).unwrap(), b"existing content");
        let listed = run_at(&state, &json!({"action":"list"}), &mut |_| 0).unwrap();
        assert_eq!(listed[0]["state"], json!(state));
        assert_eq!(listed[0]["record"]["status"], "recovery");
        fs::remove_file(root.join("data")).unwrap();
        assert_eq!(
            run_at(&state, &operation, &mut |_| 0).unwrap()["status"],
            "restored"
        );
        assert_eq!(fs::read(root.join("data")).unwrap(), b"original content");
        assert!(run_at(&state, &operation, &mut |_| 0).is_err());
        assert_eq!(fs::read(root.join("data")).unwrap(), b"original content");
    }
}
