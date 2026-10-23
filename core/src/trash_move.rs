//! Journalled local removal. No overwrite, recursive unlink, or copy-delete path exists.
use crate::{
    filesystem,
    process::Failure,
    storage,
    trash_journal::{self, Operation},
    trash_location,
    trash_restore::{absolute, chain, identity, same},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    path::Path,
};
type Result<T> = std::result::Result<T, Failure>;
fn io(e: std::io::Error) -> Failure {
    e.to_string().into()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub state: String,
    pub data_home: String,
    pub desktop: bool,
    pub root_fd: i32,
    pub root: String,
    pub relative: String,
    pub expected: Value,
    pub expected_tree: Option<Value>,
    pub identifier: Option<String>,
}
pub(crate) fn identifier(value: Option<&str>) -> Result<String> {
    if let Some(value) = value {
        trash_journal::name(&format!("{value}.json"))?;
        return Ok(value.into());
    }
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(io)?;
    bytes[6] = bytes[6] & 15 | 64;
    bytes[8] = bytes[8] & 63 | 128;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}
fn date() -> Result<String> {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    if unsafe { libc::localtime_r(&now, local.as_mut_ptr()) }.is_null() {
        return Err("Cannot determine local deletion time".into());
    }
    let local = unsafe { local.assume_init() };
    let mut output = [0u8; 64];
    let n = unsafe {
        libc::strftime(
            output.as_mut_ptr().cast(),
            output.len(),
            c"%Y-%m-%dT%H:%M:%S".as_ptr(),
            &local,
        )
    };
    if n == 0 {
        return Err("Cannot format deletion time".into());
    }
    String::from_utf8(output[..n].to_vec()).map_err(|_| "Invalid deletion time".into())
}
fn encoded(path: &Path) -> String {
    path.as_os_str()
        .as_encoded_bytes()
        .iter()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"/-._~".contains(b) {
                (*b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn rename(source: &File, name: &str, target: &File, destination: &str) -> Result<()> {
    let name = CString::new(name).map_err(|_| "Invalid source filename")?;
    let destination = CString::new(destination).map_err(|_| "Invalid trash filename")?;
    if unsafe {
        libc::renameat2(
            source.as_raw_fd(),
            name.as_ptr(),
            target.as_raw_fd(),
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } != 0
    {
        return Err(io(std::io::Error::last_os_error()));
    }
    Ok(())
}
fn check_directory(path: &Path, fd: &File) -> Result<()> {
    if identity(&storage::directory(path, false, true)?)? != identity(fd)? {
        return Err("Trash directory was replaced during the operation".into());
    }
    Ok(())
}
fn prove(snapshot: &Value, request: &Request) -> Result<()> {
    let values = if snapshot[""]["kind"] == "folder" {
        request
            .expected_tree
            .clone()
            .ok_or("Trashing a folder requires a complete unchanged subtree proof")?
    } else {
        json!({request.relative.clone():request.expected})
    };
    let values = values.as_object().ok_or("Invalid deletion plan")?;
    let snapshot = snapshot.as_object().ok_or("Invalid local snapshot")?;
    if values.len() != snapshot.len() {
        return Err("The local subtree differs from the deletion plan".into());
    }
    for (suffix, actual) in snapshot {
        let key = if suffix.is_empty() {
            request.relative.clone()
        } else {
            format!("{}/{suffix}", request.relative)
        };
        let wanted = values
            .get(&key)
            .ok_or("The local subtree differs from the deletion plan")?;
        let mut fields = vec!["kind", "device", "inode"];
        if actual["kind"] == "file" {
            fields.extend(["size", "sha1", "mtime_ns"]);
            if wanted["ctime_ns"].as_i64().is_some_and(|v| v != 0) {
                fields.push("ctime_ns")
            }
        }
        if fields.iter().any(|field| wanted[field] != actual[field]) {
            return Err("The local object changed since the deletion plan".into());
        }
    }
    Ok(())
}
fn cheap_proof(fd: &File, name: &str, expected: &Value) -> Result<()> {
    let name = CString::new(name).map_err(|_| "Invalid local filename")?;
    let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            fd.as_raw_fd(),
            name.as_ptr(),
            st.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(io(std::io::Error::last_os_error()));
    }
    let st = unsafe { st.assume_init() };
    if json!(st.st_dev) != expected["device"]
        || json!(st.st_ino) != expected["inode"]
        || json!(st.st_mtime as i128 * 1_000_000_000 + st.st_mtime_nsec as i128)
            != expected["mtime_ns"]
        || json!(st.st_ctime as i128 * 1_000_000_000 + st.st_ctime_nsec as i128)
            != expected["ctime_ns"]
        || (expected["kind"] == "file" && json!(st.st_size) != expected["size"])
    {
        return Err("Source changed during final deletion validation".into());
    }
    Ok(())
}
fn save(
    id: u64,
    record: &Value,
    initial: bool,
    hook: &mut impl FnMut(&str, &Value) -> Result<()>,
) -> Result<bool> {
    hook(if initial { "prepare" } else { "save" }, record)?;
    trash_journal::run(
        Operation::Save {
            id,
            record: record.clone(),
            initial,
        },
        &mut |_| 0,
    )?
    .as_bool()
    .ok_or_else(|| "Invalid journal save result".into())
}
pub fn run(request: Request, callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    run_with_hook(request, callback, &mut |_, _| Ok(()))
}
fn run_with_hook(
    request: Request,
    callback: &mut impl FnMut(Value) -> i32,
    hook: &mut impl FnMut(&str, &Value) -> Result<()>,
) -> Result<Value> {
    let root = absolute(&request.root)?;
    if absolute(&request.state)?.starts_with(&root) {
        return Err("Trash journal must be outside the synchronized folder".into());
    }
    let parts: Vec<_> = request.relative.split('/').collect();
    if parts
        .iter()
        .any(|s| s.is_empty() || *s == "." || *s == ".." || s.contains(['\\', '\0']))
    {
        return Err("Unsafe relative trash path".into());
    }
    let identifier = identifier(request.identifier.as_deref())?;
    let id = trash_journal::run(
        Operation::Lock {
            state: request.state.clone(),
            timeout: 5.0,
        },
        callback,
    )?
    .as_u64()
    .ok_or("Invalid journal handle")?;
    struct Unlock(u64);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = trash_journal::run(Operation::Unlock { id: self.0 }, &mut |_| 0);
        }
    }
    let _lock = Unlock(id);
    let mut parent = filesystem::duplicate(request.root_fd)?;
    let mut parents = vec![identity(&parent)?];
    for part in &parts[..parts.len() - 1] {
        parent = filesystem::open(parent.as_raw_fd(), part, libc::O_DIRECTORY)?;
        parents.push(identity(&parent)?);
    }
    let name = parts[parts.len() - 1];
    let mut record = json!({"root_path":root,"relative":request.relative,"parent_chain":parents});
    chain(&record)?;
    hook("hash", &record)?;
    let snapshot = filesystem::snapshot(parent.as_raw_fd(), name, callback)?;
    prove(&snapshot, &request)?;
    let location = trash_location::choose(
        &request.root,
        &request.data_home,
        snapshot[""]["device"].as_u64().ok_or("Invalid device")?,
        request.desktop,
    )?;
    let trash = absolute(location["path"].as_str().ok_or("Invalid trash location")?)?;
    let mut end = name.len().min(80);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    let trash_name = format!("{}-{identifier}", &name[..end]);
    let deleted_at = date()?;
    record = json!({"id":identifier,"original_path":root.join(&request.relative),"trash_path":trash.join("files").join(&trash_name),"metadata_path":trash.join("info").join(format!("{trash_name}.trashinfo")),"deleted_at":deleted_at,"kind":snapshot[""]["kind"],"status":"prepared","backend":location["backend"],"error":"","root_path":root,"relative":request.relative,"parent_chain":parents,"snapshot":snapshot});
    if !save(id, &record, true, hook)? {
        let existing = trash_journal::run(
            Operation::Read {
                id,
                name: format!("{identifier}.json"),
            },
            callback,
        )?;
        callback(json!({"kind":"trash_receipt","record":existing}));
        return Ok(
            json!({"ok":false,"record":existing,"error":"This trash operation already has a receipt; inspect it before retrying","cancelled":false}),
        );
    }
    let result = (|| -> Result<()> {
        let files_path = trash.join("files");
        let info_path = trash.join("info");
        let files = storage::directory(&files_path, false, true)?;
        let info = storage::directory(&info_path, false, true)?;
        let original = root.join(&request.relative);
        let top = location["top"].as_str().map(Path::new);
        let stored = if let Some(top) = top {
            original
                .strip_prefix(top)
                .map_err(|_| "Invalid trash mount root")?
        } else {
            &original
        };
        let contents = format!(
            "[Trash Info]\nPath={}\nDeletionDate={deleted_at}\n",
            encoded(stored)
        );
        let metadata = CString::new(format!("{trash_name}.trashinfo"))
            .map_err(|_| "Invalid trash metadata filename")?;
        let fd = unsafe {
            libc::openat(
                info.as_raw_fd(),
                metadata.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io(std::io::Error::last_os_error()));
        }
        let mut output = unsafe { File::from_raw_fd(fd) };
        output.write_all(contents.as_bytes()).map_err(io)?;
        output.sync_all().map_err(io)?;
        info.sync_all().map_err(io)?;
        chain(&record)?;
        hook("hash", &record)?;
        let before = filesystem::snapshot(parent.as_raw_fd(), name, callback)?;
        if !same(&snapshot, &before, false) {
            return Err("The source changed before it could be moved to trash".into());
        }
        if callback(json!({"kind":"trash_validate"})) != 0 {
            return Err(Failure {
                message: "Final deletion validation failed".into(),
                cancelled: true,
            });
        }
        filesystem::poll(callback)?;
        chain(&record)?;
        cheap_proof(&parent, name, &before[""])?;
        check_directory(&files_path, &files)?;
        check_directory(&info_path, &info)?;
        hook("rename", &record)?;
        rename(&parent, name, &files, &trash_name)?;
        let verify = (|| -> Result<Value> {
            hook("after_rename", &record)?;
            parent.sync_all().map_err(io)?;
            files.sync_all().map_err(io)?;
            hook("hash", &record)?;
            let after = filesystem::snapshot(files.as_raw_fd(), &trash_name, callback)?;
            if !same(&snapshot, &after, true) {
                return Err("Source was replaced or modified during the move".into());
            }
            chain(&record)?;
            check_directory(&files_path, &files)?;
            Ok(after)
        })();
        match verify {
            Ok(after) => {
                record["snapshot"] = after;
                record["status"] = json!("trashed");
                save(id, &record, false, hook)?;
                Ok(())
            }
            Err(error) => {
                let rollback = (|| -> Result<()> {
                    chain(&record)?;
                    rename(&files, &trash_name, &parent, name)?;
                    parent.sync_all().map_err(io)?;
                    files.sync_all().map_err(io)?;
                    Ok(())
                })();
                if rollback.is_ok() {
                    record["status"] = json!("aborted")
                } else {
                    record["status"] = json!("recovery");
                    for (key, fd, basename) in [
                        ("trash_path", &files, trash_name.clone()),
                        ("metadata_path", &info, format!("{trash_name}.trashinfo")),
                    ] {
                        if let Ok(actual) =
                            std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd()))
                        {
                            record[key] = json!(actual.join(basename))
                        }
                    }
                }
                Err(error)
            }
        }
    })();
    if let Err(error) = &result {
        if !matches!(record["status"].as_str(), Some("aborted" | "recovery")) {
            record["status"] = json!("recovery")
        }
        record["error"] = json!(error.message);
        let _ = save(id, &record, false, hook);
    }
    callback(json!({"kind":"trash_receipt","record":record}));
    Ok(match result {
        Ok(()) => json!({"ok":true,"record":record}),
        Err(error) => {
            json!({"ok":false,"record":record,"error":error.message,"cancelled":error.cancelled})
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::RefCell,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Tree {
        base: PathBuf,
        root: PathBuf,
        state: PathBuf,
        fd: File,
    }
    impl Tree {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!(
                "nd-move-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let root = base.join("sync");
            let state = base.join("state");
            let fd = storage::directory(&root, true, true).unwrap();
            storage::directory(&state.join("entries"), true, true).unwrap();
            std::fs::write(root.join("data"), b"original content").unwrap();
            Self {
                base,
                root,
                state,
                fd,
            }
        }
        fn request(&self, relative: &str) -> Request {
            let path = self.root.join(relative);
            let parent = storage::directory(path.parent().unwrap(), false, false).unwrap();
            let snapshot = filesystem::snapshot(
                parent.as_raw_fd(),
                path.file_name().unwrap().to_str().unwrap(),
                &mut |_| 0,
            )
            .unwrap();
            Request {
                state: self.state.to_str().unwrap().into(),
                data_home: self.base.join("xdg").to_str().unwrap().into(),
                desktop: true,
                root_fd: self.fd.as_raw_fd(),
                root: self.root.to_str().unwrap().into(),
                relative: relative.into(),
                expected: snapshot[""].clone(),
                expected_tree: None,
                identifier: None,
            }
        }
        fn execute(&self, hook: &mut impl FnMut(&str, &Value) -> Result<()>) -> Value {
            run_with_hook(self.request("data"), &mut |_| 0, hook).unwrap()
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }
    fn path(record: &Value, field: &str) -> PathBuf {
        PathBuf::from(record[field].as_str().unwrap())
    }
    #[test]
    fn collision_preserves_source_and_existing_trash() {
        let tree = Tree::new();
        let result = tree.execute(&mut |phase, r| {
            if phase == "rename" {
                std::fs::write(path(r, "trash_path"), b"other trash").unwrap();
            }
            Ok(())
        });
        assert_eq!(result["ok"], false);
        assert_eq!(
            std::fs::read(tree.root.join("data")).unwrap(),
            b"original content"
        );
        assert_eq!(
            std::fs::read(path(&result["record"], "trash_path")).unwrap(),
            b"other trash"
        );
    }
    #[test]
    fn last_window_source_replacement_rolls_back_without_loss() {
        let tree = Tree::new();
        let result = tree.execute(&mut |phase, _| {
            if phase == "rename" {
                std::fs::rename(tree.root.join("data"), tree.root.join("kept")).unwrap();
                std::fs::write(tree.root.join("data"), b"replacement").unwrap();
            }
            Ok(())
        });
        assert_eq!(result["record"]["status"], "aborted");
        assert_eq!(
            std::fs::read(tree.root.join("data")).unwrap(),
            b"replacement"
        );
        assert_eq!(
            std::fs::read(tree.root.join("kept")).unwrap(),
            b"original content"
        );
    }
    #[test]
    fn edited_payload_and_occupied_source_are_both_preserved() {
        let tree = Tree::new();
        let result = tree.execute(&mut |phase, r| {
            if phase == "after_rename" {
                std::fs::write(tree.root.join("data"), b"new source").unwrap();
                std::fs::write(path(r, "trash_path"), b"concurrent edit").unwrap();
            }
            Ok(())
        });
        assert_eq!(result["record"]["status"], "recovery");
        assert_eq!(
            std::fs::read(tree.root.join("data")).unwrap(),
            b"new source"
        );
        assert_eq!(
            std::fs::read(path(&result["record"], "trash_path")).unwrap(),
            b"concurrent edit"
        );
    }
    #[test]
    fn prepared_receipt_survives_failed_commit_and_can_restore() {
        let tree = Tree::new();
        let result = tree.execute(&mut |phase, _| {
            if phase == "save" {
                Err("synthetic fsync failure".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(result["ok"], false);
        let records = crate::trash_restore::list(tree.state.to_str().unwrap(), &mut |_| 0).unwrap();
        assert_eq!(records[0]["status"], "prepared");
        crate::trash_restore::run(
            tree.state.to_str().unwrap(),
            records[0]["id"].as_str().unwrap(),
            None,
            &mut |_| 0,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(tree.root.join("data")).unwrap(),
            b"original content"
        );
    }
    #[test]
    fn failed_prepare_does_not_move_source() {
        let tree = Tree::new();
        assert!(
            run_with_hook(
                tree.request("data"),
                &mut |_| 0,
                &mut |phase, _| if phase == "prepare" {
                    Err("synthetic full disk".into())
                } else {
                    Ok(())
                }
            )
            .is_err()
        );
        assert!(tree.root.join("data").exists());
        assert_eq!(
            crate::trash_restore::list(tree.state.to_str().unwrap(), &mut |_| 0).unwrap(),
            json!([])
        );
    }
    #[test]
    fn failed_rename_does_not_copy_or_delete() {
        let tree = Tree::new();
        let result = tree.execute(&mut |phase, _| {
            if phase == "rename" {
                Err(io(std::io::Error::from_raw_os_error(libc::EXDEV)))
            } else {
                Ok(())
            }
        });
        assert_eq!(result["ok"], false);
        assert_eq!(
            std::fs::read(tree.root.join("data")).unwrap(),
            b"original content"
        );
        assert!(!path(&result["record"], "trash_path").exists());
    }
    #[test]
    fn moved_source_parent_does_not_touch_replacement() {
        let tree = Tree::new();
        std::fs::create_dir(tree.root.join("folder")).unwrap();
        std::fs::write(tree.root.join("folder/one"), b"one").unwrap();
        let result = run_with_hook(tree.request("folder/one"), &mut |_| 0, &mut |phase, _| {
            if phase == "rename" {
                std::fs::rename(tree.root.join("folder"), tree.root.join("old-folder")).unwrap();
                std::fs::create_dir(tree.root.join("folder")).unwrap();
                std::fs::write(tree.root.join("folder/one"), b"unrelated replacement").unwrap();
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(result["record"]["status"], "recovery");
        assert_eq!(
            std::fs::read(path(&result["record"], "trash_path")).unwrap(),
            b"one"
        );
        assert_eq!(
            std::fs::read(tree.root.join("folder/one")).unwrap(),
            b"unrelated replacement"
        );
    }
    #[test]
    fn remote_proof_follows_hashes_and_precedes_rename() {
        let tree = Tree::new();
        let events = RefCell::new(Vec::new());
        let result = run_with_hook(
            tree.request("data"),
            &mut |event| {
                if event["kind"] == "trash_validate" {
                    events.borrow_mut().push("remote-proof".to_string())
                }
                0
            },
            &mut |phase, _| {
                if matches!(phase, "hash" | "rename") {
                    events.borrow_mut().push(phase.into())
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(
            *events.borrow(),
            ["hash", "hash", "remote-proof", "rename", "hash"]
        );
    }
}
