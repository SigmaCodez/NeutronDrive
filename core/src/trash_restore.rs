//! Native recovery transaction. Payload moves are same-filesystem and never overwrite.
use crate::{
    filesystem,
    process::Failure,
    storage,
    trash_journal::{self, Operation},
};
use serde_json::{json, Value};
use std::{
    ffi::CString,
    fs::File,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, Failure>;
fn io(e: std::io::Error) -> Failure {
    e.to_string().into()
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Invalid trash receipt".into())
}
pub(crate) fn absolute(value: &str) -> Result<PathBuf> {
    let p = Path::new(value);
    if !p.is_absolute() || value.split('/').any(|s| s == ".." || s == ".") {
        return Err("An absolute path without dot components is required".into());
    }
    Ok(p.into())
}
pub(crate) fn identity(f: &File) -> Result<Value> {
    let m = f.metadata().map_err(io)?;
    Ok(json!([m.dev(), m.ino()]))
}
fn basename(p: &Path) -> Result<&str> {
    p.file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Invalid restore filename".into())
}
fn parent(p: &Path, private: bool) -> Result<File> {
    storage::directory(p.parent().ok_or("Invalid restore path")?, false, private)
}
pub(crate) fn chain(record: &Value) -> Result<()> {
    let root = absolute(text(record, "root_path")?)?;
    let relative = text(record, "relative")?;
    let parts: Vec<_> = relative.split('/').collect();
    if parts
        .iter()
        .any(|s| s.is_empty() || *s == "." || *s == ".." || s.contains(['\\', '\0']))
    {
        return Err("Unsafe relative trash path".into());
    }
    let mut fd = storage::directory(&root, false, false)?;
    let mut ids = vec![identity(&fd)?];
    for name in &parts[..parts.len() - 1] {
        fd = filesystem::open(fd.as_raw_fd(), name, libc::O_DIRECTORY)?;
        ids.push(identity(&fd)?);
    }
    if json!(ids) != record["parent_chain"] {
        return Err("The sync root or an ancestor directory changed".into());
    }
    Ok(())
}
pub(crate) fn same(a: &Value, b: &Value, renamed: bool) -> bool {
    let (Some(a), Some(b)) = (a.as_object(), b.as_object()) else {
        return false;
    };
    if a.len() != b.len() {
        return false;
    }
    a.iter().all(|(k, v)| {
        let Some(other) = b.get(k) else { return false };
        let mut v = v.clone();
        let mut other = other.clone();
        if renamed && k.is_empty() {
            if let Some(v) = v.as_object_mut() {
                v.remove("ctime_ns");
            }
            if let Some(v) = other.as_object_mut() {
                v.remove("ctime_ns");
            }
        }
        v == other
    })
}
fn exists(fd: &File, name: &str) -> Result<bool> {
    let name = CString::new(name).map_err(|_| "Invalid restore filename")?;
    let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            fd.as_raw_fd(),
            name.as_ptr(),
            st.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } == 0
    {
        return Ok(true);
    }
    let e = std::io::Error::last_os_error();
    if e.kind() == std::io::ErrorKind::NotFound {
        Ok(false)
    } else {
        Err(io(e))
    }
}
fn save(id: u64, record: &Value, hook: &mut impl FnMut(&str, &Value) -> Result<()>) -> Result<()> {
    hook("save", record)?;
    trash_journal::run(
        Operation::Save {
            id,
            record: record.clone(),
            initial: false,
        },
        &mut |_| 0,
    )?;
    Ok(())
}
fn verify_parent(path: &Path, fd: &File, private: bool) -> Result<()> {
    if identity(&parent(path, private)?)? != identity(fd)? {
        return Err("Restore destination parent changed".into());
    }
    Ok(())
}
fn restore(
    id: u64,
    record: &mut Value,
    destination: Option<&str>,
    callback: &mut impl FnMut(Value) -> i32,
    hook: &mut impl FnMut(&str, &Value) -> Result<()>,
) -> Result<()> {
    if matches!(record["status"].as_str(), Some("restored" | "aborted")) {
        return Err("This receipt cannot be restored again".into());
    }
    let target = absolute(destination.unwrap_or(text(record, "original_path")?))?;
    let original = destination.is_none();
    if original {
        chain(record)?;
    }
    let payload = absolute(text(record, "trash_path")?)?;
    if record["restore_path"]
        .as_str()
        .is_some_and(|p| !p.is_empty())
        && matches!(record["status"].as_str(), Some("restoring" | "recovery"))
    {
        let files = parent(&payload, true)?;
        if !exists(&files, basename(&payload)?)? {
            let previous = absolute(text(record, "restore_path")?)?;
            if destination.is_some() && target != previous {
                return Err("An interrupted restore has a different recorded destination".into());
            }
            let restored = parent(&previous, false)?;
            if identity(&restored)? != record["restore_parent_identity"] {
                return Err(format!(
                    "Interrupted restore parent changed; inspect {}",
                    previous.display()
                )
                .into());
            }
            let actual =
                filesystem::snapshot(restored.as_raw_fd(), basename(&previous)?, callback)?;
            if !same(&record["snapshot"], &actual, true) {
                return Err(format!(
                    "Interrupted restore contents changed; inspect {}",
                    previous.display()
                )
                .into());
            }
            restored.sync_all().map_err(io)?;
            verify_parent(&previous, &restored, false)?;
            record["status"] = json!("restored");
            record["error"] = json!("");
            save(id, record, hook)?;
            return Ok(());
        }
    }
    let files = parent(&payload, true)?;
    let dest = parent(&target, false)?;
    let before = filesystem::snapshot(files.as_raw_fd(), basename(&payload)?, callback)?;
    if !same(
        &record["snapshot"],
        &before,
        matches!(record["status"].as_str(), Some("prepared" | "recovery")),
    ) {
        return Err("Trash contents changed; inspect the recovery copy before restoring".into());
    }
    if json!(dest.metadata().map_err(io)?.dev()) != before[""]["device"] {
        return Err("Restore must remain on the same filesystem".into());
    }
    record["status"] = json!("restoring");
    record["restore_path"] = json!(target);
    record["restore_parent_identity"] = identity(&dest)?;
    save(id, record, hook)?;
    verify_parent(&payload, &files, true)?;
    verify_parent(&target, &dest, false)?;
    if original {
        chain(record)?;
    }
    filesystem::poll(callback)?;
    hook("rename", record)?;
    let source = CString::new(basename(&payload)?).map_err(|_| "Invalid restore filename")?;
    let to = CString::new(basename(&target)?).map_err(|_| "Invalid restore filename")?;
    if unsafe {
        libc::renameat2(
            files.as_raw_fd(),
            source.as_ptr(),
            dest.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } < 0
    {
        return Err(io(std::io::Error::last_os_error()));
    }
    dest.sync_all().map_err(io)?;
    files.sync_all().map_err(io)?;
    let after = filesystem::snapshot(dest.as_raw_fd(), basename(&target)?, callback)?;
    if !same(&before, &after, true) {
        return Err("Restored object changed; preserved at the restore destination".into());
    }
    if verify_parent(&target, &dest, false).is_err() {
        let location = std::fs::read_link(format!("/proc/self/fd/{}", dest.as_raw_fd()))
            .map_err(io)?
            .join(basename(&target)?);
        record["restore_path"] = json!(location);
        return Err(format!(
            "Restore parent moved concurrently; content remains at {}",
            location.display()
        )
        .into());
    }
    if original {
        chain(record)?;
    }
    record["status"] = json!("restored");
    record["error"] = json!("");
    save(id, record, hook)
}
pub fn run(
    state: &str,
    identifier: &str,
    destination: Option<&str>,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    run_with_hook(state, identifier, destination, callback, &mut |_, _| Ok(()))
}
fn run_with_hook(
    state: &str,
    identifier: &str,
    destination: Option<&str>,
    callback: &mut impl FnMut(Value) -> i32,
    hook: &mut impl FnMut(&str, &Value) -> Result<()>,
) -> Result<Value> {
    let id = trash_journal::run(
        Operation::Lock {
            state: state.into(),
            timeout: 5.0,
        },
        callback,
    )?
    .as_u64()
    .ok_or("Invalid trash transaction")?;
    struct Unlock(u64);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = trash_journal::run(Operation::Unlock { id: self.0 }, &mut |_| 0);
        }
    }
    let _unlock = Unlock(id);
    let mut record = trash_journal::run(
        Operation::Read {
            id,
            name: format!("{identifier}.json"),
        },
        callback,
    )?;
    let already_restored = record["status"] == "restored";
    let result = restore(id, &mut record, destination, callback, hook);
    if let Err(error) = &result {
        if !already_restored && matches!(record["status"].as_str(), Some("restoring" | "restored"))
        {
            record["status"] = json!("recovery");
            record["error"] = json!(error.message);
            let _ = save(id, &record, hook);
        }
    }
    // Deliver recovery metadata even when the host previously cancelled a poll.
    // The binding attaches it to the original callback exception after cleanup.
    callback(json!({"kind":"trash_receipt","record":record}));
    Ok(match result {
        Ok(()) => json!({"ok":true,"record":record}),
        Err(error) => {
            json!({"ok":false,"record":record,"error":error.message,"cancelled":error.cancelled})
        }
    })
}

