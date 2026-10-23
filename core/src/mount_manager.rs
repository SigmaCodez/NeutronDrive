//! Owned mount lifecycle shared by the native desktop service and temporary UI binding.
use crate::{
    fuse,
    host::Services,
    ondemand_fs::{self, Action, Operation as Fs},
    ondemand_handles,
    ondemand_io::{fail, Error, Result},
    process::Failure,
    storage,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Availability,
    Create {
        state_dir: PathBuf,
    },
    Prepare {
        id: u64,
        mountpoint: PathBuf,
        remote_root: String,
        writable: bool,
        offline: bool,
    },
    Run {
        id: u64,
    },
    Wait {
        id: u64,
    },
    Unmount {
        id: u64,
    },
    Status {
        id: u64,
    },
    Abort {
        id: u64,
    },
    Forget {
        id: u64,
    },
    Pin {
        id: u64,
        path: String,
        pinned: bool,
    },
    Evict {
        id: u64,
        path: String,
    },
    Refresh {
        id: u64,
    },
    Invalidate {
        id: u64,
        root_uid: Option<String>,
        kind: String,
    },
}
#[derive(PartialEq, Eq, Clone, Copy)]
enum Phase {
    Stopped,
    Prepared,
    Running,
    Mounted,
}
struct State {
    phase: Phase,
    fs: Option<u64>,
    point: PathBuf,
    cache: PathBuf,
    remote_root: String,
    status: Value,
    recovery: Value,
    error: String,
    lease: Option<Lease>,
    directory: Option<File>,
    mount_id: Option<u64>,
}
struct Manager {
    root: PathBuf,
    state: Mutex<State>,
    changed: Condvar,
    lifecycle: Mutex<()>,
    inspection: Mutex<()>,
    cancel: AtomicBool,
    forgotten: AtomicBool,
}
static MANAGERS: LazyLock<Mutex<BTreeMap<u64, Arc<Manager>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static NEXT: AtomicU64 = AtomicU64::new(1);
static ACTIVE: AtomicBool = AtomicBool::new(false);
struct Lease;
impl Lease {
    fn acquire() -> Result<Self> {
        ACTIVE
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| {
                fail(
                    libc::EBUSY,
                    "Only one native FUSE mount per desktop-service process is supported",
                )
            })?;
        Ok(Self)
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::SeqCst);
    }
}
struct Local;
impl Services for Local {
    fn request(&mut self, _: &str, _: Value) -> std::result::Result<Value, Failure> {
        Err("Unexpected remote request in local mount operation".into())
    }
    fn poll(&mut self) -> std::result::Result<(), Failure> {
        Ok(())
    }
}
fn envelope(v: Result<Value>) -> Value {
    match v {
        Ok(v) => json!({"value":v}),
        Err(Error::Fs(errno, message)) => json!({"errno":errno,"message":message}),
        Err(Error::Backend(e)) => json!({"backend_error":e.message,"cancelled":e.cancelled}),
    }
}
fn unwrap(v: Value) -> Result<Value> {
    if let Some(errno) = v["errno"].as_i64() {
        return Err(fail(
            errno as i32,
            v["message"].as_str().unwrap_or("Mount operation failed"),
        ));
    }
    if let Some(msg) = v["backend_error"].as_str() {
        return Err(Error::Backend(Failure {
            message: msg.into(),
            cancelled: v["cancelled"] == true,
        }));
    }
    Ok(v["value"].clone())
}
fn home(user: Option<&str>) -> Result<PathBuf> {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    if user.is_none() {
        if let Some(home) = std::env::var_os("HOME") {
            return Ok(home.into());
        }
    }
    let user = user
        .map(CString::new)
        .transpose()
        .map_err(|_| fail(libc::EINVAL, "Invalid home-directory user"))?;
    let mut size = 16384;
    loop {
        let mut buffer = vec![0u8; size];
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let status = unsafe {
            match &user {
                Some(user) => libc::getpwnam_r(
                    user.as_ptr(),
                    entry.as_mut_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut result,
                ),
                None => libc::getpwuid_r(
                    libc::getuid(),
                    entry.as_mut_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut result,
                ),
            }
        };
        if status == libc::ERANGE && size < 1024 * 1024 {
            size *= 2;
            continue;
        }
        if status != 0 || result.is_null() {
            return Err(fail(libc::EINVAL, "Home directory unavailable"));
        }
        let entry = unsafe { entry.assume_init() };
        if entry.pw_dir.is_null() {
            return Err(fail(libc::EINVAL, "Home directory unavailable"));
        }
        let bytes = unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes();
        return Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)));
    }
}
pub(crate) fn current_user_home() -> std::result::Result<PathBuf, Failure> {
    home(None).map_err(|_| Failure::from("Das Benutzerverzeichnis ist nicht verfügbar."))
}
fn absolute(path: &Path) -> Result<PathBuf> {
    let first = path
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str());
    let expanded = if let Some(first) = first.filter(|s| s.starts_with('~')) {
        let user = &first[1..];
        home(if user.is_empty() { None } else { Some(user) })?
            .join(path.strip_prefix(first).unwrap())
    } else {
        path.to_owned()
    };
    let path = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()?.join(expanded)
    };
    let mut result = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            _ => result.push(c.as_os_str()),
        }
    }
    Ok(result)
}
fn helper() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| p.join("fusermount3"))
        .find(|p| {
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}
fn availability() -> Value {
    let reason = if std::env::var_os("FLATPAK_ID").is_some() {
        "Flatpak requires the separately installed native NeutronDrive desktop service for FUSE mounts."
    } else if std::env::var_os("SNAP").is_some() {
        "On-demand FUSE mounts are unavailable inside strict Snap confinement. Install and run the native NeutronDrive DEB/RPM package for this feature."
    } else if !Path::new("/dev/fuse").exists() {
        "The FUSE device /dev/fuse is unavailable in this environment."
    } else if helper().is_none() {
        "fusermount3 is missing. Install the fuse3 package."
    } else {
        ""
    };
    json!([reason.is_empty(), reason])
}
fn mount_identity(point: &Path) -> Result<Option<u64>> {
    let text = std::fs::read_to_string("/proc/self/mountinfo")?;
    let mut found = None;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let id = fields.next();
        let path = fields.nth(3);
        if let (Some(id), Some(path)) = (id, path) {
            let path = path
                .replace("\\040", " ")
                .replace("\\011", "\t")
                .replace("\\012", "\n")
                .replace("\\134", "\\");
            if Path::new(&path) == point {
                found = Some(
                    id.parse()
                        .map_err(|_| fail(libc::EIO, "Invalid mount identity"))?,
                );
            }
        }
    }
    Ok(found)
}
fn validate(point: &Path, root: &Path) -> Result<File> {
    for part in point.ancestors() {
        if std::fs::symlink_metadata(part).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(fail(
                libc::ELOOP,
                "Mountpoint must not contain symbolic links",
            ));
        }
    }
    if point.starts_with(root) || root.starts_with(point) {
        return Err(fail(
            libc::EINVAL,
            "Mountpoint and cache/state directories must not overlap",
        ));
    }
    let directory =
        storage::directory(point, true, false).map_err(|e| fail(libc::EACCES, &e.message))?;
    let meta = directory.metadata()?;
    if meta.uid() != unsafe { libc::getuid() }
        || mount_identity(point)?.is_some()
        || std::fs::read_dir(point)?.next().is_some()
    {
        return Err(fail(
            libc::EBUSY,
            "Mountpoint must be an empty, unmounted directory owned by you",
        ));
    }
    Ok(directory)
}
impl Manager {
    fn alive(&self) -> Result<()> {
        if self.forgotten.load(Ordering::SeqCst) {
            return Err(fail(libc::EBADF, "Mount manager was released"));
        }
        Ok(())
    }

