//! Owned online-drive file descriptors, edit journals and local write lifecycle.
use crate::{
    ondemand_io::{self as io, fail, Error, Result},
    process::Failure,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::fs::{FileExt, MetadataExt},
    },
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        LazyLock, Mutex,
    },
};
struct Handle {
    file: File,
    writable: bool,
    append: bool,
    work: Option<PathBuf>,
    journal: Option<PathBuf>,
    record: Option<Value>,
    dirty: bool,
}
static HANDLES: LazyLock<Mutex<BTreeMap<u64, Handle>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static NEXT: AtomicU64 = AtomicU64::new(1);
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Recovery {
        directory: PathBuf,
        active: Vec<PathBuf>,
    },
    Adopt {
        fd: RawFd,
    },
    Edit {
        recovery: PathBuf,
        source: Option<PathBuf>,
        node: Option<Value>,
        append: bool,
    },
    Journal {
        id: u64,
        value: Value,
    },
    Read {
        id: u64,
        offset: i64,
        size: u64,
    },
    Write {
        id: u64,
        offset: i64,
        content: String,
    },
    Truncate {
        id: u64,
        size: i64,
    },
    Sync {
        id: u64,
    },
    Settled {
        id: u64,
    },
    Release {
        id: u64,
    },
    Abort {
        id: u64,
    },
}
fn duplicate(fd: RawFd) -> Result<File> {
    let fd = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let stat = file.metadata()?;
    if !stat.is_file()
        || stat.uid() != unsafe { libc::getuid() }
        || stat.nlink() != 1
        || stat.mode() & 0o077 != 0
    {
        return Err(fail(
            libc::EACCES,
            "Cache object is not a private regular file",
        ));
    }
    Ok(file)
}
fn insert(handle: Handle) -> Result<Value> {
    let id = NEXT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| fail(libc::EMFILE, "Handle identifiers exhausted"))?;
    let value =
        json!({"id":id,"fd":handle.file.as_raw_fd(),"work":handle.work,"journal":handle.journal});
    HANDLES.lock().unwrap().insert(id, handle);
    Ok(value)
}
fn edit(
    recovery: PathBuf,
    source: Option<PathBuf>,
    node: Option<Value>,
    append: bool,
) -> Result<Value> {
    let directory = io::private_directory(&recovery)?;
    let mut bytes = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let filename = format!("{token}.data");
    let cname = std::ffi::CString::new(filename.clone()).unwrap();
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            cname.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let prepare = (|| -> Result<()> {
        if let Some(source) = source {
            let node =
                node.ok_or_else(|| fail(libc::EINVAL, "Missing verified source metadata"))?;
            let input = io::verified(&source, &node)?;
            let mut buffer = vec![0; 1024 * 1024];
            let mut offset = 0;
            loop {
                let count = input.read_at(&mut buffer, offset)?;
                if count == 0 {
                    break;
                }
                file.write_all_at(&buffer[..count], offset)?;
                offset += count as u64;
            }
            let (sha, size, _) = io::digest(&file)?;
            if sha != io::field(&node, "sha1").to_lowercase() || Some(size) != node["size"].as_u64()
            {
                return Err(fail(
                    libc::ESTALE,
                    "Cache content changed while preparing the edit",
                ));
            }
        }
        file.sync_all()?;
        directory.sync_all()?;
        let named = io::regular(&recovery.join(&filename), libc::O_RDONLY)?.metadata()?;
        let opened = file.metadata()?;
        if (named.dev(), named.ino()) != (opened.dev(), opened.ino()) {
            return Err(fail(libc::EBUSY, "Recovery data path was replaced"));
        }
        Ok(())
    })();
    if let Err(error) = prepare {
        unsafe {
            libc::unlinkat(directory.as_raw_fd(), cname.as_ptr(), 0);
        }
        return Err(error);
    }
    insert(Handle {
        file,
        writable: true,
        append,
        work: Some(recovery.join(filename)),
        journal: Some(recovery.join(format!("{token}.json"))),
        record: None,
        dirty: false,
    })
}
fn persist(handle: &mut Handle, value: Value) -> Result<()> {
    if !handle.writable {
        return Err(fail(libc::EBADF, "Read-only handle"));
    }
    let work = handle.work.as_ref().unwrap();
    if value["data"] != work.file_name().unwrap().to_string_lossy().as_ref()
        || value["version"] != 1
    {
        return Err(fail(libc::EINVAL, "Invalid recovery metadata"));
    }
    // Retain dirty state even if persistence fails, so release cannot erase bytes.
    if value["state"] != "clean" {
        handle.dirty = true;
    }
    handle.record = Some(value.clone());
    io::write_json(handle.journal.as_ref().unwrap(), value)?;
    Ok(())
}
fn writable(handle: &Handle) -> Result<()> {
    if !handle.writable {
        return Err(fail(libc::EBADF, "Read-only handle"));
    }
    if !handle.dirty || handle.record.is_none() {
        return Err(fail(
            libc::EIO,
            "Persist an edit intent before changing file content",
        ));
    }
    // Re-persist before every mutation, including retries after a journal error.
    io::write_json(
        handle.journal.as_ref().unwrap(),
        handle.record.clone().unwrap(),
    )
}
fn remove_work(handle: &Handle) -> Result<()> {
    if let Some(work) = &handle.work {
        match io::regular(work, libc::O_RDONLY) {
            Ok(named) => {
                let a = named.metadata()?;
                let b = handle.file.metadata()?;
                if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
                    return Err(fail(libc::EBUSY, "Recovery data path was replaced"));
                }
                std::fs::remove_file(work)?;
            }
            Err(Error::Fs(libc::ENOENT, _)) => {}
            Err(e) => return Err(e),
        }
        if let Some(journal) = &handle.journal {
            match std::fs::remove_file(journal) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        io::private_directory(work.parent().unwrap())?.sync_all()?;
    }
    Ok(())
}
fn release(mut handle: Handle, abort: bool) -> Result<Value> {
    if abort || !handle.dirty {
        remove_work(&handle)?;
        return Ok(Value::Null);
    }
    let mut record = handle.record.clone().ok_or_else(|| {
        fail(
            libc::EIO,
            "Missing recovery metadata; preserve local content",
        )
    })?;
    record["state"] = json!("recovery");
    if io::field(&record, "error").is_empty() {
        record["error"] = json!("Handle closed without confirmed writeback");
    }
    persist(&mut handle, record)?;
    handle.file.sync_all()?;
    Ok(Value::Null)
}
pub(crate) fn pending(directory: &std::path::Path, active: &[PathBuf]) -> Result<Value> {
    io::private_directory(directory)?;
    let mut paths = std::fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    let mut result = Vec::new();
    for path in paths {
        if path.extension().and_then(|s| s.to_str()) != Some("json") || active.contains(&path) {
            continue;
        }
        let mut value = io::read_json(&path)?;
        if !value.is_object() {
            return Err(fail(libc::EIO, "Invalid recovery metadata"));
        }
        if value["state"] == "clean" {
            continue;
        }
        let name = io::field(&value, "data");
        if name.len() != 37
            || !name.ends_with(".data")
            || !name.as_bytes()[..32]
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        {
            return Err(fail(libc::EIO, "Invalid recovery metadata"));
        }
        let data = directory.join(name);
        io::regular(&data, libc::O_RDONLY)?;
        value["data_path"] = json!(data);
        value["journal_path"] = json!(path);
        result.push(value);
    }
    Ok(json!(result))
}
pub(crate) fn apply(operation: Operation) -> Result<Value> {
    match operation {
        Operation::Recovery { directory, active } => return pending(&directory, &active),
        Operation::Adopt { fd } => {
            return insert(Handle {
                file: duplicate(fd)?,
                writable: false,
                append: false,
                work: None,
                journal: None,
                record: None,
                dirty: false,
            })
        }
        Operation::Edit {
            recovery,
            source,
            node,
            append,
        } => return edit(recovery, source, node, append),
        Operation::Release { id } | Operation::Abort { id } => {
            let abort = matches!(operation, Operation::Abort { .. });
            let handle = HANDLES
                .lock()
                .unwrap()
                .remove(&id)
                .ok_or_else(|| fail(libc::EBADF, "Unknown file handle"))?;
            return release(handle, abort);
        }
        _ => {}
    }
    let id = match &operation {
        Operation::Journal { id, .. }
        | Operation::Read { id, .. }
        | Operation::Write { id, .. }
        | Operation::Truncate { id, .. }
        | Operation::Sync { id }
        | Operation::Settled { id } => *id,
        _ => unreachable!(),
    };
    let mut handles = HANDLES.lock().unwrap();
    let handle = handles
        .get_mut(&id)
        .ok_or_else(|| fail(libc::EBADF, "Unknown file handle"))?;
    Ok(match operation {
        Operation::Journal { value, .. } => {
            persist(handle, value)?;
            Value::Null
        }
        Operation::Read { offset, size, .. } => {
            if offset < 0 {
                return Err(fail(libc::EINVAL, "Invalid read range"));
            }
            let mut buffer = vec![0; (size.min(8 * 1024 * 1024)) as usize];
            let count = handle.file.read_at(&mut buffer, offset as u64)?;
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let mut encoded = String::with_capacity(count * 2);
            for byte in &buffer[..count] {
                encoded.push(HEX[(byte >> 4) as usize] as char);
                encoded.push(HEX[(byte & 15) as usize] as char);
            }
            json!(encoded)
        }
        Operation::Write {
            offset, content, ..
        } => {
            if offset < 0 || content.len() % 2 != 0 || content.len() > 16 * 1024 * 1024 {
                return Err(fail(libc::EINVAL, "Invalid write range"));
            }
            let data: std::result::Result<Vec<u8>, _> = content
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| {
                    std::str::from_utf8(pair)
                        .ok()
                        .and_then(|s| u8::from_str_radix(s, 16).ok())
                        .ok_or_else(|| fail(libc::EINVAL, "Invalid write content"))
                })
                .collect();
            let data = data?;
            writable(handle)?;
            let offset = if handle.append {
                handle.file.metadata()?.len()
            } else {
                offset as u64
            };
            let written = handle.file.write_at(&data, offset)?;
            handle.file.sync_all()?;
            json!(written)
        }
        Operation::Truncate { size, .. } => {
            if size < 0 {
                return Err(fail(libc::EINVAL, "Negative size"));
            }
            writable(handle)?;
            handle.file.set_len(size as u64)?;
            handle.file.sync_all()?;
            Value::Null
        }
        Operation::Sync { .. } => {
            handle.file.sync_all()?;
            Value::Null
        }
        Operation::Settled { .. } => {
            handle.dirty = false;
            Value::Null
        }
        _ => unreachable!(),
    })
}
pub fn run(operation: Operation) -> std::result::Result<Value, Failure> {
    match apply(operation) {
        Ok(value) => Ok(json!({"value":value})),
        Err(Error::Fs(errno, message)) => Ok(json!({"errno":errno,"message":message})),
        Err(Error::Backend(e)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "nd-handles-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            io::private_directory(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn value(handle: &Value, state: &str) -> Value {
        json!({"version":1,"state":state,"data":PathBuf::from(handle["work"].as_str().unwrap()).file_name().unwrap().to_str().unwrap(),"name":"file","parent":"/my-files/root","baseline":{"uid":"file","revision":"one"},"new_file":false,"intent":null,"error":""})
    }
    fn create(dir: &Temp, append: bool) -> Value {
        apply(Operation::Edit {
            recovery: dir.0.clone(),
            source: None,
            node: None,
            append,
        })
        .unwrap()
    }
    #[test]
    fn recovery_listing_excludes_active_files_and_rejects_invalid_metadata() {
        let dir = Temp::new();
        let handle = create(&dir, false);
        let id = handle["id"].as_u64().unwrap();
        let journal = PathBuf::from(handle["journal"].as_str().unwrap());
        apply(Operation::Journal {
            id,
            value: value(&handle, "dirty"),
        })
        .unwrap();
        assert_eq!(
            pending(&dir.0, std::slice::from_ref(&journal)).unwrap(),
            json!([])
        );
        apply(Operation::Release { id }).unwrap();
        assert_eq!(pending(&dir.0, &[]).unwrap().as_array().unwrap().len(), 1);
        let mut corrupt = value(&handle, "recovery");
        corrupt["data"] = json!("../outside.data");
        io::write_json(&journal, corrupt).unwrap();
        assert!(matches!(pending(&dir.0, &[]), Err(Error::Fs(libc::EIO, _))));
        io::write_json(&journal, json!([])).unwrap();
        assert!(matches!(pending(&dir.0, &[]), Err(Error::Fs(libc::EIO, _))));
        assert!(std::path::Path::new(handle["work"].as_str().unwrap()).exists());
    }
    #[test]
    fn owned_read_handle_outlives_borrowed_descriptor_and_closes_on_release() {
        let dir = Temp::new();
        let path = dir.0.join("object");
        io::write_json(&path, json!({"value":1})).unwrap();
        let source = io::regular(&path, libc::O_RDONLY).unwrap();
        let handle = apply(Operation::Adopt {
            fd: source.as_raw_fd(),
        })
        .unwrap();
        drop(source);
        let id = handle["id"].as_u64().unwrap();
        assert!(!apply(Operation::Read {
            id,
            offset: 0,
            size: 256
        })
        .unwrap()
        .as_str()
        .unwrap()
        .is_empty());
        assert!(matches!(
            apply(Operation::Write {
                id,
                offset: 0,
                content: "61".into()
            }),
            Err(Error::Fs(libc::EBADF, _))
        ));
        apply(Operation::Release { id }).unwrap();
        assert!(!HANDLES.lock().unwrap().contains_key(&id));
        assert!(matches!(
            apply(Operation::Read {
                id,
                offset: 0,
                size: 1
            }),
            Err(Error::Fs(libc::EBADF, _))
        ));
        assert!(path.exists());
    }
    #[test]
    fn journal_before_write_append_truncate_and_recovery_on_release() {
        let dir = Temp::new();
        let handle = create(&dir, true);
        let id = handle["id"].as_u64().unwrap();
        assert!(matches!(
            apply(Operation::Write {
                id,
                offset: 0,
                content: "61".into()
            }),
            Err(Error::Fs(libc::EIO, _))
        ));
        apply(Operation::Journal {
            id,
            value: value(&handle, "dirty"),
        })
        .unwrap();
        apply(Operation::Write {
            id,
            offset: 0,
            content: "6162".into(),
        })
        .unwrap();
        apply(Operation::Write {
            id,
            offset: 0,
            content: "63".into(),
        })
        .unwrap();
        assert_eq!(
            apply(Operation::Read {
                id,
                offset: 0,
                size: 3
            })
            .unwrap(),
            "616263"
        );
        apply(Operation::Truncate { id, size: 2 }).unwrap();
        apply(Operation::Release { id }).unwrap();
        assert_eq!(
            std::fs::read(handle["work"].as_str().unwrap()).unwrap(),
            b"ab"
        );
        assert_eq!(
            io::read_json(std::path::Path::new(handle["journal"].as_str().unwrap())).unwrap()
                ["state"],
            "recovery"
        );
    }
    #[test]
    fn journal_failure_prevents_mutation_and_keeps_bytes() {
        let dir = Temp::new();
        let handle = create(&dir, false);
        let id = handle["id"].as_u64().unwrap();
        apply(Operation::Journal {
            id,
            value: value(&handle, "dirty"),
        })
        .unwrap();
        apply(Operation::Write {
            id,
            offset: 0,
            content: "6162".into(),
        })
        .unwrap();
        let journal = PathBuf::from(handle["journal"].as_str().unwrap());
        std::fs::remove_file(&journal).unwrap();
        std::fs::create_dir(&journal).unwrap();
        assert!(apply(Operation::Write {
            id,
            offset: 0,
            content: "7a7a".into()
        })
        .is_err());
        assert!(apply(Operation::Release { id }).is_err());
        assert_eq!(
            std::fs::read(handle["work"].as_str().unwrap()).unwrap(),
            b"ab"
        );
    }
    #[test]
    fn verified_edit_copy_cleanup_and_failed_preparation() {
        let dir = Temp::new();
        let source = dir.0.join("source");
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&source, b"abc").unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
        let recovery = dir.0.join("recovery");
        let node = json!({"sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","size":3});
        let handle = apply(Operation::Edit {
            recovery: recovery.clone(),
            source: Some(source.clone()),
            node: Some(node.clone()),
            append: false,
        })
        .unwrap();
        let id = handle["id"].as_u64().unwrap();
        apply(Operation::Journal {
            id,
            value: value(&handle, "dirty"),
        })
        .unwrap();
        apply(Operation::Write {
            id,
            offset: 0,
            content: "7a".into(),
        })
        .unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), b"abc");
        apply(Operation::Settled { id }).unwrap();
        apply(Operation::Release { id }).unwrap();
        assert_eq!(std::fs::read_dir(&recovery).unwrap().count(), 0);
        let mut invalid = node;
        invalid["size"] = json!(100);
        assert!(apply(Operation::Edit {
            recovery: recovery.clone(),
            source: Some(source),
            node: Some(invalid),
            append: false
        })
        .is_err());
        assert_eq!(std::fs::read_dir(&recovery).unwrap().count(), 0);
        let handle = create(&dir, false);
        apply(Operation::Abort {
            id: handle["id"].as_u64().unwrap(),
        })
        .unwrap();
        assert!(!std::path::Path::new(handle["work"].as_str().unwrap()).exists());
    }
}
