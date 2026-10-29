//! Native Linux kernel adapter. The service pump owns callbacks on the calling
//! thread; no foreign callback pointer is retained by a filesystem worker.
use crate::{
    ondemand_fs::{self, Action, Operation},
    process::Failure,
};
use fuser::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    ffi::{CString, OsStr},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    sync::{mpsc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
type Result<T> = std::result::Result<T, Errno>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub(crate) id: u64,
    pub(crate) mountpoint: PathBuf,
    pub(crate) cache: PathBuf,
    pub(crate) writable: bool,
}
struct Call {
    action: Action,
    reply: mpsc::SyncSender<Result<Value>>,
}
struct Kernel {
    id: u64,
    calls: mpsc::SyncSender<Call>,
    cache: PathBuf,
    directories: Mutex<(u64, BTreeMap<u64, Vec<Value>>)>,
}
fn integer(v: &Value) -> Result<u64> {
    v.as_u64().ok_or(Errno::EIO)
}
fn name(v: &OsStr) -> Result<String> {
    v.to_str().map(str::to_owned).ok_or(Errno::EILSEQ)
}
fn offset(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| Errno::EOVERFLOW)
}
// GLib's local content-type probe uses O_NOATIME, unlike its normal input
// stream/copy operation. A failed optional probe falls back to the filename.
// Do not hydrate (or hash a multi-GB cache object) merely to draw a folder.
// Scope this to the known GIO clients: O_NOATIME alone is a valid application
// read request and must continue to work for editors, backup tools, etc.
fn content_type_probe(pid: u32, flags: i32) -> bool {
    if flags & libc::O_ACCMODE != libc::O_RDONLY
        || flags & libc::O_NOATIME == 0
        || flags & (libc::O_TRUNC | libc::O_CREAT) != 0
    {
        return false;
    }
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|path| path.file_name().map(OsStr::to_owned))
        .is_some_and(|name| {
            matches!(
                name.to_str(),
                Some("gio" | "nautilus" | "nemo" | "caja" | "Thunar" | "thunar" | "pcmanfm")
            )
        })
}
fn unpack(v: Value) -> Result<Value> {
    if let Some(e) = v["errno"].as_i64() {
        return Err(Errno::from_i32(e as i32));
    }
    if v.get("backend_error").is_some() {
        return Err(Errno::EIO);
    }
    v.get("value").cloned().ok_or(Errno::EIO)
}
fn attrs(v: Value) -> Result<FileAttr> {
    let size = integer(&v["size"])?;
    let folder = v["folder"] == true;
    let ns = v["mtime_ns"].as_i64().unwrap_or(0);
    let delta = Duration::from_nanos(ns.unsigned_abs());
    let time = if ns < 0 {
        UNIX_EPOCH.checked_sub(delta)
    } else {
        UNIX_EPOCH.checked_add(delta)
    }
    .ok_or(Errno::EOVERFLOW)?;
    Ok(FileAttr {
        ino: INodeNo(integer(&v["inode"])?),
        size,
        blocks: if v["cached"] == true {
            size.div_ceil(512)
        } else {
            0
        },
        atime: time,
        mtime: time,
        ctime: time,
        crtime: time,
        kind: if folder {
            FileType::Directory
        } else {
            FileType::RegularFile
        },
        perm: match (folder, v["writable"] == true) {
            (true, true) => 0o700,
            (true, false) => 0o500,
            (false, true) => 0o600,
            (false, false) => 0o400,
        },
        nlink: if folder { 2 } else { 1 },
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
        rdev: 0,
        blksize: 4096,
        flags: 0,
    })
}
impl Kernel {
    fn call(&self, action: Action) -> Result<Value> {
        if let Action::Unlink {
            parent,
            name,
            directory,
        } = &action
        {
            return ondemand_fs::enqueue_delete(self.id, *parent, name, *directory).map_err(|e| {
                match e {
                    crate::ondemand_io::Error::Fs(code, _) => Errno::from_i32(code),
                    _ => Errno::EIO,
                }
            });
        }
        if let Some(value) = ondemand_fs::cached(self.id, &action) {
            return Ok(value);
        }
        if ondemand_fs::absent(self.id, &action) {
            return Err(Errno::ENOENT);
        }
        let (reply, rx) = mpsc::sync_channel(1);
        let metadata = matches!(
            action,
            Action::Metadata { .. } | Action::Lookup { .. } | Action::Listdir { .. }
        );
        let call = Call { action, reply };
        if metadata {
            // A stalled SDK request must not trap Nautilus' metadata probes.
            // Only read-only requests may time out: writes must retain their
            // original acknowledgement and recovery semantics.
            self.calls.try_send(call).map_err(|error| match error {
                mpsc::TrySendError::Full(_) => Errno::EAGAIN,
                mpsc::TrySendError::Disconnected(_) => Errno::EIO,
            })?;
            rx.recv_timeout(Duration::from_secs(3))
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => Errno::EAGAIN,
                    mpsc::RecvTimeoutError::Disconnected => Errno::EIO,
                })?
        } else {
            self.calls.send(call).map_err(|_| Errno::EIO)?;
            rx.recv().map_err(|_| Errno::EIO)?
        }
    }
    fn attr(&self, ino: INodeNo) -> Result<FileAttr> {
        attrs(self.call(Action::Metadata { inode: ino.0 })?)
    }
    fn entry(&self, v: Value) -> Result<FileAttr> {
        self.attr(INodeNo(integer(&v["inode"])?))
    }
}
macro_rules! empty {
    ($reply:expr,$op:expr) => {
        match $op {
            Ok(_) => $reply.ok(),
            Err(e) => $reply.error(e),
        }
    };
}
macro_rules! entry {
    ($reply:expr,$op:expr) => {
        match $op {
            Ok(a) => $reply.entry(&Duration::ZERO, &a, Generation(0)),
            Err(e) => $reply.error(e),
        }
    };
}
impl Filesystem for Kernel {
    fn init(&mut self, _: &Request, config: &mut KernelConfig) -> std::io::Result<()> {
        // Truncate belongs to the same guarded open transaction, not a second
        // anonymous setattr while a writer already owns the inode.
        config
            .add_capabilities(InitFlags::FUSE_ATOMIC_O_TRUNC)
            .map_err(|_| std::io::Error::other("Kernel lacks atomic open/truncate"))?;
        Ok(())
    }