/// Inspect only registered receipts; this never changes durable recovery evidence.
pub fn list(state: &str, callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    let id = trash_journal::run(
        Operation::Lock {
            state: state.into(),
            timeout: 5.0,
        },
        callback,
    )?
    .as_u64()
    .ok_or("Invalid trash journal handle")?;
    struct Unlock(u64);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = trash_journal::run(Operation::Unlock { id: self.0 }, &mut |_| 0);
        }
    }
    let _lock = Unlock(id);
    let names = trash_journal::run(Operation::Names { id }, callback)?;
    let mut records = Vec::new();
    for name in names.as_array().ok_or("Invalid trash journal listing")? {
        filesystem::poll(callback)?;
        let name = name.as_str().ok_or("Invalid receipt filename")?;
        if trash_journal::name(name).is_err() {
            continue;
        }
        let mut record = trash_journal::run(
            Operation::Read {
                id,
                name: name.into(),
            },
            callback,
        )?;
        if !matches!(record["status"].as_str(), Some("restored" | "aborted")) {
            match payload_identity(&record) {
                Ok(Some(actual)) => {
                    let proof = &record["snapshot"][""];
                    if actual != json!([proof["device"], proof["inode"]]) {
                        record["status"] = json!("recovery");
                        record["error"] =
                            json!("Trash object identity changed; automatic restore is blocked");
                    }
                }
                Ok(None) => {
                    if record["restore_path"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
                        && matches!(record["status"].as_str(), Some("restoring" | "recovery"))
                    {
                        record["status"] = json!("recovery");
                        record["error"] = json!(format!(
                            "Interrupted restore; inspect or retry the recorded destination: {}",
                            text(&record, "restore_path")?
                        ));
                    } else {
                        record["status"] = json!("missing");
                        record["error"] =
                            json!("Trash object is unavailable or was handled by the desktop");
                    }
                }
                Err(error) => {
                    record["status"] = json!("recovery");
                    record["error"] = json!(error.message);
                }
            }
        }
        records.push(record);
    }
    Ok(json!(records))
}

