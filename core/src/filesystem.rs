//! Descriptor-relative synchronization reads. Owned duplicates never close host FDs.
//! Digests only serve the existing SDK content comparison protocol, not signatures.
use crate::process::Failure;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{CStr, CString},
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd, RawFd},
    os::unix::fs::MetadataExt,
};
type Result<T> = std::result::Result<T, Failure>;
const DIR: i32 = libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
fn io(error: std::io::Error) -> Failure {
    error.to_string().into()
}
fn checkpoint(callback: &mut impl FnMut(Value) -> i32, phase: &str) -> Result<()> {
    if callback(json!({"kind":"poll","phase":phase})) != 0 {
        return Err(Failure {
            message: "Synchronisierung abgebrochen.".into(),
            cancelled: true,
        });
    }
    Ok(())
}
pub fn poll(callback: &mut impl FnMut(Value) -> i32) -> Result<()> {
    if callback(json!({"kind":"poll"})) != 0 {
        return Err(Failure {
            message: "Synchronisierung abgebrochen.".into(),
            cancelled: true,
        });
    }
    Ok(())
}
pub fn duplicate(fd: RawFd) -> Result<File> {
    let owned = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if owned < 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(owned) })
}
fn component(name: &str) -> Result<CString> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err("Invalid relative filename".into());
    }
    CString::new(name).map_err(|_| "Invalid relative filename".into())
}
pub fn open(parent: RawFd, name: &str, flags: i32) -> Result<File> {
    let name = component(name)?;
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
pub fn names(parent: RawFd) -> Result<Vec<String>> {
    // A new directory description avoids changing the caller's iteration offset.
    let fd = unsafe { libc::openat(parent, c".".as_ptr(), DIR) };
    if fd < 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    let pointer = unsafe { libc::fdopendir(fd) };
    if pointer.is_null() {
        unsafe { libc::close(fd) };
        return Err(io(std::io::Error::last_os_error()));
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            unsafe { libc::closedir(self.0) };
        }
    }
    let directory = Directory(pointer);
    let mut names = Vec::new();
    loop {
        unsafe {
            *libc::__errno_location() = 0;
        }
        let entry = unsafe { libc::readdir(directory.0) };
        if entry.is_null() {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(0) {
                return Err(io(error));
            }
            break;
        }
        let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let name = std::str::from_utf8(bytes)
            .map_err(|_| "Non-UTF-8 filenames cannot be addressed by the Proton SDK")?;
        names.push(name.to_owned());
    }
    names.sort();
    Ok(names)
}
fn stat(parent: RawFd, name: &str) -> Result<Option<libc::stat>> {
    let name = component(name)?;
    let mut stat = std::mem::MaybeUninit::uninit();
    if unsafe {
        libc::fstatat(
            parent,
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } < 0
    {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(io(error));
    }
    Ok(Some(unsafe { stat.assume_init() }))
}
fn stamp(info: &std::fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        info.dev(),
        info.ino(),
        info.size(),
        info.mtime(),
        info.mtime_nsec(),
        info.ctime(),
        info.ctime_nsec(),
    )
}
fn time(seconds: i64, nanos: i64) -> i128 {
    i128::from(seconds) * 1_000_000_000 + i128::from(nanos)
}
fn metadata(info: &std::fs::Metadata) -> Value {
    json!({"st_dev":info.dev(),"st_ino":info.ino(),"st_size":info.len(),"st_atime_ns":time(info.atime(),info.atime_nsec()),"st_mtime_ns":time(info.mtime(),info.mtime_nsec()),"st_ctime_ns":time(info.ctime(),info.ctime_nsec())})
}
pub fn read(
    fd: RawFd,
    output: Option<RawFd>,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    let mut source = duplicate(fd)?;
    let before = source.metadata().map_err(io)?;
    if !before.is_file() {
        return Err("Only regular files can be synchronized".into());
    }
    let mut output = output.map(duplicate).transpose()?;
    if let Some(output) = &output {
        let target = output.metadata().map_err(io)?;
        if !target.is_file() || (target.dev(), target.ino()) == (before.dev(), before.ino()) {
            return Err("Source and staging destination must be distinct regular files".into());
        }
    }
    let mut digest = Sha1::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        checkpoint(callback, "read")?;
        let count = source.read(&mut buffer).map_err(io)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        if let Some(output) = &mut output {
            output.write_all(&buffer[..count]).map_err(io)?;
        }
    }
    let after = source.metadata().map_err(io)?;
    if stamp(&before) != stamp(&after) {
        return Err("Local file changed while being read".into());
    }
    Ok(json!({"sha1":format!("{:x}",digest.finalize()),"stat":metadata(&after)}))
}
pub fn entry(parent: RawFd, name: &str, callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    poll(callback)?;
    let Some(info) = stat(parent, name)? else {
        return Ok(Value::Null);
    };
    let kind = info.st_mode & libc::S_IFMT;
    if kind == libc::S_IFDIR {
        return Ok(
            json!({"kind":"folder","mtime_ns":time(info.st_mtime,info.st_mtime_nsec),"inode":info.st_ino,"device":info.st_dev,"ctime_ns":time(info.st_ctime,info.st_ctime_nsec)}),
        );
    }
    if kind != libc::S_IFREG {
        return Err("Symbolic links and non-regular files are excluded".into());
    }
    let file = open(parent, name, libc::O_RDONLY | libc::O_NONBLOCK)?;
    let result = read(file.as_raw_fd(), None, callback)?;
    let after = &result["stat"];
    if json!(info.st_dev) != after["st_dev"] || json!(info.st_ino) != after["st_ino"] {
        return Err("Local file was replaced while being scanned".into());
    }
    Ok(
        json!({"kind":"file","sha1":result["sha1"],"size":after["st_size"],"mtime_ns":after["st_mtime_ns"],"inode":after["st_ino"],"device":after["st_dev"],"ctime_ns":after["st_ctime_ns"]}),
    )
}
pub fn name_error(name: &str) -> Option<&'static str> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        Some("Unsafe or ambiguous path component")
    } else if name.chars().any(|c| c < ' ' || c == '\u{7f}') {
        Some("Control characters in file name are unsupported")
    } else if name.contains(['*', '?', '[', '{']) {
        Some("Name contains glob syntax interpreted by the Proton CLI")
    } else if name.starts_with(".neutrondrive-") {
        Some("Reserved local backup/staging name; excluded from sync")
    } else {
        None
    }
}
pub fn scan(root: RawFd, callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    struct Scan {
        entries: BTreeMap<String, Value>,
        blocked: BTreeMap<String, String>,
        ancestors: BTreeSet<(u64, u64)>,
    }
    impl Scan {
        fn visit(
            &mut self,
            fd: RawFd,
            prefix: &str,
            depth: usize,
            callback: &mut impl FnMut(Value) -> i32,
        ) -> Result<()> {
            poll(callback)?;
            if depth > 256 {
                return Err("Directory nesting exceeds the safe scan limit".into());
            }
            for name in names(fd)? {
                poll(callback)?;
                let relative = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                if let Some(reason) = name_error(&name) {
                    self.blocked.insert(relative, reason.into());
                    continue;
                }
                let result = (|| {
                    let value = entry(fd, &name, callback)?;
                    if value.is_null() {
                        return Err("File disappeared during scan".into());
                    }
                    self.entries.insert(relative.clone(), value.clone());
                    if value["kind"] == "folder" {
                        let identity = (
                            value["device"]
                                .as_u64()
                                .ok_or("Invalid directory identity")?,
                            value["inode"]
                                .as_u64()
                                .ok_or("Invalid directory identity")?,
                        );
                        if self.ancestors.contains(&identity) {
                            return Err("Directory cycle or duplicate ancestor".into());
                        }
                        let child = open(fd, &name, DIR)?;
                        let info = child.metadata().map_err(io)?;
                        if identity != (info.dev(), info.ino()) {
                            return Err("Directory was replaced during scan".into());
                        }
                        self.ancestors.insert(identity);
                        let result = self.visit(child.as_raw_fd(), &relative, depth + 1, callback);
                        self.ancestors.remove(&identity);
                        result?;
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    let error: Failure = error;
                    if error.cancelled {
                        return Err(error);
                    }
                    self.blocked.insert(relative, error.message);
                }
            }
            Ok(())
        }
    }
    let root = duplicate(root)?;
    let info = root.metadata().map_err(io)?;
    if !info.is_dir() {
        return Err("An open local root directory is required".into());
    }
    let mut scan = Scan {
        entries: BTreeMap::new(),
        blocked: BTreeMap::new(),
        ancestors: BTreeSet::from([(info.dev(), info.ino())]),
    };
    scan.visit(root.as_raw_fd(), "", 0, callback)?;
    Ok(json!({"entries":scan.entries,"blocked":scan.blocked}))
}

fn parent(root: RawFd, relative: &str) -> Result<(File, String)> {
    if relative.is_empty() || relative.starts_with('/') {
        return Err("Invalid relative path".into());
    }
    let parts: Vec<_> = relative.split('/').collect();
    if parts.iter().any(|p| name_error(p).is_some()) {
        return Err("Unsafe relative path".into());
    }
    let mut directory = duplicate(root)?;
    for name in &parts[..parts.len() - 1] {
        directory = open(directory.as_raw_fd(), name, DIR)?;
    }
    Ok((directory, parts.last().unwrap().to_string()))
}
fn create(parent: RawFd, name: &str) -> Result<File> {
    let name = component(name)?;
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn equal(left: &Value, right: &Value) -> bool {
    if left.is_null() || right.is_null() {
        return left.is_null() && right.is_null();
    }
    ["kind", "sha1", "uid", "revision", "remote_path"]
        .iter()
        .all(|k| left[*k].as_str().unwrap_or("") == right[*k].as_str().unwrap_or(""))
        && ["size", "mtime_ns", "inode", "device"].iter().all(|k| {
            left[*k]
                .as_number()
                .and_then(serde_json::Number::as_i128)
                .unwrap_or(0)
                == right[*k]
                    .as_number()
                    .and_then(serde_json::Number::as_i128)
                    .unwrap_or(0)
        })
}
fn random_name(prefix: &str) -> Result<String> {
    let mut bytes = [0; 16];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(io)?;
    Ok(format!(
        ".neutrondrive-{prefix}-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}
pub fn stage_upload(
    root: RawFd,
    relative: &str,
    expected: Value,
    folder: &std::path::Path,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    let (parent, name) = parent(root, relative)?;
    let source = open(parent.as_raw_fd(), &name, libc::O_RDONLY | libc::O_NONBLOCK)?;
    let target_dir = crate::storage::directory(folder, false, false)?;
    let target = create(target_dir.as_raw_fd(), &name)?;
    let proof = read(source.as_raw_fd(), Some(target.as_raw_fd()), callback)?;
    target.sync_all().map_err(io)?;
    let info = &proof["stat"];
    let actual = json!({"kind":"file","sha1":proof["sha1"],"size":info["st_size"],"mtime_ns":info["st_mtime_ns"],"inode":info["st_ino"],"device":info["st_dev"]});
    if !equal(&actual, &expected) {
        return Err("Local upload source changed after preview".into());
    }
    let ns = expected["mtime_ns"]
        .as_i64()
        .ok_or("Invalid local modification time")?;
    let atime = info["st_atime_ns"]
        .as_i64()
        .ok_or("Invalid local access time")?;
    let times = [
        libc::timespec {
            tv_sec: atime.div_euclid(1_000_000_000),
            tv_nsec: atime.rem_euclid(1_000_000_000),
        },
        libc::timespec {
            tv_sec: ns.div_euclid(1_000_000_000),
            tv_nsec: ns.rem_euclid(1_000_000_000),
        },
    ];
    if unsafe { libc::futimens(target.as_raw_fd(), times.as_ptr()) } < 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    target.sync_all().map_err(io)?;
    target_dir.sync_all().map_err(io)?;
    Ok(json!(folder.join(name)))
}
fn link(parent: RawFd, source: &str, target: &str) -> Result<()> {
    let source = component(source)?;
    let target = component(target)?;
    if unsafe { libc::linkat(parent, source.as_ptr(), parent, target.as_ptr(), 0) } < 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    Ok(())
}
pub fn install_download(
    root: RawFd,
    relative: &str,
    source: &std::path::Path,
    expected: Value,
    root_path: &std::path::Path,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    let (parent, name) = parent(root, relative)?;
    let temporary = random_name("stage")?;
    let output = create(parent.as_raw_fd(), &temporary)?;
    struct Temporary<'a>(&'a File, String);
    impl Drop for Temporary<'_> {
        fn drop(&mut self) {
            if let Ok(name) = component(&self.1) {
                unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), 0) };
            }
        }
    }
    let _temporary = Temporary(&parent, temporary.clone());
    let mut backup = None;
    let result = (|| {
        let source_dir = crate::storage::directory(
            source.parent().ok_or("Invalid staging file")?,
            false,
            false,
        )?;
        let source = open(
            source_dir.as_raw_fd(),
            source
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("Invalid staging filename")?,
            libc::O_RDONLY | libc::O_NONBLOCK,
        )?;
        read(source.as_raw_fd(), Some(output.as_raw_fd()), callback)?;
        output.sync_all().map_err(io)?;
        if !equal(&entry(parent.as_raw_fd(), &name, callback)?, &expected) {
            return Err("Local destination changed while downloading".into());
        }
        if !expected.is_null() {
            let target = random_name("backup")?;
            let from = component(&name)?;
            let to = component(&target)?;
            // Never overwrite a backup, even in the unlikely event of an ID collision.
            if unsafe {
                libc::renameat2(
                    parent.as_raw_fd(),
                    from.as_ptr(),
                    parent.as_raw_fd(),
                    to.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            } < 0
            {
                return Err(io(std::io::Error::last_os_error()));
            }
            backup = Some(target.clone());
            parent.sync_all().map_err(io)?;
            let path = root_path
                .join(std::path::Path::new(relative).parent().unwrap())
                .join(&target);
            if callback(json!({"kind":"local_backup","path":path})) != 0 {
                return Err(Failure {
                    message: "Synchronisierung abgebrochen.".into(),
                    cancelled: true,
                });
            }
            if !equal(&entry(parent.as_raw_fd(), &target, callback)?, &expected) {
                return Err("Local destination changed during backup; original retained".into());
            }
        }
        checkpoint(callback, "install")?;
        link(parent.as_raw_fd(), &temporary, &name)?;
        parent.sync_all().map_err(io)?;
        Ok(Value::Null)
    })();
    if result.is_err() {
        if let Some(backup) = backup {
            let _ = link(parent.as_raw_fd(), &backup, &name);
            let _ = parent.sync_all();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Tree {
        base: PathBuf,
        root: File,
    }
    impl Tree {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!(
                "nd-native-fs-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&base).unwrap();
            std::fs::create_dir(base.join("root")).unwrap();
            let root = File::open(base.join("root")).unwrap();
            Self { base, root }
        }
        fn local(&self) -> PathBuf {
            self.base.join("root")
        }
        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.local().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }
    #[test]
    fn hashes_bytes_and_repeated_scans_preserve_the_borrowed_descriptor() {
        let tree = Tree::new();
        tree.file("a", b"abc");
        let first = scan(tree.root.as_raw_fd(), &mut |_| 0).unwrap();
        assert_eq!(
            first["entries"]["a"]["sha1"],
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(scan(tree.root.as_raw_fd(), &mut |_| 0).unwrap(), first);
        assert!(tree.root.metadata().unwrap().is_dir());
    }
    #[test]
    fn links_special_and_reserved_files_are_blocked() {
        use std::os::unix::fs::symlink;
        let tree = Tree::new();
        let outside = tree.base.join("outside");
        std::fs::write(&outside, b"preserve").unwrap();
        symlink(&outside, tree.local().join("link")).unwrap();
        tree.file(".neutrondrive-backup-test", b"preserve");
        let result = scan(tree.root.as_raw_fd(), &mut |_| 0).unwrap();
        assert_eq!(result["blocked"].as_object().unwrap().len(), 2);
        assert!(result["entries"].as_object().unwrap().is_empty());
        assert_eq!(std::fs::read(outside).unwrap(), b"preserve");
    }
    #[test]
    fn non_utf8_names_fail_closed_without_loss() {
        use std::os::unix::ffi::OsStringExt;
        let tree = Tree::new();
        let path = tree.local().join(std::ffi::OsString::from_vec(vec![255]));
        std::fs::write(&path, b"preserve").unwrap();
        assert!(scan(tree.root.as_raw_fd(), &mut |_| 0).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"preserve");
    }
    #[test]
    fn cancel_during_local_hash_scan_aborts_before_transfer() {
        let tree = Tree::new();
        tree.file("large", &vec![b'x'; 3 * 1024 * 1024]);
        let mut reads = 0;
        let result = scan(tree.root.as_raw_fd(), &mut |event| {
            if event["phase"] == "read" {
                reads += 1;
                if reads == 2 {
                    return 1;
                }
            }
            0
        });
        assert!(result.unwrap_err().cancelled);
        assert_eq!(reads, 2);
        assert_eq!(
            std::fs::metadata(tree.local().join("large")).unwrap().len(),
            3 * 1024 * 1024
        );
    }
    #[test]
    fn new_local_name_racing_install_is_not_replaced() {
        let tree = Tree::new();
        let source = tree.base.join("download");
        std::fs::write(&source, b"remote").unwrap();
        let result = install_download(
            tree.root.as_raw_fd(),
            "a",
            &source,
            Value::Null,
            &tree.local(),
            &mut |event| {
                if event["phase"] == "install" {
                    tree.file("a", b"concurrent");
                }
                0
            },
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(tree.local().join("a")).unwrap(),
            b"concurrent"
        );
        assert_eq!(names(tree.root.as_raw_fd()).unwrap(), vec!["a"]);
    }
    #[test]
    fn failed_install_restores_old_file_and_preserves_backup() {
        let tree = Tree::new();
        tree.file("a", b"original");
        let expected = entry(tree.root.as_raw_fd(), "a", &mut |_| 0).unwrap();
        let source = tree.base.join("download");
        std::fs::write(&source, b"remote").unwrap();
        let mut backup = None;
        let result = install_download(
            tree.root.as_raw_fd(),
            "a",
            &source,
            expected,
            &tree.local(),
            &mut |event| {
                if event["kind"] == "local_backup" {
                    backup = Some(PathBuf::from(event["path"].as_str().unwrap()));
                }
                if event["phase"] == "install" {
                    for name in names(tree.root.as_raw_fd()).unwrap() {
                        if name.starts_with(".neutrondrive-stage-") {
                            std::fs::remove_file(tree.local().join(name)).unwrap();
                        }
                    }
                }
                0
            },
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(tree.local().join("a")).unwrap(), b"original");
        assert_eq!(std::fs::read(backup.unwrap()).unwrap(), b"original");
    }
    #[test]
    fn cancellation_after_backup_restores_original() {
        let tree = Tree::new();
        tree.file("a", b"original");
        let expected = entry(tree.root.as_raw_fd(), "a", &mut |_| 0).unwrap();
        let source = tree.base.join("download");
        std::fs::write(&source, b"remote").unwrap();
        let result = install_download(
            tree.root.as_raw_fd(),
            "a",
            &source,
            expected,
            &tree.local(),
            &mut |event| i32::from(event["kind"] == "local_backup"),
        );
        assert!(result.unwrap_err().cancelled);
        assert_eq!(std::fs::read(tree.local().join("a")).unwrap(), b"original");
    }
    #[test]
    fn writes_to_source_during_read_are_detected() {
        let tree = Tree::new();
        let path = tree.file("a", b"original");
        let mut calls = 0;
        let result = entry(tree.root.as_raw_fd(), "a", &mut |event| {
            if event["phase"] == "read" {
                calls += 1;
                if calls == 2 {
                    std::fs::write(&path, b"changed").unwrap();
                }
            }
            0
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"changed");
    }
}

fn mount_id(fd: RawFd) -> Result<u64> {
    let text = std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")).map_err(io)?;
    text.lines()
        .find_map(|line| {
            line.strip_prefix("mnt_id:")
                .and_then(|v| v.trim().parse().ok())
        })
        .ok_or_else(|| "Cannot verify the Linux mount identity".into())
}
fn stat_stamp(info: &libc::stat) -> (u64, u64, i64, i64, i64, i64, i64) {
    (
        info.st_dev,
        info.st_ino,
        info.st_size,
        info.st_mtime,
        info.st_mtime_nsec,
        info.st_ctime,
        info.st_ctime_nsec,
    )
}
pub fn snapshot(
    parent: RawFd,
    name: &str,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    snapshot_with_mounts(parent, name, callback, &mut mount_id)
}
fn snapshot_with_mounts(
    parent: RawFd,
    name: &str,
    callback: &mut impl FnMut(Value) -> i32,
    mount: &mut impl FnMut(RawFd) -> Result<u64>,
) -> Result<Value> {
    struct Tree {
        device: u64,
        mount: u64,
        ancestors: BTreeSet<(u64, u64)>,
        values: BTreeMap<String, Value>,
    }
    impl Tree {
        fn visit(
            &mut self,
            parent: RawFd,
            name: &str,
            relative: &str,
            depth: usize,
            callback: &mut impl FnMut(Value) -> i32,
            mount: &mut impl FnMut(RawFd) -> Result<u64>,
        ) -> Result<()> {
            poll(callback)?;
            if self.values.len() >= 100000 || depth > 256 {
                return Err("Trash tree exceeds the verification limit".into());
            }
            let before =
                stat(parent, name)?.ok_or("Source disappeared during trash verification")?;
            if before.st_dev != self.device {
                return Err("Nested filesystems cannot be moved into this trash".into());
            }
            let directory = before.st_mode & libc::S_IFMT == libc::S_IFDIR;
            let regular = before.st_mode & libc::S_IFMT == libc::S_IFREG && before.st_nlink == 1;
            if !directory && !regular {
                return Err("Links and special files cannot be automatically trashed".into());
            }
            let child = open(
                parent,
                name,
                if directory {
                    DIR
                } else {
                    libc::O_RDONLY | libc::O_NONBLOCK
                },
            )?;
            let opened = child.metadata().map_err(io)?;
            let identity = (opened.dev(), opened.ino());
            if identity != (before.st_dev, before.st_ino) || mount(child.as_raw_fd())? != self.mount
            {
                return Err("File or directory replaced during trash verification".into());
            }
            if directory {
                if !self.ancestors.insert(identity) {
                    return Err("Directory replaced during trash verification".into());
                }
                let children = names(child.as_raw_fd())?;
                self.values.insert(relative.into(),json!({"kind":"folder","device":opened.dev(),"inode":opened.ino(),"size":0,"mtime_ns":time(opened.mtime(),opened.mtime_nsec()),"ctime_ns":time(opened.ctime(),opened.ctime_nsec()),"sha1":""}));
                for name in &children {
                    let path = if relative.is_empty() {
                        name.clone()
                    } else {
                        format!("{relative}/{name}")
                    };
                    self.visit(child.as_raw_fd(), name, &path, depth + 1, callback, mount)?;
                }
                if stamp(&opened) != stamp(&child.metadata().map_err(io)?)
                    || children != names(child.as_raw_fd())?
                {
                    return Err("Directory contents changed during trash verification".into());
                }
                checkpoint(callback, "sync_directory")?;
                child.sync_all().map_err(io)?;
                self.ancestors.remove(&identity);
            } else {
                if !opened.is_file()
                    || opened.nlink() != 1
                    || stamp(&opened)
                        != (
                            before.st_dev,
                            before.st_ino,
                            before.st_size as u64,
                            before.st_mtime,
                            before.st_mtime_nsec,
                            before.st_ctime,
                            before.st_ctime_nsec,
                        )
                {
                    return Err("File replaced during trash verification".into());
                }
                let proof = read(child.as_raw_fd(), None, callback)?;
                let after = child.metadata().map_err(io)?;
                if stamp(&opened) != stamp(&after) {
                    return Err("File changed during trash verification".into());
                }
                child.sync_all().map_err(io)?;
                self.values.insert(relative.into(),json!({"kind":"file","device":after.dev(),"inode":after.ino(),"size":after.size(),"mtime_ns":time(after.mtime(),after.mtime_nsec()),"ctime_ns":time(after.ctime(),after.ctime_nsec()),"sha1":proof["sha1"]}));
            }
            let after =
                stat(parent, name)?.ok_or("Source disappeared during trash verification")?;
            if stat_stamp(&before) != stat_stamp(&after) {
                return Err("Source changed during trash verification".into());
            }
            Ok(())
        }
    }
    let initial = stat(parent, name)?.ok_or("Source disappeared during trash verification")?;
    let mut tree = Tree {
        device: initial.st_dev,
        mount: mount(parent)?,
        ancestors: BTreeSet::new(),
        values: BTreeMap::new(),
    };
    tree.visit(parent, name, "", 0, callback, mount)?;
    Ok(json!(tree.values))
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn mount_identity_change_is_rejected_even_on_same_device() {
        let root =
            std::env::temp_dir().join(format!("nd-native-mount-proof-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("data"), b"synthetic").unwrap();
        let file = File::open(&root).unwrap();
        let mut id = 0;
        let result = snapshot_with_mounts(file.as_raw_fd(), "data", &mut |_| 0, &mut |_| {
            id += 1;
            Ok(id)
        });
        assert!(result.unwrap_err().message.contains("replaced"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn every_nested_directory_is_fsynced_before_move() {
        let root = std::env::temp_dir().join(format!("nd-native-snapshot-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir_all(root.join("folder/nested")).unwrap();
        std::fs::write(root.join("folder/nested/data"), b"new child").unwrap();
        let fd = File::open(&root).unwrap();
        let mut directories = 0;
        let value = snapshot(fd.as_raw_fd(), "folder", &mut |event| {
            if event["phase"] == "sync_directory" {
                directories += 1;
            }
            0
        })
        .unwrap();
        assert_eq!(directories, 2);
        assert_eq!(value.as_object().unwrap().len(), 3);
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// Stat-only echo proof. Any uncertainty schedules reconciliation instead of
/// suppressing a possibly genuine local edit. Traversal never follows symlinks.
pub fn matches_snapshot(root: &std::path::Path, relative: &str, snapshot: Value) -> bool {
    fn check(root: &std::path::Path, relative: &str, snapshot: &Value) -> Result<bool> {
        let expected = snapshot.as_object().ok_or("Invalid echo snapshot")?;
        let directory = crate::storage::directory(root, false, false)?;
        let identity = directory.metadata().map_err(io)?;
        let path = root.join(relative);
        for (mounted, kind) in crate::jobs::mounts()? {
            let mounted = std::path::Path::new(&mounted);
            let overlap = path.starts_with(mounted) || mounted.starts_with(&path);
            if overlap
                && (kind == "autofs"
                    || kind.starts_with("fuse")
                    || (mounted != root && mounted.starts_with(root)))
            {
                return Ok(false);
            }
        }
        let (parent, name) = parent(directory.as_raw_fd(), relative)?;
        fn visit(
            fd: RawFd,
            name: &str,
            relative: &str,
            expected: &serde_json::Map<String, Value>,
            found: &mut BTreeSet<String>,
            depth: usize,
        ) -> Result<bool> {
            if depth > 256 || found.len() >= 100000 {
                return Ok(false);
            }
            let Some(info) = stat(fd, name)? else {
                return Ok(false);
            };
            let Some(entry) = expected.get(relative) else {
                return Ok(false);
            };
            if entry["device"] != json!(info.st_dev) || entry["inode"] != json!(info.st_ino) {
                return Ok(false);
            }
            found.insert(relative.into());
            match entry["kind"].as_str() {
                Some("file") => Ok(info.st_mode & libc::S_IFMT == libc::S_IFREG
                    && entry["size"] == json!(info.st_size)
                    && entry["mtime_ns"] == json!(time(info.st_mtime, info.st_mtime_nsec))
                    && entry["ctime_ns"] == json!(time(info.st_ctime, info.st_ctime_nsec))),
                Some("folder") => {
                    if info.st_mode & libc::S_IFMT != libc::S_IFDIR {
                        return Ok(false);
                    }
                    let child = open(fd, name, DIR)?;
                    let before = child.metadata().map_err(io)?;
                    if (before.dev(), before.ino()) != (info.st_dev, info.st_ino) {
                        return Ok(false);
                    }
                    for name in names(child.as_raw_fd())? {
                        if name.starts_with(".neutrondrive-") {
                            continue;
                        }
                        if !visit(
                            child.as_raw_fd(),
                            &name,
                            &format!("{relative}/{name}"),
                            expected,
                            found,
                            depth + 1,
                        )? {
                            return Ok(false);
                        }
                    }
                    let after = stat(fd, name)?.ok_or("Echo directory disappeared")?;
                    Ok(stat_stamp(&info) == stat_stamp(&after)
                        && stamp(&before) == stamp(&child.metadata().map_err(io)?))
                }
                _ => Ok(false),
            }
        }
        let mut found = BTreeSet::new();
        let matched = if stat(parent.as_raw_fd(), &name)?.is_none() {
            !expected.contains_key(relative)
                && !expected
                    .keys()
                    .any(|p| p.starts_with(&format!("{relative}/")))
        } else {
            visit(parent.as_raw_fd(), &name, relative, expected, &mut found, 0)?
                && found
                    == expected
                        .keys()
                        .filter(|p| {
                            p.as_str() == relative || p.starts_with(&format!("{relative}/"))
                        })
                        .cloned()
                        .collect()
        };
        let current = crate::storage::directory(root, false, false)?
            .metadata()
            .map_err(io)?;
        let (current_parent, _) = self::parent(directory.as_raw_fd(), relative)?;
        let original_parent = parent.metadata().map_err(io)?;
        let current_parent = current_parent.metadata().map_err(io)?;
        Ok(matched
            && (identity.dev(), identity.ino()) == (current.dev(), current.ino())
            && (original_parent.dev(), original_parent.ino())
                == (current_parent.dev(), current_parent.ino()))
    }
    check(root, relative, &snapshot).unwrap_or(false)
}