    fn lookup(&self, _: &Request, parent: INodeNo, n: &OsStr, reply: ReplyEntry) {
        entry!(
            reply,
            (|| self.entry(self.call(Action::Lookup {
                parent: parent.0,
                name: name(n)?
            })?))()
        );
    }
    fn getattr(&self, _: &Request, ino: INodeNo, _: Option<FileHandle>, reply: ReplyAttr) {
        match self.attr(ino) {
            Ok(a) => reply.attr(&Duration::ZERO, &a),
            Err(e) => reply.error(e),
        }
    }
    fn opendir(&self, _: &Request, ino: INodeNo, _: OpenFlags, reply: ReplyOpen) {
        let result = (|| {
            let values = self.call(Action::Listdir {
                inode: ino.0,
                refresh: true,
            })?;
            let entries = values.as_array().ok_or(Errno::EIO)?.clone();
            let mut dirs = self.directories.lock().map_err(|_| Errno::EIO)?;
            let id = dirs.0;
            dirs.0 = id.checked_add(1).ok_or(Errno::EOVERFLOW)?;
            dirs.1.insert(id, entries);
            Ok(id)
        })();
        match result {
            Ok(id) => reply.opened(FileHandle(id), FopenFlags::empty()),
            Err(e) => reply.error(e),
        }
    }
    fn readdir(
        &self,
        _: &Request,
        _: INodeNo,
        fh: FileHandle,
        off: u64,
        mut reply: ReplyDirectory,
    ) {
        let result = (|| {
            let dirs = self.directories.lock().map_err(|_| Errno::EIO)?;
            let entries = dirs.1.get(&fh.0).ok_or(Errno::EBADF)?;
            let start = usize::try_from(off).map_err(|_| Errno::EOVERFLOW)?;
            for (i, v) in entries.iter().enumerate().skip(start) {
                let ino = integer(&v["inode"])?;
                let n = v["node"]["name"].as_str().ok_or(Errno::EIO)?;
                let kind = if v["node"]["type"] == "folder" {
                    FileType::Directory
                } else {
                    FileType::RegularFile
                };
                if reply.add(INodeNo(ino), (i + 1) as u64, kind, n) {
                    break;
                }
            }
            Ok(())
        })();
        empty!(reply, result);
    }
    fn releasedir(&self, _: &Request, _: INodeNo, fh: FileHandle, _: OpenFlags, reply: ReplyEmpty) {
        empty!(
            reply,
            self.directories
                .lock()
                .map(|mut d| d.1.remove(&fh.0))
                .map_err(|_| Errno::EIO)
        );
    }
    fn open(&self, request: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        if content_type_probe(request.pid(), flags.0) {
            // EPERM would make GLib retry without O_NOATIME. EAGAIN makes it
            // keep its filename-based type without suppressing normal opens.
            reply.error(Errno::EAGAIN);
            return;
        }
        match self
            .call(Action::Open {
                inode: ino.0,
                flags: flags.0,
            })
            .and_then(|v| integer(&v))
        {
            Ok(id) => reply.opened(FileHandle(id), FopenFlags::FOPEN_DIRECT_IO),
            Err(e) => reply.error(e),
        }
    }
    fn read(
        &self,
        _: &Request,
        _: INodeNo,
        fh: FileHandle,
        off: u64,
        size: u32,
        _: OpenFlags,
        _: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let result = (|| {
            let v = self.call(Action::Read {
                handle: fh.0,
                offset: offset(off)?,
                size: size.into(),
            })?;
            let s = v.as_str().ok_or(Errno::EIO)?;
            if s.len() % 2 != 0 || !s.is_ascii() {
                return Err(Errno::EIO);
            }
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| Errno::EIO))
                .collect::<Result<Vec<_>>>()
        })();
        match result {
            Ok(data) => reply.data(&data),
            Err(e) => reply.error(e),
        }
    }
    fn write(
        &self,
        _: &Request,
        _: INodeNo,
        fh: FileHandle,
        off: u64,
        data: &[u8],
        _: WriteFlags,
        _: OpenFlags,
        _: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let result = (|| {
            let content = data
                .iter()
                .take(8 * 1024 * 1024)
                .map(|b| format!("{b:02x}"))
                .collect();
            let n = integer(&self.call(Action::Write {
                handle: fh.0,
                offset: offset(off)?,
                content,
            })?)?;
            u32::try_from(n).map_err(|_| Errno::EOVERFLOW)
        })();
        match result {
            Ok(n) => reply.written(n),
            Err(e) => reply.error(e),
        }
    }
    fn flush(&self, _: &Request, _: INodeNo, fh: FileHandle, _: LockOwner, reply: ReplyEmpty) {
        empty!(reply, self.call(Action::Flush { handle: fh.0 }));
    }
    fn fsync(&self, _: &Request, _: INodeNo, fh: FileHandle, _: bool, reply: ReplyEmpty) {
        empty!(reply, self.call(Action::Flush { handle: fh.0 }));
    }
    fn release(
        &self,
        _: &Request,
        _: INodeNo,
        fh: FileHandle,
        _: OpenFlags,
        _: Option<LockOwner>,
        _: bool,
        reply: ReplyEmpty,
    ) {
        if self.call(Action::Release { handle: fh.0 }).is_err() {
            let _ = self.call(Action::SetError {
                message: "A closed handle requires local recovery.".into(),
            });
        }
        reply.ok();
    }
    fn create(
        &self,
        _: &Request,
        parent: INodeNo,
        n: &OsStr,
        _: u32,
        _: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        let result = (|| {
            let v = self.call(Action::Create {
                parent: parent.0,
                name: name(n)?,
                flags,
            })?;
            let id = integer(&v[0])?;
            match self.entry(v[1].clone()) {
                Ok(a) => Ok((id, a)),
                Err(e) => {
                    let _ = self.call(Action::Release { handle: id });
                    Err(e)
                }
            }
        })();
        match result {
            Ok((id, a)) => reply.created(
                &Duration::ZERO,
                &a,
                Generation(0),
                FileHandle(id),
                FopenFlags::FOPEN_DIRECT_IO,
            ),
            Err(e) => reply.error(e),
        }
    }
    fn setattr(
        &self,
        _: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _: Option<SystemTime>,
        fh: Option<FileHandle>,
        _: Option<SystemTime>,
        _: Option<SystemTime>,
        _: Option<SystemTime>,
        _: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let result = (|| {
            let a = self.attr(ino)?;
            if uid.is_some_and(|u| u != a.uid) || gid.is_some_and(|g| g != a.gid) {
                return Err(Errno::EPERM);
            }
            if mode.is_some_and(|m| m & 0o7777 != u32::from(a.perm)) {
                return Err(Errno::ENOTSUP);
            }
            if let Some(size) = size {
                self.call(Action::Truncate {
                    inode: ino.0,
                    size: offset(size)?,
                    handle: fh.map(|f| f.0),
                })?;
            }
            if atime.is_some() || mtime.is_some() {
                return Err(Errno::ENOTSUP);
            }
            self.attr(ino)
        })();
        match result {
            Ok(a) => reply.attr(&Duration::ZERO, &a),
            Err(e) => reply.error(e),
        }
    }
    fn mkdir(&self, _: &Request, parent: INodeNo, n: &OsStr, _: u32, _: u32, reply: ReplyEntry) {
        entry!(
            reply,
            (|| self.entry(self.call(Action::Mkdir {
                parent: parent.0,
                name: name(n)?
            })?))()
        );
    }
    fn unlink(&self, _: &Request, parent: INodeNo, n: &OsStr, reply: ReplyEmpty) {
        empty!(
            reply,
            (|| self.call(Action::Unlink {
                parent: parent.0,
                name: name(n)?,
                directory: false
            }))()
        );
    }
    fn rmdir(&self, _: &Request, parent: INodeNo, n: &OsStr, reply: ReplyEmpty) {
        empty!(
            reply,
            (|| self.call(Action::Unlink {
                parent: parent.0,
                name: name(n)?,
                directory: true
            }))()
        );
    }
    fn rename(
        &self,
        _: &Request,
        parent: INodeNo,
        n: &OsStr,
        new_parent: INodeNo,
        new: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        empty!(
            reply,
            (|| {
                if flags.bits() & !libc::RENAME_NOREPLACE != 0 {
                    return Err(Errno::ENOTSUP);
                }
                self.call(Action::Rename {
                    parent: parent.0,
                    name: name(n)?,
                    new_parent: new_parent.0,
                    new_name: name(new)?,
                    no_replace: flags.contains(RenameFlags::RENAME_NOREPLACE),
                })
            })()
        );
    }
    fn statfs(&self, _: &Request, _: INodeNo, reply: ReplyStatfs) {
        let result = (|| {
            let path =
                CString::new(self.cache.as_os_str().as_bytes()).map_err(|_| Errno::EINVAL)?;
            let mut s = std::mem::MaybeUninit::<libc::statvfs>::uninit();
            if unsafe { libc::statvfs(path.as_ptr(), s.as_mut_ptr()) } < 0 {
                return Err(Errno::from_i32(
                    std::io::Error::last_os_error()
                        .raw_os_error()
                        .unwrap_or(libc::EIO),
                ));
            }
            Ok(unsafe { s.assume_init() })
        })();
        match result {
            Ok(s) => reply.statfs(
                s.f_blocks,
                s.f_bfree,
                s.f_bavail,
                s.f_files,
                s.f_ffree,
                s.f_bsize as u32,
                255,
                s.f_frsize as u32,
            ),
            Err(e) => reply.error(e),
        }
    }
    fn getxattr(&self, _: &Request, ino: INodeNo, n: &OsStr, size: u32, reply: ReplyXattr) {
        let result = (|| {
            if n != "user.neutrondrive.state" {
                return Err(Errno::ENODATA);
            }
            let v = self.call(Action::Metadata { inode: ino.0 })?;
            Ok(if v["pinned"] == true {
                b"pinned".as_slice()
            } else if v["cached"] == true {
                b"cached".as_slice()
            } else {
                b"online-only".as_slice()
            })
        })();
        match result {
            Ok(v) => xattr(reply, size, v),
            Err(e) => reply.error(e),
        }
    }
    fn listxattr(&self, _: &Request, _: INodeNo, size: u32, reply: ReplyXattr) {
        xattr(reply, size, b"user.neutrondrive.state\0");
    }
}
fn xattr(reply: ReplyXattr, size: u32, data: &[u8]) {
    if size == 0 {
        reply.size(data.len() as u32);
    } else if (size as usize) < data.len() {
        reply.error(Errno::ERANGE);
    } else {
        reply.data(data);
    }
}
pub fn run(
    config: Config,
    executable: Option<String>,
    callback: &mut impl FnMut(Value) -> i32,
) -> std::result::Result<Value, Failure> {
    let (tx, rx) = mpsc::sync_channel(16);
    let fs = Kernel {
        id: config.id,
        calls: tx,
        cache: config.cache,
        directories: Mutex::new((1, BTreeMap::new())),
    };
    let mut options = fuser::Config::default();
    options.n_threads = Some(4);
    options.mount_options = vec![
        MountOption::FSName("NeutronDrive".into()),
        MountOption::Subtype("neutrondrive".into()),
        MountOption::DefaultPermissions,
        MountOption::NoDev,
        MountOption::NoSuid,
    ];
    if !config.writable {
        options.mount_options.push(MountOption::RO);
    }
    let mut session = Session::new(fs, &config.mountpoint, &options).map_err(|error| {
        Failure::from(format!(
            "FUSE mount failed: {error}. Check /dev/fuse permissions and fuse3 installation."
        ))
    })?;
    if let Err(error) = crate::desktop_bookmarks::ensure(&config.mountpoint) {
        eprintln!("Could not add Proton Drive bookmark: {error}");
    }
    let mut unmounter = session.unmount_callable();
    let worker = std::thread::Builder::new()
        .name("nd-fuse".into())
        .spawn(move || session.run())
        .map_err(|_| Failure::from("Cannot start FUSE worker"))?;
    // Keep the pump alive while draining pending replies after cancellation.
    // The foreign callback remains scoped to this caller thread throughout.
    let mut cancelled = callback(json!({"kind":"sync_started"})) != 0;
    let mut detached = false;
    let mut generation = ondemand_fs::generation(config.id);
    let mut prefetched = BTreeMap::<u64, std::time::Instant>::new();
    loop {
        if cancelled && !detached {
            unmounter
                .unmount()
                .map_err(|_| Failure::from("Cannot detach cancelled FUSE mount"))?;
            detached = true;
        }
        let call = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(call) => call,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !cancelled {
                    cancelled = callback(json!({"kind":"poll"})) != 0;
                    if !cancelled {
                        if ondemand_fs::deletions_ready(config.id) {
                            let deletion = Operation::Call {
                                id: config.id,
                                operation: Action::DrainDeletes,
                            };
                            let result = match &executable {
                                Some(executable) => ondemand_fs::run(
                                    deletion,
                                    &mut crate::sync_remote::Sdk {
                                        executable: executable.clone(),
                                        callback,
                                    },
                                ),
                                None => {
                                    ondemand_fs::run(deletion, &mut crate::host::Callback(callback))
                                }
                            };
                            if let Ok(v) = result {
                                cancelled =
                                    callback(json!({"kind":"fuse_status","status":v["status"]}))
                                        != 0;
                            }
                        }
                        if cancelled {
                            continue;
                        }
                        let current_generation = ondemand_fs::generation(config.id);
                        if current_generation != generation {
                            prefetched.clear();
                            generation = current_generation;
                        }
                        // Breadth-first metadata-only traversal, one directory per idle turn.
                        // Cached kernel lookups use the independent mirror during this request.
                        if let Some(inode) =
                            ondemand_fs::folders(config.id).into_iter().find(|id| {
                                prefetched
                                    .get(id)
                                    .is_none_or(|t| t.elapsed() >= Duration::from_secs(300))
                            })
                        {
                            prefetched.insert(inode, std::time::Instant::now());
                            let operation = Operation::Call {
                                id: config.id,
                                operation: Action::Listdir {
                                    inode,
                                    refresh: true,
                                },
                            };
                            let result = match &executable {
                                Some(executable) => ondemand_fs::run(
                                    operation,
                                    &mut crate::sync_remote::Sdk {
                                        executable: executable.clone(),
                                        callback,
                                    },
                                ),
                                None => ondemand_fs::run(
                                    operation,
                                    &mut crate::host::Callback(callback),
                                ),
                            };
                            if let Ok(v) = result {
                                cancelled =
                                    callback(json!({"kind":"fuse_status","status":v["status"]}))
                                        != 0;
                            }
                        }
                    }
                }
                continue;
            }
        };
        if cancelled {
            let _ = call.reply.send(Err(Errno::EIO));
            continue;
        }
        let operation = Operation::Call {
            id: config.id,
            operation: call.action,
        };
        let result = match &executable {
            Some(executable) => ondemand_fs::run(
                operation,
                &mut crate::sync_remote::Sdk {
                    executable: executable.clone(),
                    callback,
                },
            ),
            None => ondemand_fs::run(operation, &mut crate::host::Callback(callback)),
        };
        let result = match result {
            Ok(v) => {
                cancelled = callback(json!({"kind":"fuse_status","status":v["status"]})) != 0;
                unpack(v)
            }
            Err(_) => Err(Errno::EIO),
        };
        let _ = call.reply.send(result);
    }
    worker
        .join()
        .map_err(|_| Failure::from("FUSE worker panicked"))?
        .map_err(|_| Failure::from("FUSE worker stopped unexpectedly"))?;
    if cancelled {
        return Err(Failure {
            message: "FUSE session cancelled; unsaved content retained locally".into(),
            cancelled: true,
        });
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_has_private_permissions_precise_times_and_online_blocks() {
        let mut v = json!({"inode":7,"size":513,"mtime_ns":-1,"folder":false,"writable":true,"cached":false});
        let a = attrs(v.clone()).unwrap();
        assert_eq!(a.perm, 0o600);
        assert_eq!(a.blocks, 0);
        assert_eq!(a.mtime, UNIX_EPOCH - Duration::from_nanos(1));
        v["cached"] = json!(true);
        v["writable"] = json!(false);
        let a = attrs(v.clone()).unwrap();
        assert_eq!(a.blocks, 2);
        assert_eq!(a.perm, 0o400);
        v["folder"] = json!(true);
        let a = attrs(v).unwrap();
        assert_eq!(a.perm, 0o500);
        assert_eq!(a.nlink, 2);
    }
    #[test]
    fn kernel_inputs_and_failures_do_not_silently_succeed() {
        assert!(name(OsStr::from_bytes(b"invalid\xff")).is_err());
        assert!(offset(u64::MAX).is_err());
        assert_eq!(
            unpack(json!({"errno":libc::ESTALE,"message":"conflict"})).unwrap_err(),
            Errno::ESTALE
        );
        assert!(unpack(json!({"backend_error":"failed"})).is_err());
        assert!(unpack(json!({})).is_err());
        assert_eq!(unpack(json!({"value":null})).unwrap(), Value::Null);
    }
}
