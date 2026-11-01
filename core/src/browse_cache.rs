//! Persistent, session-isolated browsing snapshots. All I/O stays on workers.
//! Cloud operations remain authoritative; cache failures are ordinary misses.
use crate::{
    drive,
    process::{self, Failure},
    storage,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};
const LIMIT: usize = 64 * 1024 * 1024;
const BUDGET: u64 = 256 * 1024 * 1024;
static EPOCHS: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
fn root() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
        .filter(|p| p.is_absolute())
        .map(|p| p.join("neutrondrive/views-v1"))
}
fn identity(executable: &str, cb: &mut impl FnMut(Value) -> i32) -> Option<String> {
    let v = process::run(
        process::Request {
            version: 1,
            executable: executable.into(),
            args: vec!["cache".into(), "identity".into()],
            timeout: 5.,
            parse: true,
            login: false,
            max_output: 4096,
        },
        cb,
    )
    .ok()?;
    v["namespace"]
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_owned)
}
fn key(request: &Value) -> String {
    format!("{:x}.json", Sha256::digest(request.to_string().as_bytes()))
}
fn photo_request(id: &Value) -> Value {
    json!({"photo":id})
}
fn cacheable(op: &Value) -> bool {
    op["action"] == "list"
        || (op["action"] == "photo_library"
            && matches!(
                op["request"]["action"].as_str(),
                Some("index" | "details" | "albums")
            ))
}
fn mutation(op: &Value) -> bool {
    !cacheable(op)
        && !matches!(
            op["action"].as_str(),
            Some(
                "account_storage"
                    | "version"
                    | "info"
                    | "sharing"
                    | "versions"
                    | "download"
                    | "download_photo"
                    | "takeout_scan"
            )
        )
}
fn valid(op: &Value, v: &Value) -> bool {
    if op["action"] == "list" {
        return v.as_array().is_some_and(|a| a.len() <= 100000);
    }
    match op["request"]["action"].as_str() {
        Some("index") => v["items"]
            .as_array()
            .is_some_and(|a| a.len() <= 500000 && v["total"].as_u64() == Some(a.len() as u64)),
        Some("albums") => v["albums"].as_array().is_some_and(|a| a.len() <= 10000),
        Some("details") => op["request"]["ids"].as_array().is_some_and(|ids| {
            ids.len() <= 48
                && v["photos"].as_array().is_some_and(|p| {
                    p.len() <= ids.len() && p.iter().all(|p| ids.contains(&p["uid"]))
                })
        }),
        _ => false,
    }
}
fn read_entry(dir: &Path, request: &Value) -> Option<Value> {
    let v = storage::read_optional(&dir.join(key(request)), LIMIT, true).ok()??;
    (v["schema"] == 1 && v["request"] == *request).then(|| v["value"].clone())
}
fn read(dir: &Path, op: &Value) -> Option<Value> {
    if op["request"]["action"] == "details" {
        let ids = op["request"]["ids"].as_array()?;
        if ids.is_empty() || ids.len() > 48 {
            return None;
        }
        let photos: Option<Vec<_>> = ids
            .iter()
            .map(|id| read_entry(dir, &photo_request(id)))
            .collect();
        let v = json!({"photos":photos?});
        return valid(op, &v).then_some(v);
    }
    let v = read_entry(dir, op)?;
    valid(op, &v).then_some(v)
}
fn write_entry(dir: &Path, request: &Value, value: &Value) {
    if read_entry(dir, request).as_ref() == Some(value) {
        return;
    }
    let record = json!({"schema":1,"request":request,"value":value});
    if serde_json::to_vec(&record).is_ok_and(|b| b.len() <= LIMIT) {
        let _ = storage::write(&dir.join(key(request)), record);
    }
}
fn files(dir: &Path) -> Vec<(std::time::SystemTime, u64, PathBuf)> {
    if storage::directory(dir, false, true).is_err() {
        return vec![];
    }
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_str()?;
            if name.len() != 69
                || !name.ends_with(".json")
                || !name[..64].bytes().all(|b| b.is_ascii_hexdigit())
            {
                return None;
            }
            let m = e.path().symlink_metadata().ok()?;
            m.is_file().then(|| {
                (
                    m.modified().unwrap_or(std::time::UNIX_EPOCH),
                    m.len(),
                    e.path(),
                )
            })
        })
        .collect()
}
fn trim(dir: &Path, budget: u64) {
    let mut entries = files(dir);
    entries.sort_by_key(|e| e.0);
    let mut size: u64 = entries.iter().map(|e| e.1).sum();
    let mut count = entries.len();
    for (_, bytes, path) in entries {
        if size <= budget && count <= 10000 {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            size = size.saturating_sub(bytes);
            count -= 1;
        }
    }
}
fn write(dir: &Path, op: &Value, v: &Value) {
    if !valid(op, v) || storage::directory(dir, true, true).is_err() {
        return;
    }
    // Confirmed removals also discard stale disk previews/descendant views.
    if let Some(old) = read_entry(dir, op) {
        if op["request"]["action"] == "index"
            && op["request"]["album"].as_str().unwrap_or("").is_empty()
        {
            if let (Some(before), Some(after)) = (old["items"].as_array(), v["items"].as_array()) {
                let current: std::collections::HashSet<_> =
                    after.iter().filter_map(|p| p["uid"].as_str()).collect();
                for photo in before
                    .iter()
                    .filter(|p| !current.contains(p["uid"].as_str().unwrap_or("")))
                {
                    let _ = std::fs::remove_file(dir.join(key(&photo_request(&photo["uid"]))));
                }
            }
        } else if op["action"] == "list" {
            if let (Some(before), Some(after)) = (old.as_array(), v.as_array()) {
                let current: std::collections::HashSet<_> =
                    after.iter().filter_map(|p| p["uid"].as_str()).collect();
                let removed: Vec<_> = before
                    .iter()
                    .filter(|p| !current.contains(p["uid"].as_str().unwrap_or("")))
                    .filter_map(|p| p["path"].as_str())
                    .collect();
                if !removed.is_empty() {
                    for (_, _, file) in files(dir) {
                        if let Ok(Some(record)) = storage::read_optional(&file, LIMIT, true) {
                            if record["request"]["action"] == "list"
                                && record["request"]["path"].as_str().is_some_and(|path| {
                                    removed
                                        .iter()
                                        .any(|p| path == *p || path.starts_with(&format!("{p}/")))
                                })
                            {
                                let _ = std::fs::remove_file(file);
                            }
                        }
                    }
                }
            }
        }
    }
    if op["request"]["action"] == "details" {
        for id in op["request"]["ids"].as_array().unwrap() {
            let photo = v["photos"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["uid"] == *id)
                .cloned()
                .unwrap_or_else(|| json!({"uid":id,"safe":false,"thumbnail":""}));
            write_entry(dir, &photo_request(id), &photo);
        }
    } else {
        write_entry(dir, op, v);
    }
    trim(dir, BUDGET);
}
fn invalidate(namespace: &str, dir: &Path) {
    if let Ok(mut epochs) = EPOCHS.lock() {
        *epochs.entry(namespace.into()).or_default() += 1;
        trim(dir, 0);
    }
}
fn invalidate_operation(namespace: &str, dir: &Path, op: &Value) {
    if matches!(op["action"].as_str(), Some("login" | "logout")) {
        invalidate(namespace, dir);
        return;
    }
    if let Ok(mut epochs) = EPOCHS.lock() {
        *epochs.entry(namespace.into()).or_default() += 1;
        let photo_change = matches!(
            op["action"].as_str(),
            Some("photo_library" | "photo_import" | "takeout_import")
        );
        let mut paths: Vec<&str> = ["path", "parent", "destination"]
            .iter()
            .filter_map(|k| op[*k].as_str())
            .collect();
        if let Some(selected) = op["paths"].as_array() {
            paths.extend(selected.iter().filter_map(Value::as_str));
        }
        for (_, _, file) in files(dir) {
            let Ok(Some(record)) = storage::read_optional(&file, LIMIT, true) else {
                continue;
            };
            let request = &record["request"];
            let affected = if request["action"] == "list" && !photo_change {
                let path = request["path"].as_str().unwrap_or("");
                paths.is_empty()
                    || path.starts_with("/shared-")
                    || path == "/trash"
                    || path == "/devices"
                    || paths.iter().any(|p| {
                        path == *p
                            || path.starts_with(&format!("{p}/"))
                            || p.rsplit_once('/').is_some_and(|(parent, _)| parent == path)
                    })
            } else if photo_change {
                request["action"] == "photo_library"
                    || op["request"]["ids"]
                        .as_array()
                        .is_some_and(|ids| ids.contains(&request["photo"]))
            } else {
                false
            };
            if affected {
                let _ = std::fs::remove_file(file);
            }
        }
    }
}
pub fn forget(executable: &str, cb: &mut impl FnMut(Value) -> i32) {
    if let (Some(root), Some(namespace)) = (root(), identity(executable, cb)) {
        invalidate(&namespace, &root.join(&namespace));
    }
}
/// Only the UI uses cached reads. Sync and mutation validation always use live SDK reads.
pub fn run(
    executable: String,
    mut operation: Value,
    fresh: bool,
    cb: &mut impl FnMut(Value) -> i32,
) -> Result<Value, Failure> {
    let fresh = fresh || operation["_fresh"] == true;
    if let Some(o) = operation.as_object_mut() {
        o.remove("_fresh");
    }
    let namespace = identity(&executable, cb);
    let dir = namespace.as_ref().and_then(|n| root().map(|r| r.join(n)));
    if let (Some(n), Some(d)) = (&namespace, &dir) {
        if mutation(&operation) {
            invalidate_operation(n, d, &operation);
        }
    }
    let epoch = namespace
        .as_ref()
        .and_then(|n| EPOCHS.lock().ok().map(|e| *e.get(n).unwrap_or(&0)));
    if cacheable(&operation) && !fresh {
        if let Some(v) = dir.as_deref().and_then(|d| read(d, &operation)) {
            if cb(json!({"kind":"cache_hit"})) != 0 {
                return Err(Failure {
                    message: "Cancelled".into(),
                    cancelled: true,
                });
            }
            return Ok(v);
        }
    }
    let parsed = serde_json::from_value(operation.clone())
        .map_err(|_| Failure::from("Invalid browsing operation"))?;
    let value = drive::run(executable.clone(), parsed, cb)?;
    if cacheable(&operation) && namespace.is_some() && namespace == identity(&executable, cb) {
        if let (Some(n), Some(d), Some(epoch)) = (&namespace, &dir, epoch) {
            if let Ok(epochs) = EPOCHS.lock() {
                if *epochs.get(n).unwrap_or(&0) == epoch {
                    write(d, &operation, &value);
                }
            }
        }
    }
    Ok(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authoritative_inventory_removal_discards_preview_and_unrelated_views_survive_mutation() {
        let root = std::env::temp_dir().join(format!("nd-browse-removal-{}", std::process::id()));
        let photos = json!({"action":"photo_library","request":{"action":"details","ids":["p"]}});
        let index = json!({"action":"photo_library","request":{"action":"index","album":""}});
        write(
            &root,
            &photos,
            &json!({"photos":[{"uid":"p","thumbnail":"cached"}]}),
        );
        write(&root, &index, &json!({"items":[{"uid":"p"}],"total":1}));
        write(&root, &index, &json!({"items":[],"total":0}));
        assert!(read(&root, &photos).is_none());
        let a = json!({"action":"list","path":"/my-files/a"});
        let b = json!({"action":"list","path":"/my-files/b"});
        write(&root, &a, &json!([]));
        write(&root, &b, &json!([]));
        invalidate_operation(
            "removal-test",
            &root,
            &json!({"action":"rename","path":"/my-files/a/file","name":"new"}),
        );
        assert!(read(&root, &a).is_none());
        assert!(read(&root, &b).is_some());
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn snapshots_survive_reopen_and_apply_deletions_and_account_boundaries() {
        let root = std::env::temp_dir().join(format!("nd-browse-{}", std::process::id()));
        let a = root.join("a");
        let b = root.join("b");
        for path in [
            "/my-files",
            "/shared-with-me",
            "/shared-by-me",
            "/devices",
            "/trash",
        ] {
            let op = json!({"action":"list","path":path});
            write(&a, &op, &json!([{"uid":"one","name":"Old"}]));
            assert_eq!(read(&a, &op).unwrap()[0]["name"], "Old");
            assert!(read(&b, &op).is_none());
            write(&a, &op, &json!([]));
            assert_eq!(read(&a, &op), Some(json!([])));
        }
        let op = json!({"action":"photo_library","request":{"action":"details","ids":["a","b"]}});
        write(
            &a,
            &op,
            &json!({"photos":[{"uid":"a","thumbnail":"cached"},{"uid":"b"}]}),
        );
        let reordered =
            json!({"action":"photo_library","request":{"action":"details","ids":["b","a"]}});
        assert_eq!(
            read(&a, &reordered).unwrap()["photos"][1]["thumbnail"],
            "cached"
        );
        write(&a, &op, &json!({"photos":[]}));
        assert_eq!(read(&a, &op).unwrap()["photos"][0]["safe"], false);
        invalidate("a", &a);
        assert!(read(&a, &op).is_none());
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn corrupt_and_symlink_entries_are_misses_and_budget_is_bounded() {
        let root = std::env::temp_dir().join(format!("nd-browse-corrupt-{}", std::process::id()));
        let op = json!({"action":"list","path":"/my-files"});
        write(&root, &op, &json!([]));
        std::fs::write(root.join(key(&op)), b"bad json").unwrap();
        assert!(read(&root, &op).is_none());
        std::fs::remove_file(root.join(key(&op))).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", root.join(key(&op))).unwrap();
        assert!(read(&root, &op).is_none());
        std::fs::remove_file(root.join(key(&op))).unwrap();
        write(&root, &op, &json!([]));
        trim(&root, 1);
        assert!(read(&root, &op).is_none());
        assert!(!cacheable(&json!({"action":"download"})));
        assert!(mutation(&json!({"action":"trash"})));
        let _ = std::fs::remove_dir_all(root);
    }
}