fn payload_identity(record: &Value) -> Result<Option<Value>> {
    use std::os::fd::FromRawFd;
    let path = absolute(text(record, "trash_path")?)?;
    let mut directory = File::open("/").map_err(io)?;
    for part in path
        .parent()
        .ok_or("Invalid trash path")?
        .components()
        .skip(1)
    {
        let name =
            CString::new(part.as_os_str().as_encoded_bytes()).map_err(|_| "Invalid trash path")?;
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) {
                return Ok(None);
            }
            return Err(io(error));
        }
        directory = unsafe { File::from_raw_fd(fd) };
    }
    let info = directory.metadata().map_err(io)?;
    if info.uid() != unsafe { libc::getuid() } || info.mode() & 0o077 != 0 {
        return Err("Trash directory must be owned by this user and private".into());
    }
    let name = CString::new(basename(&path)?).map_err(|_| "Invalid trash filename")?;
    let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            st.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        let error = std::io::Error::last_os_error();
        return if error.kind() == std::io::ErrorKind::NotFound {
            Ok(None)
        } else {
            Err(io(error))
        };
    }
    let st = unsafe { st.assume_init() };
    Ok(Some(json!([st.st_dev, st.st_ino])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    const ID: &str = "12345678-1234-1234-1234-123456789abc";
    struct Tree {
        base: PathBuf,
        state: PathBuf,
        root: PathBuf,
        payload: PathBuf,
    }
    impl Tree {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!(
                "nd-restore-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let state = base.join("state");
            let root = base.join("sync");
            let files = base.join("trash/files");
            storage::directory(&state.join("entries"), true, true).unwrap();
            let root_fd = storage::directory(&root, true, true).unwrap();
            let files_fd = storage::directory(&files, true, true).unwrap();
            let payload = files.join("payload");
            std::fs::write(&payload, b"preserved content").unwrap();
            let snapshot =
                filesystem::snapshot(files_fd.as_raw_fd(), "payload", &mut |_| 0).unwrap();
            let record = json!({"id":ID,"status":"trashed","original_path":root.join("data"),"trash_path":payload,"metadata_path":base.join("trash/info"),"deleted_at":"2026-10-23T00:00:00","kind":"file","backend":"managed","error":"","root_path":root,"relative":"data","parent_chain":[identity(&root_fd).unwrap()],"snapshot":snapshot});
            let id = trash_journal::run(
                Operation::Lock {
                    state: state.to_str().unwrap().into(),
                    timeout: 0.0,
                },
                &mut |_| 0,
            )
            .unwrap()
            .as_u64()
            .unwrap();
            trash_journal::run(
                Operation::Save {
                    id,
                    record,
                    initial: true,
                },
                &mut |_| 0,
            )
            .unwrap();
            trash_journal::run(Operation::Unlock { id }, &mut |_| 0).unwrap();
            Self {
                base,
                state,
                root,
                payload,
            }
        }
        fn run(&self) -> Value {
            run(self.state.to_str().unwrap(), ID, None, &mut |_| 0).unwrap()
        }
        fn record(&self) -> Value {
            serde_json::from_slice(
                &std::fs::read(self.state.join("entries").join(format!("{ID}.json"))).unwrap(),
            )
            .unwrap()
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }
    #[test]
    fn listing_preserves_journal_and_classifies_missing_payloads() {
        let tree = Tree::new();
        let state = tree.state.to_str().unwrap();
        let journal = tree.state.join("entries").join(format!("{ID}.json"));
        let original = std::fs::read(&journal).unwrap();
        std::fs::write(tree.state.join("entries/unrelated.tmp"), b"ignored").unwrap();
        let entries = list(state, &mut |_| 0).unwrap();
        assert_eq!(entries.as_array().unwrap().len(), 1);
        assert_eq!(entries[0]["status"], "trashed");
        std::fs::rename(&tree.payload, tree.base.join("preserved")).unwrap();
        assert_eq!(list(state, &mut |_| 0).unwrap()[0]["status"], "missing");
        std::fs::write(&tree.payload, b"replacement").unwrap();
        assert_eq!(list(state, &mut |_| 0).unwrap()[0]["status"], "recovery");
        assert_eq!(std::fs::read(journal).unwrap(), original);
    }

    #[test]
    fn cancelled_listing_releases_transaction_lock() {
        let tree = Tree::new();
        let state = tree.state.to_str().unwrap();
        assert!(list(state, &mut |_| 1).unwrap_err().cancelled);
        assert_eq!(list(state, &mut |_| 0).unwrap()[0]["status"], "trashed");
    }

    #[test]
    fn restores_without_overwrite_and_rejects_duplicate() {
        let tree = Tree::new();
        std::fs::write(tree.root.join("data"), b"new local data").unwrap();
        assert_eq!(tree.run()["ok"], false);
        assert!(tree.payload.exists());
        assert_eq!(
            std::fs::read(tree.root.join("data")).unwrap(),
            b"new local data"
        );
        std::fs::remove_file(tree.root.join("data")).unwrap();
        assert_eq!(tree.run()["ok"], true);
        assert_eq!(tree.run()["ok"], false);
        assert_eq!(tree.record()["status"], "restored");
        assert_eq!(
            std::fs::read(tree.root.join("data")).unwrap(),
            b"preserved content"
        );
    }
    #[test]
    fn interrupted_commit_is_reconciled_without_a_second_move() {
        let tree = Tree::new();
        let result = run_with_hook(
            tree.state.to_str().unwrap(),
            ID,
            None,
            &mut |_| 0,
            &mut |phase, record| {
                if phase == "save" && record["status"] != "restoring" {
                    Err("synthetic commit failure".into())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(tree.record()["status"], "restoring");
        assert!(!tree.payload.exists());
        assert!(tree.root.join("data").exists());
        assert_eq!(tree.run()["ok"], true);
        assert_eq!(tree.record()["status"], "restored");
    }
    #[test]
    fn moved_parent_reports_actual_location_with_or_without_replacement() {
        for replace in [false, true] {
            let tree = Tree::new();
            let old = tree.base.join("moved-root");
            let result = run_with_hook(
                tree.state.to_str().unwrap(),
                ID,
                None,
                &mut |_| 0,
                &mut |phase, _| {
                    if phase == "rename" {
                        std::fs::rename(&tree.root, &old).map_err(io)?;
                        if replace {
                            std::fs::create_dir(&tree.root).map_err(io)?;
                            std::fs::write(tree.root.join("data"), b"unrelated").map_err(io)?;
                        }
                    }
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(result["ok"], false);
            assert_eq!(result["record"]["status"], "recovery");
            assert_eq!(result["record"]["restore_path"], json!(old.join("data")));
            assert_eq!(
                std::fs::read(old.join("data")).unwrap(),
                b"preserved content"
            );
            if replace {
                assert_eq!(std::fs::read(tree.root.join("data")).unwrap(), b"unrelated");
            }
        }
    }
    #[test]
    fn failure_to_prepare_journal_never_moves_payload() {
        let tree = Tree::new();
        let result = run_with_hook(
            tree.state.to_str().unwrap(),
            ID,
            None,
            &mut |_| 0,
            &mut |_, _| Err("synthetic full disk".into()),
        )
        .unwrap();
        assert_eq!(result["ok"], false);
        assert!(tree.payload.exists());
        assert!(!tree.root.join("data").exists());
        assert_eq!(tree.record()["status"], "trashed");
    }
    #[test]
    fn cancelled_copy_proof_keeps_payload_and_receipt() {
        let tree = Tree::new();
        let mut polls = 0;
        let mut receipt = None;
        let result = run(tree.state.to_str().unwrap(), ID, None, &mut |event| {
            if event["kind"] == "trash_receipt" {
                receipt = Some(event["record"].clone());
                return 0;
            }
            polls += 1;
            i32::from(polls >= 3)
        })
        .unwrap();
        assert_eq!(result["cancelled"], true);
        assert!(receipt.is_some());
        assert!(tree.payload.exists());
        assert!(!tree.root.join("data").exists());
    }
}
