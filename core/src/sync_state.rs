//! Compatible v1/v2 baselines, bounded private reads and durable atomic writes.
use crate::{process::Failure, storage, sync_remote};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::File,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, Failure>;
const LIMIT: usize = 32 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Prepare {
        state: String,
    },
    Key {
        root: String,
        uid: String,
    },
    Load {
        state: String,
        key: String,
        root: String,
        uid: String,
    },
    Save {
        state: String,
        key: String,
        value: Value,
    },
}
fn io(e: std::io::Error) -> Failure {
    e.to_string().into()
}
fn path(state: &str, key: &str) -> Result<PathBuf> {
    if key.len() != 64
        || !key
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("Invalid sync baseline key".into());
    }
    Ok(Path::new(state).join(format!("{key}.json")))
}
fn relative(value: &str) -> Result<()> {
    if value
        .split('/')
        .any(|part| !sync_remote::name_error(part).is_empty())
    {
        return Err("Invalid relative path in sync baseline".into());
    }
    Ok(())
}
fn integer(value: &Value) -> bool {
    value.as_u64().is_some()
}
fn validate(mut value: Value, root: &str, uid: &str) -> Result<Value> {
    if !matches!(value["version"].as_u64(), Some(1 | 2))
        || value["local_root"] != root
        || value["remote_root_uid"] != uid
        || !value["entries"].is_object()
    {
        return Err("Sync baseline identity/schema does not match".into());
    }
    let v2 = value["version"] == 2;
    let identity = if v2 {
        let identity = &value["local_root_identity"];
        if !identity
            .as_array()
            .is_some_and(|v| v.len() == 2 && v.iter().all(integer))
        {
            return Err("Invalid v2 baseline root identity or deletion journal".into());
        }
        identity.clone()
    } else {
        Value::Null
    };
    let pending = if v2 {
        value.get("pending_deletions").cloned().unwrap_or(json!({}))
    } else {
        json!({})
    };
    let pending = pending
        .as_object()
        .ok_or("Invalid v2 baseline root identity or deletion journal")?;
    let entries = value["entries"].as_object_mut().unwrap();
    for (name, entry) in entries.iter_mut() {
        relative(name)?;
        if !entry.is_object()
            || !matches!(entry["kind"].as_str(), Some("file" | "folder"))
            || (entry["kind"] == "file"
                && !entry["sha1"]
                    .as_str()
                    .is_some_and(|s| s.len() == 40 && s.bytes().all(|c| c.is_ascii_hexdigit())))
        {
            return Err("Invalid sync baseline entry".into());
        }
        if !v2 {
            entry["identity_version"] = json!(1)
        }
        match entry["identity_version"].as_u64() {
            Some(2) => {
                if [
                    "size",
                    "local_device",
                    "local_inode",
                    "local_mtime_ns",
                    "local_ctime_ns",
                ]
                .iter()
                .any(|key| !integer(&entry[key]))
                    || !entry["remote_uid"].as_str().is_some_and(|v| !v.is_empty())
                    || !entry["remote_revision"].is_string()
                    || !entry["remote_path"].is_string()
                    || entry
                        .get("subtree_complete")
                        .is_some_and(|v| !v.is_boolean())
                {
                    return Err("Invalid v2 sync baseline identity".into());
                }
            }
            Some(1) => {}
            _ => return Err("Unknown baseline entry identity version".into()),
        }
    }
    for (name, record) in pending {
        relative(name)?;
        if !entries.contains_key(name)
            || !record.is_object()
            || !matches!(record["direction"].as_str(), Some("local" | "remote"))
            || !record["operation_id"].as_str().is_some_and(|s| {
                s.len() == 32
                    && s.bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            })
        {
            return Err("Invalid pending deletion journal".into());
        }
    }
    Ok(json!({"entries":entries,"identity":identity,"pending":pending}))
}
fn load(state: &str, key: &str, root: &str, uid: &str) -> Result<Value> {
    let path = path(state, key)?;
    let parent = storage::directory(Path::new(state), false, true)?;
    let name = CString::new(path.file_name().unwrap().as_encoded_bytes())
        .map_err(|_| "Invalid baseline name")?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(json!({"entries":{},"digest":"","pending":{},"identity":null}));
        }
        return Err(io(error));
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    let before = file.metadata().map_err(io)?;
    if !before.is_file()
        || before.len() > LIMIT as u64
        || before.uid() != unsafe { libc::getuid() }
        || before.nlink() != 1
    {
        return Err("Invalid or oversized sync baseline".into());
    }
    let mut raw = Vec::new();
    (&mut file)
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut raw)
        .map_err(io)?;
    let after = file.metadata().map_err(io)?;
    if raw.len() > LIMIT
        || (
            before.dev(),
            before.ino(),
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
            before.ctime(),
            before.ctime_nsec(),
        ) != (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec(),
        )
    {
        return Err("Sync baseline changed during read".into());
    }
    let value = serde_json::from_slice(&raw)
        .map_err(|_| "Corrupt sync baseline; preserved without changes")?;
    let mut result = validate(value, root, uid)?;
    result["digest"] = json!(format!("{:x}", Sha256::digest(&raw)));
    Ok(result)
}
pub fn run(operation: Operation) -> Result<Value> {
    match operation {
        Operation::Prepare { state } => {
            storage::directory(Path::new(&state), true, true)?;
            Ok(Value::Null)
        }
        Operation::Key { root, uid } => Ok(json!(format!(
            "{:x}",
            Sha256::digest(format!("{root}\0{uid}").as_bytes())
        ))),
        Operation::Load {
            state,
            key,
            root,
            uid,
        } => load(&state, &key, &root, &uid),
        Operation::Save { state, key, value } => {
            let target = path(&state, &key)?;
            let root = value["local_root"]
                .as_str()
                .ok_or("Missing baseline local root")?;
            let uid = value["remote_root_uid"]
                .as_str()
                .ok_or("Missing baseline remote identity")?;
            validate(value.clone(), root, uid)?;
            if serde_json::to_vec_pretty(&value)
                .map_err(|_| "Invalid sync baseline")?
                .len()
                + 1
                > LIMIT
            {
                return Err("Invalid or oversized sync baseline".into());
            }
            storage::write(&target, value)?;
            Ok(Value::Null)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::symlink,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    struct Tree(PathBuf);
    impl Tree {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "nd-baseline-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            storage::directory(&p, true, true).unwrap();
            Self(p)
        }
        fn state(&self) -> &str {
            self.0.to_str().unwrap()
        }
        fn path(&self) -> PathBuf {
            path(self.state(), KEY).unwrap()
        }
        fn load(&self) -> Result<Value> {
            load(self.state(), KEY, "/tmp/sync", "root-1")
        }
        fn save(&self, value: Value) -> Result<Value> {
            run(Operation::Save {
                state: self.state().into(),
                key: KEY.into(),
                value,
            })
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn value() -> Value {
        json!({"version":2,"local_root":"/tmp/sync","remote_root_uid":"root-1","local_root_identity":[1,2],"entries":{"data":{"kind":"file","sha1":"0000000000000000000000000000000000000000","identity_version":2,"size":4,"local_device":1,"local_inode":3,"local_mtime_ns":0,"local_ctime_ns":0,"remote_uid":"file-1","remote_revision":"rev-1","remote_path":"/my-files/data","subtree_complete":true}},"pending_deletions":{"data":{"direction":"local","operation_id":"0123456789abcdef0123456789abcdef"}}})
    }
    #[test]
    fn formats_roundtrip_and_raw_digest_remains_compatible() {
        let tree = Tree::new();
        assert_eq!(tree.load().unwrap()["digest"], "");
        tree.save(value()).unwrap();
        let loaded = tree.load().unwrap();
        assert_eq!(loaded["pending"], value()["pending_deletions"]);
        assert_eq!(loaded["identity"], json!([1, 2]));
        assert_eq!(
            loaded["digest"],
            format!("{:x}", Sha256::digest(std::fs::read(tree.path()).unwrap()))
        );
        assert_eq!(
            std::fs::metadata(tree.path()).unwrap().mode() & 0o777,
            0o600
        );
        let mut old = value();
        old["version"] = json!(1);
        old["entries"]["data"]
            .as_object_mut()
            .unwrap()
            .remove("identity_version");
        std::fs::write(tree.path(), serde_json::to_vec_pretty(&old).unwrap()).unwrap();
        let before = std::fs::read(tree.path()).unwrap();
        let loaded = tree.load().unwrap();
        assert_eq!(loaded["entries"]["data"]["identity_version"], 1);
        assert!(loaded["identity"].is_null());
        assert_eq!(loaded["pending"], json!({}));
        assert_eq!(std::fs::read(tree.path()).unwrap(), before);
    }
    #[test]
    fn corrupt_invalid_and_unsafe_baselines_remain_untouched() {
        let tree = Tree::new();
        std::fs::write(tree.path(), b"incomplete JSON").unwrap();
        assert!(tree
            .load()
            .unwrap_err()
            .message
            .contains("Corrupt sync baseline"));
        let original = std::fs::read(tree.path()).unwrap();
        let mut bad = value();
        bad["local_root_identity"] = json!([true, 2]);
        assert!(tree.save(bad).is_err());
        let mut bad = value();
        bad["pending_deletions"]["missing"] =
            json!({"direction":"local","operation_id":"0123456789abcdef0123456789abcdef"});
        assert!(tree.save(bad).is_err());
        assert_eq!(std::fs::read(tree.path()).unwrap(), original);
        let target = tree.0.join("preserved");
        std::fs::rename(tree.path(), &target).unwrap();
        symlink(&target, tree.path()).unwrap();
        assert!(tree.load().is_err());
        std::fs::remove_file(tree.path()).unwrap();
        std::fs::hard_link(&target, tree.path()).unwrap();
        assert!(tree.load().is_err());
        assert_eq!(std::fs::read(target).unwrap(), original);
        assert!(path(tree.state(), "../escape").is_err());
        assert!(relative("folder/../data").is_err());
    }
}
