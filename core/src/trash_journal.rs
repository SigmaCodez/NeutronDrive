//! Owned locks and durable receipt storage for local trash transactions.
use crate::{filesystem, process::Failure, storage};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Failure>;
const LIMIT: usize = 64 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(1);
static LEASES: LazyLock<Mutex<BTreeMap<u64, Arc<Lease>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
struct Lease {
    state: PathBuf,
    directory: File,
    entries: File,
    lock: File,
    operation: Mutex<()>,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Lock {
        state: String,
        timeout: f64,
    },
    Unlock {
        id: u64,
    },
    Read {
        id: u64,
        name: String,
    },
    Names {
        id: u64,
    },
    Save {
        id: u64,
        record: Value,
        initial: bool,
    },
}
fn io(e: std::io::Error) -> Failure {
    e.to_string().into()
}
fn identity(file: &File) -> Result<(u64, u64)> {
    let m = file.metadata().map_err(io)?;
    Ok((m.dev(), m.ino()))
}
fn safe_file(file: &File) -> Result<()> {
    let m = file.metadata().map_err(io)?;
    if !m.is_file()
        || m.uid() != unsafe { libc::getuid() }
        || m.nlink() != 1
        || m.mode() & 0o077 != 0
        || m.len() > LIMIT as u64
    {
        return Err("Unsafe trash journal".into());
    }
    Ok(())
}
pub(crate) fn name(value: &str) -> Result<CString> {
    if !value.is_ascii()
        || value.len() != 41
        || !value.ends_with(".json")
        || !value[..36].bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_digit() || (b'a'..=b'f').contains(&c)
            }
        })
    {
        return Err("Invalid trash receipt identity".into());
    }
    CString::new(value).map_err(|_| "Invalid trash receipt identity".into())
}
impl Lease {
    fn validate(&self) -> Result<()> {
        if identity(&storage::directory(&self.state, false, true)?)? != identity(&self.directory)?
            || identity(&storage::directory(
                &self.state.join("entries"),
                false,
                true,
            )?)? != identity(&self.entries)?
        {
            return Err("Trash journal directory was replaced".into());
        }
        let lock = filesystem::open(
            self.directory.as_raw_fd(),
            "lock",
            libc::O_RDONLY | libc::O_NONBLOCK,
        )?;
        safe_file(&lock)?;
        if identity(&lock)? != identity(&self.lock)? {
            return Err("Trash lock was replaced".into());
        }
        Ok(())
    }
    fn read(&self, key: &str) -> Result<Value> {
        name(key)?;
        self.validate()?;
        let file = filesystem::open(
            self.entries.as_raw_fd(),
            key,
            libc::O_RDONLY | libc::O_NONBLOCK,
        )?;
        safe_file(&file)?;
        let mut bytes = Vec::new();
        file.take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(io)?;
        if bytes.len() > LIMIT {
            return Err("Trash journal is too large".into());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "Unsafe trash journal")?;
        if value["id"].as_str() != Some(&key[..36]) {
            return Err("Trash journal identity mismatch".into());
        }
        self.validate()?;
        Ok(value)
    }
    fn save(&self, record: Value, initial: bool) -> Result<Value> {
        let target = name(&format!(
            "{}.json",
            record["id"]
                .as_str()
                .ok_or("Invalid trash receipt identity")?
        ))?;
        let bytes = serde_json::to_vec(&record).map_err(|_| "Invalid trash journal")?;
        if bytes.len() > LIMIT {
            return Err("Trash journal is too large".into());
        }
        self.validate()?;
        let mut random = [0u8; 16];
        File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(io)?;
        let temporary = CString::new(format!(
            ".{}.tmp",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ))
        .unwrap();
        let destination = if initial { &target } else { &temporary };
        let fd = unsafe {
            libc::openat(
                self.entries.as_raw_fd(),
                destination.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        };
        if fd < 0 {
            let e = std::io::Error::last_os_error();
            if initial && e.kind() == std::io::ErrorKind::AlreadyExists {
                return Ok(json!(false));
            }
            return Err(io(e));
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        struct Cleanup<'a> {
            directory: &'a File,
            name: &'a CString,
            active: bool,
        }
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                if self.active {
                    unsafe { libc::unlinkat(self.directory.as_raw_fd(), self.name.as_ptr(), 0) };
                }
            }
        }
        let _cleanup = Cleanup {
            directory: &self.entries,
            name: &temporary,
            active: !initial,
        };
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(io)?;
        if !initial
            && unsafe {
                libc::renameat(
                    self.entries.as_raw_fd(),
                    temporary.as_ptr(),
                    self.entries.as_raw_fd(),
                    target.as_ptr(),
                )
            } < 0
        {
            return Err(io(std::io::Error::last_os_error()));
        }
        self.entries.sync_all().map_err(io)?;
        self.validate()?;
        Ok(json!(true))
    }
}
fn acquire(state: &Path, timeout: f64, callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    if !timeout.is_finite() || !(0.0..=300.0).contains(&timeout) {
        return Err("Invalid trash lock timeout".into());
    }
    let directory = storage::directory(state, false, true)?;
    let entries = storage::directory(&state.join("entries"), false, true)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            c"lock".as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    let lock = unsafe { File::from_raw_fd(fd) };
    safe_file(&lock)?;
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    loop {
        filesystem::poll(callback)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            break;
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock
            || Instant::now() >= deadline
        {
            return Err("Another local trash operation is busy; retry later".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let lease = Arc::new(Lease {
        state: state.into(),
        directory,
        entries,
        lock,
        operation: Mutex::new(()),
    });
    lease.validate()?;
    let mut leases = LEASES
        .lock()
        .map_err(|_| "Trash lock registry unavailable")?;
    if leases.len() >= 128 {
        return Err("Too many open trash transactions".into());
    }
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    leases.insert(id, lease);
    Ok(json!(id))
}
pub fn run(operation: Operation, callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    match operation {
        Operation::Lock { state, timeout } => acquire(Path::new(&state), timeout, callback),
        Operation::Unlock { id } => {
            LEASES
                .lock()
                .map_err(|_| "Trash lock registry unavailable")?
                .remove(&id)
                .ok_or("Unknown trash transaction")?;
            Ok(Value::Null)
        }
        operation => {
            let id = match &operation {
                Operation::Read { id, .. }
                | Operation::Names { id }
                | Operation::Save { id, .. } => *id,
                _ => unreachable!(),
            };
            let lease = LEASES
                .lock()
                .map_err(|_| "Trash lock registry unavailable")?
                .get(&id)
                .cloned()
                .ok_or("Unknown trash transaction")?;
            let _guard = lease
                .operation
                .lock()
                .map_err(|_| "Trash transaction unavailable")?;
            lease.validate()?;
            match operation {
                Operation::Read { name, .. } => lease.read(&name),
                Operation::Names { .. } => Ok(json!(filesystem::names(lease.entries.as_raw_fd())?)),
                Operation::Save {
                    record, initial, ..
                } => lease.save(record, initial),
                _ => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "12345678-1234-1234-1234-123456789abc";
    struct Tree(PathBuf);
    impl Tree {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "nd-journal-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            storage::directory(&p.join("entries"), true, true).unwrap();
            Self(p)
        }
        fn lock(&self) -> u64 {
            acquire(&self.0, 0.0, &mut |_| 0).unwrap().as_u64().unwrap()
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn unlock(id: u64) {
        run(Operation::Unlock { id }, &mut |_| 0).unwrap();
    }
    #[test]
    fn receipts_are_exclusive_durable_and_reopenable() {
        let tree = Tree::new();
        let id = tree.lock();
        let record = json!({"id":ID,"status":"prepared"});
        assert_eq!(
            run(
                Operation::Save {
                    id,
                    record: record.clone(),
                    initial: true
                },
                &mut |_| 0
            )
            .unwrap(),
            true
        );
        assert_eq!(
            run(
                Operation::Save {
                    id,
                    record: json!({"id":ID,"status":"overwrite"}),
                    initial: true
                },
                &mut |_| 0
            )
            .unwrap(),
            false
        );
        unlock(id);
        let id = tree.lock();
        assert_eq!(
            run(
                Operation::Read {
                    id,
                    name: format!("{ID}.json")
                },
                &mut |_| 0
            )
            .unwrap(),
            record
        );
        let updated = json!({"id":ID,"status":"trashed"});
        run(
            Operation::Save {
                id,
                record: updated.clone(),
                initial: false,
            },
            &mut |_| 0,
        )
        .unwrap();
        assert_eq!(
            run(
                Operation::Read {
                    id,
                    name: format!("{ID}.json")
                },
                &mut |_| 0
            )
            .unwrap(),
            updated
        );
        unlock(id);
        assert!(run(Operation::Names { id }, &mut |_| 0).is_err());
    }
    #[test]
    fn cancellation_and_contention_do_not_leak_locks() {
        let tree = Tree::new();
        let id = tree.lock();
        assert!(acquire(&tree.0, 0.0, &mut |_| 0).is_err());
        assert!(acquire(&tree.0, 1.0, &mut |_| 1).unwrap_err().cancelled);
        unlock(id);
        let id = tree.lock();
        unlock(id);
    }
    #[test]
    fn replaced_entries_directory_blocks_commit() {
        let tree = Tree::new();
        let id = tree.lock();
        std::fs::rename(tree.0.join("entries"), tree.0.join("old")).unwrap();
        storage::directory(&tree.0.join("entries"), true, true).unwrap();
        assert!(run(
            Operation::Save {
                id,
                record: json!({"id":ID}),
                initial: true
            },
            &mut |_| 0
        )
        .is_err());
        assert_eq!(
            std::fs::read_dir(tree.0.join("entries")).unwrap().count(),
            0
        );
        unlock(id);
    }
    #[test]
    fn unsafe_names_and_hardlinked_receipts_are_rejected() {
        let tree = Tree::new();
        let id = tree.lock();
        for value in ["../escape.json", "éééééééééééééééééé.json"] {
            assert!(run(
                Operation::Read {
                    id,
                    name: value.into()
                },
                &mut |_| 0
            )
            .is_err());
        }
        run(
            Operation::Save {
                id,
                record: json!({"id":ID}),
                initial: true,
            },
            &mut |_| 0,
        )
        .unwrap();
        std::fs::hard_link(
            tree.0.join("entries").join(format!("{ID}.json")),
            tree.0.join("alias"),
        )
        .unwrap();
        assert!(run(
            Operation::Read {
                id,
                name: format!("{ID}.json")
            },
            &mut |_| 0
        )
        .is_err());
        unlock(id);
    }
}