    fn new(root: PathBuf) -> Result<Self> {
        Ok(Self {
            root: absolute(&root)?,
            state: Mutex::new(State {
                phase: Phase::Stopped,
                fs: None,
                point: PathBuf::new(),
                cache: PathBuf::new(),
                remote_root: String::new(),
                status: json!({}),
                recovery: json!([]),
                error: String::new(),
                lease: None,
                directory: None,
                mount_id: None,
            }),
            changed: Condvar::new(),
            lifecycle: Mutex::new(()),
            inspection: Mutex::new(()),
            cancel: AtomicBool::new(false),
            forgotten: AtomicBool::new(false),
        })
    }
    fn fs_call(&self, operation: Fs, host: &mut impl Services) -> Result<Value> {
        let v = ondemand_fs::run(operation, host)?;
        if v["status"].is_object() {
            self.state.lock().unwrap().status = v["status"].clone();
        }
        unwrap(v)
    }
    fn prepare(
        &self,
        point: PathBuf,
        remote_root: String,
        writable: bool,
        offline: bool,
        host: &mut impl Services,
    ) -> Result<Value> {
        let _operation = self.lifecycle.lock().unwrap();
        self.alive()?;
        let _inspection = self.inspection.lock().unwrap();
        if self.state.lock().unwrap().phase != Phase::Stopped {
            return Err(fail(libc::EBUSY, "This manager already has a mount"));
        }
        let available = availability();
        if available[0] != true {
            return Err(fail(libc::ENOSYS, available[1].as_str().unwrap()));
        }
        let point = absolute(&point)?;
        let directory = validate(&point, &self.root)?;
        let lease = Lease::acquire()?;
        let key = format!(
            "{:x}",
            Sha256::digest(format!("{}\0{}", point.to_string_lossy(), remote_root).as_bytes())
        );
        let cache = self.root.join("ondemand").join(key);
        let config=serde_json::from_value(json!({"cache":cache,"remote_root":remote_root,"writable":writable,"offline":offline,"metadata_ttl":15.0})).map_err(|_|fail(libc::EINVAL,"Invalid mount configuration"))?;
        let fs = self
            .fs_call(Fs::Create { config }, host)?
            .as_u64()
            .ok_or_else(|| fail(libc::EIO, "Invalid native mount identity"))?;
        let mut s = self.state.lock().unwrap();
        s.phase = Phase::Prepared;
        s.fs = Some(fs);
        s.point = point;
        s.cache = cache;
        s.remote_root = remote_root;
        s.lease = Some(lease);
        s.directory = Some(directory);
        s.error.clear();
        s.recovery = json!([]);
        s.mount_id = None;
        self.cancel.store(false, Ordering::SeqCst);
        Ok(Value::Null)
    }
    fn status(&self) -> Result<Value> {
        self.alive()?;
        if let Ok(_inspection) = self.inspection.try_lock() {
            let (fs, stopped, cache) = {
                let s = self.state.lock().unwrap();
                (s.fs, s.phase == Phase::Stopped, s.cache.clone())
            };
            if let Some(id) = fs {
                let v = if stopped {
                    ondemand_handles::run(ondemand_handles::Operation::Recovery {
                        directory: cache.join("recovery"),
                        active: vec![],
                    })?
                } else {
                    ondemand_fs::run(Fs::TryRecovery { id }, &mut Local)?
                };
                if v["busy"] != true {
                    let mut s = self.state.lock().unwrap();
                    if v["status"].is_object() {
                        s.status = v["status"].clone();
                    }
                    s.recovery = unwrap(v)?;
                }
            }
        }
        let s = self.state.lock().unwrap();
        Ok(
            json!({"status":if s.phase==Phase::Mounted{"mounted"}else{"stopped"},"mounted":s.phase==Phase::Mounted,"mountpoint":s.point,"error":if s.error.is_empty(){s.status["last_error"].as_str().unwrap_or("")}else{&s.error},"read_only":s.status["writable"]!=true||s.status["account_blocked"]==true,"cache_dir":s.cache,"remote_root":s.remote_root,"offline":s.status["offline"]==true,"recovery":s.recovery}),
        )
    }
    fn finish(&self, result: std::result::Result<Value, Failure>) {
        let _inspection = self.inspection.lock().unwrap();
        let id = self.state.lock().unwrap().fs;
        if let Some(id) = id {
            let close = self.fs_call(Fs::Close { id }, &mut Local);
            if let Err(e) = close {
                let mut s = self.state.lock().unwrap();
                s.error = match e {
                    Error::Fs(_, m) => m,
                    Error::Backend(e) => e.message,
                };
            }
        }
        let mut s = self.state.lock().unwrap();
        if let Err(e) = result {
            s.error = e.message;
        }
        s.phase = Phase::Stopped;
        s.lease.take();
        s.directory.take();
        s.mount_id = None;
        self.changed.notify_all();
    }
    fn run(
        &self,
        executable: Option<String>,
        callback: &mut impl FnMut(Value) -> i32,
    ) -> Result<Value> {
        let config = {
            let mut s = self.state.lock().unwrap();
            self.alive()?;
            if s.phase != Phase::Prepared {
                return Err(fail(libc::EINVAL, "Mount is not prepared"));
            }
            let prior = s.directory.as_ref().unwrap().metadata()?;
            let current = validate(&s.point, &self.root)?.metadata()?;
            if prior.dev() != current.dev() || prior.ino() != current.ino() {
                return Err(fail(libc::ESTALE, "Mountpoint changed before startup"));
            }
            s.phase = Phase::Running;
            fuse::Config {
                id: s.fs.unwrap(),
                mountpoint: s.point.clone(),
                cache: s.cache.clone(),
                writable: s.status["writable"] == true,
            }
        };
        let result = fuse::run(config, executable, &mut |v| {
            match v["kind"].as_str() {
                Some("sync_started") => {
                    let mut s = self.state.lock().unwrap();
                    match mount_identity(&s.point) {
                        Ok(Some(id)) => {
                            s.mount_id = Some(id);
                            s.phase = Phase::Mounted;
                            self.changed.notify_all();
                        }
                        _ => {
                            self.cancel.store(true, Ordering::SeqCst);
                        }
                    }
                }
                Some("fuse_status") => {
                    self.state.lock().unwrap().status = v["status"].clone();
                }
                _ => {
                    if callback(v) != 0 {
                        self.cancel.store(true, Ordering::SeqCst);
                    }
                }
            }
            i32::from(self.cancel.load(Ordering::SeqCst))
        });
        let cancelled = result.as_ref().err().is_some_and(|error| error.cancelled);
        self.finish(result);
        let mut status = self.status()?;
        status["cancelled"] = json!(cancelled);
        Ok(status)
    }
    fn wait(&self, stopped: bool) -> Result<()> {
        self.alive()?;
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut s = self.state.lock().unwrap();
        loop {
            if stopped && s.phase == Phase::Stopped {
                return Ok(());
            }
            if !stopped {
                if s.phase == Phase::Mounted {
                    return Ok(());
                }
                if s.phase == Phase::Stopped {
                    return Err(fail(
                        libc::EIO,
                        if s.error.is_empty() {
                            "FUSE mount failed"
                        } else {
                            &s.error
                        },
                    ));
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                if !stopped {
                    self.cancel.store(true, Ordering::SeqCst);
                }
                return Err(fail(
                    if stopped {
                        libc::EBUSY
                    } else {
                        libc::ETIMEDOUT
                    },
                    "FUSE is still finishing active work",
                ));
            }
            s = self.changed.wait_timeout(s, remaining).unwrap().0;
        }
    }
    fn abort(&self) -> Result<Value> {
        let _op = self.lifecycle.lock().unwrap();
        self.alive()?;
        let phase = self.state.lock().unwrap().phase;
        if phase == Phase::Prepared {
            self.finish(Err("FUSE worker did not start".into()));
        } else if phase != Phase::Stopped {
            self.cancel.store(true, Ordering::SeqCst);
            self.wait(true)?;
        }
        self.status()
    }
    fn unmount(&self) -> Result<Value> {
        let _op = self.lifecycle.lock().unwrap();
        self.alive()?;
        let (phase, point, identity) = {
            let s = self.state.lock().unwrap();
            (s.phase, s.point.clone(), s.mount_id)
        };
        if phase == Phase::Stopped {
            return self.status();
        }
        if phase != Phase::Mounted {
            return Err(fail(libc::EBUSY, "FUSE mount is still starting"));
        }
        // Never apply a stale path-based unmount to a replacement filesystem.
        if mount_identity(&point)? == identity {
            let helper = helper().ok_or_else(|| fail(libc::ENOENT, "fusermount3 is missing"))?;
            let mut child = Command::new(helper)
                .arg("-u")
                .arg("--")
                .arg(&point)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()?;
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                if let Some(status) = child.try_wait()? {
                    if !status.success() {
                        return Err(fail(
                            libc::EBUSY,
                            "Mount is busy; close files and terminals using it before unmounting",
                        ));
                    }
                    break;
                }
                if Instant::now() >= deadline {
                    unsafe {
                        libc::kill(-(child.id() as i32), libc::SIGKILL);
                    }
                    let _ = child.wait();
                    return Err(fail(libc::ETIMEDOUT, "FUSE unmount helper timed out"));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        self.wait(true)?;
        self.status()
    }
    fn active_fs(&self) -> Result<u64> {
        let s = self.state.lock().unwrap();
        if s.phase != Phase::Mounted {
            return Err(fail(libc::ENOTCONN, "No active on-demand mount"));
        }
        s.fs.ok_or_else(|| fail(libc::EBADF, "Unknown native mount"))
    }
    fn call(&self, id: u64, operation: Action, host: &mut impl Services) -> Result<Value> {
        self.fs_call(Fs::Call { id, operation }, host)
    }
    fn resolve(&self, id: u64, path: &str, host: &mut impl Services) -> Result<u64> {
        let path = Path::new(path);
        if path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
            return Err(fail(libc::EINVAL, "Use a path relative to the mount"));
        }
        let mut inode = 1;
        for name in path.components() {
            if name == Component::CurDir {
                continue;
            }
            let name = name
                .as_os_str()
                .to_str()
                .ok_or_else(|| fail(libc::EILSEQ, "Invalid path"))?
                .to_owned();
            let entry = self.call(
                id,
                Action::Lookup {
                    parent: inode,
                    name,
                },
                host,
            )?;
            inode = entry["inode"]
                .as_u64()
                .ok_or_else(|| fail(libc::EIO, "Invalid inode"))?;
        }
        Ok(inode)
    }
    fn action(&self, action: Operation, host: &mut impl Services) -> Result<Value> {
        match action {
            Operation::Prepare {
                mountpoint,
                remote_root,
                writable,
                offline,
                ..
            } => return self.prepare(mountpoint, remote_root, writable, offline, host),
            Operation::Wait { .. } => {
                self.wait(false)?;
                return self.status();
            }
            Operation::Status { .. } => return self.status(),
            Operation::Unmount { .. } => return self.unmount(),
            Operation::Abort { .. } => return self.abort(),
            _ => {}
        }
        let _op = self.lifecycle.lock().unwrap();
        self.alive()?;
        if let Operation::Invalidate { root_uid, kind, .. } = action {
            let (id, status) = {
                let s = self.state.lock().unwrap();
                if s.phase == Phase::Stopped {
                    return Ok(Value::Null);
                }
                (s.fs.unwrap(), s.status.clone())
            };
            let blocked = root_uid
                .as_deref()
                .is_some_and(|r| !r.is_empty() && status["root_uid"] != r)
                || kind == "tree_remove";
            return self.call(
                id,
                if blocked {
                    Action::BlockAccount
                } else {
                    Action::Invalidate
                },
                host,
            );
        }
        let fs = self.active_fs()?;
        match action {
            Operation::Pin { path, pinned, .. } => {
                let inode = self.resolve(fs, &path, host)?;
                self.call(fs, Action::Pin { inode, pinned }, host)?;
            }
            Operation::Evict { path, .. } => {
                let inode = self.resolve(fs, &path, host)?;
                self.call(
                    fs,
                    Action::Pin {
                        inode,
                        pinned: false,
                    },
                    host,
                )?;
                self.call(fs, Action::Evict { inode }, host)?;
            }
            Operation::Refresh { .. } => {
                self.call(fs, Action::Invalidate, host)?;
                self.call(fs, Action::Refresh { inode: 1 }, host)?;
            }
            _ => return Err(fail(libc::EINVAL, "Invalid mount-manager operation")),
        }
        self.status()
    }
}
fn manager(id: u64) -> Result<Arc<Manager>> {
    MANAGERS
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| fail(libc::EBADF, "Unknown mount manager"))
}
pub fn run(
    operation: Operation,
    executable: Option<String>,
    callback: &mut impl FnMut(Value) -> i32,
) -> std::result::Result<Value, Failure> {
    let result = (|| -> Result<Value> {
        match operation {
            Operation::Availability => Ok(availability()),
            Operation::Create { state_dir } => {
                let value = Arc::new(Manager::new(state_dir)?);
                let id = NEXT.fetch_add(1, Ordering::Relaxed);
                MANAGERS.lock().unwrap().insert(id, value);
                Ok(json!(id))
            }
            Operation::Run { id } => {
                let m = manager(id)?;
                let result = m.run(executable, callback);
                if result.is_err() && m.state.lock().unwrap().phase == Phase::Prepared {
                    m.finish(Err("FUSE startup validation failed".into()));
                }
                result
            }
            Operation::Forget { id } => {
                let mut registry = MANAGERS.lock().unwrap();
                if let Some(m) = registry.get(&id).cloned() {
                    let _operation = m
                        .lifecycle
                        .try_lock()
                        .map_err(|_| fail(libc::EBUSY, "Mount manager is busy"))?;
                    if m.state.lock().unwrap().phase != Phase::Stopped {
                        return Err(fail(libc::EBUSY, "Cannot release an active mount manager"));
                    }
                    m.forgotten.store(true, Ordering::SeqCst);
                    registry.remove(&id);
                }
                Ok(Value::Null)
            }
            operation => {
                let id = match &operation {
                    Operation::Prepare { id, .. }
                    | Operation::Wait { id }
                    | Operation::Unmount { id }
                    | Operation::Status { id }
                    | Operation::Abort { id }
                    | Operation::Pin { id, .. }
                    | Operation::Evict { id, .. }
                    | Operation::Refresh { id }
                    | Operation::Invalidate { id, .. } => *id,
                    _ => unreachable!(),
                };
                let m = manager(id)?;
                match executable {
                    Some(executable) => m.action(
                        operation,
                        &mut crate::sync_remote::Sdk {
                            executable,
                            callback,
                        },
                    ),
                    None => m.action(operation, &mut crate::host::Callback(callback)),
                }
            }
        }
    })();
    Ok(envelope(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "nd-manager-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            storage::directory(&root, true, true).unwrap();
            Self(root)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn mountpoint_validation_rejects_symlinks_overlap_and_nonempty_directories() {
        let t = Temp::new();
        let point = t.0.join("point");
        let state = t.0.join("state");
        let original = validate(&point, &state).unwrap();
        assert_eq!(
            original.metadata().unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(matches!(
            validate(&state.join("nested"), &state),
            Err(Error::Fs(libc::EINVAL, _))
        ));
        assert!(matches!(
            validate(&t.0, &state),
            Err(Error::Fs(libc::EINVAL, _))
        ));
        let link = t.0.join("link");
        std::os::unix::fs::symlink(&point, &link).unwrap();
        assert!(matches!(
            validate(&link.join("child"), &state),
            Err(Error::Fs(libc::ELOOP, _))
        ));
        std::fs::write(point.join("content"), b"retain").unwrap();
        assert!(matches!(
            validate(&point, &state),
            Err(Error::Fs(libc::EBUSY, _))
        ));
        assert_eq!(std::fs::read(point.join("content")).unwrap(), b"retain");
    }
    #[test]
    fn status_remains_available_while_inspection_is_busy_and_paths_cannot_escape() {
        let t = Temp::new();
        let m = Manager::new(t.0.join("state")).unwrap();
        let _inspection = m.inspection.lock().unwrap();
        assert_eq!(m.status().unwrap()["mounted"], false);
        for path in ["/absolute", "../escape", "child/../../escape"] {
            assert!(matches!(
                m.resolve(999, path, &mut Local),
                Err(Error::Fs(libc::EINVAL, _))
            ));
        }
        assert_eq!(m.resolve(999, ".", &mut Local).unwrap(), 1);
        assert!(matches!(m.active_fs(), Err(Error::Fs(libc::ENOTCONN, _))));
    }
    #[test]
    fn home_expansion_rejects_unknown_users_instead_of_creating_literal_paths() {
        assert!(absolute(Path::new("~")).unwrap().is_absolute());
        assert!(matches!(
            absolute(Path::new("~neutrondrive-nonexistent-home-user-57f29/child")),
            Err(Error::Fs(libc::EINVAL, _))
        ));
        assert_eq!(
            absolute(Path::new("/tmp/one/../two")).unwrap(),
            Path::new("/tmp/two")
        );
    }
    #[test]
    fn a_retained_reference_cannot_start_after_manager_release() {
        let t = Temp::new();
        let m = Arc::new(Manager::new(t.0.join("state")).unwrap());
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        MANAGERS.lock().unwrap().insert(id, m.clone());
        assert!(unwrap(run(Operation::Forget { id }, None, &mut |_| 0).unwrap()).is_ok());
        assert!(matches!(
            m.prepare(
                t.0.join("mount"),
                "/my-files".into(),
                false,
                false,
                &mut Local
            ),
            Err(Error::Fs(libc::EBADF, _))
        ));
        assert!(matches!(m.status(), Err(Error::Fs(libc::EBADF, _))));
        assert!(matches!(
            m.action(
                Operation::Invalidate {
                    id,
                    root_uid: None,
                    kind: "ready".into()
                },
                &mut Local
            ),
            Err(Error::Fs(libc::EBADF, _))
        ));
        assert!(!t.0.join("mount").exists());
    }
    #[test]
    fn session_reservation_is_exclusive_and_released_by_ownership() {
        let lease = Lease::acquire().unwrap();
        assert!(matches!(Lease::acquire(), Err(Error::Fs(libc::EBUSY, _))));
        drop(lease);
        let _again = Lease::acquire().unwrap();
    }
}
