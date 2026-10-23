//! Private on-demand cache I/O and verified hydration transactions.
use crate::{host::Services, process::Failure, storage};
use serde::Deserialize;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::{
    fs::{File, Metadata},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd},
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, FileExt, MetadataExt},
        },
    },
    path::{Path, PathBuf},
};
#[derive(Debug)]
pub enum Error {
    Fs(i32, String),
    Backend(Failure),
}
pub type Result<T> = std::result::Result<T, Error>;
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Fs(e.raw_os_error().unwrap_or(libc::EIO), e.to_string())
    }
}
impl From<Failure> for Error {
    fn from(e: Failure) -> Self {
        Self::Backend(e)
    }
}
pub fn fail(code: i32, message: &str) -> Error {
    Error::Fs(code, message.into())
}
fn private_failure(error: Failure) -> Error {
    fail(libc::EACCES, &error.message)
}
fn cstring(value: &std::ffi::OsStr) -> Result<std::ffi::CString> {
    std::ffi::CString::new(value.as_bytes()).map_err(|_| fail(libc::EINVAL, "Invalid cache path"))
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Key {
        node: Value,
    },
    PrivateDirectory {
        path: PathBuf,
    },
    Regular {
        path: PathBuf,
        flags: i32,
    },
    Digest {
        fd: RawFd,
    },
    ReadJson {
        path: PathBuf,
    },
    WriteJson {
        path: PathBuf,
        value: Value,
    },
    Verified {
        path: PathBuf,
        node: Value,
    },
    Hydrate {
        objects: PathBuf,
        staging: PathBuf,
        node: Value,
        offline: bool,
        remote_root: String,
        root_uid: String,
    },
}
pub fn field<'a>(value: &'a Value, name: &str) -> &'a str {
    value[name].as_str().unwrap_or("")
}
pub fn revision(node: &Value) -> Value {
    json!([
        node["uid"],
        node["revision"],
        field(node, "sha1").to_lowercase(),
        node["size"]
    ])
}
pub(crate) fn ascii_json(value: &Value) -> String {
    let encoded = value.to_string();
    let mut out = String::new();
    for ch in encoded.chars() {
        if ch.is_ascii() {
            out.push(ch);
        } else {
            for code in ch.encode_utf16(&mut [0; 2]) {
                out.push_str(&format!("\\u{code:04x}"));
            }
        }
    }
    out
}
pub fn key(node: &Value) -> Result<String> {
    let uid = node["uid"]
        .as_str()
        .ok_or_else(|| fail(libc::EINVAL, "Missing cache node identity"))?;
    let tuple = revision(node);
    let encoded = format!(
        "[{}]",
        tuple
            .as_array()
            .unwrap()
            .iter()
            .map(ascii_json)
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(format!(
        "{:x}-{:x}",
        Sha256::digest(uid.as_bytes()),
        Sha256::digest(encoded.as_bytes())
    ))
}
pub fn private_directory(path: &Path) -> Result<File> {
    let directory = storage::directory(path, true, false).map_err(private_failure)?;
    if directory.metadata()?.uid() != unsafe { libc::getuid() } {
        return Err(fail(
            libc::EACCES,
            "Cache directory belongs to another user",
        ));
    }
    if unsafe { libc::fchmod(directory.as_raw_fd(), 0o700) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(directory)
}
fn parent(path: &Path) -> Result<(File, std::ffi::CString)> {
    let parent = storage::directory(
        path.parent()
            .ok_or_else(|| fail(libc::EINVAL, "Invalid cache path"))?,
        false,
        false,
    )
    .map_err(private_failure)?;
    let name = path
        .file_name()
        .ok_or_else(|| fail(libc::EINVAL, "Invalid cache path"))?;
    Ok((parent, cstring(name)?))
}
fn check_regular(file: &File, private: bool) -> Result<Metadata> {
    let info = file.metadata()?;
    if !info.is_file() || info.uid() != unsafe { libc::getuid() } || info.nlink() != 1 {
        return Err(fail(
            libc::EACCES,
            "Cache object is not a private regular file",
        ));
    }
    if private && info.mode() & 0o077 != 0 {
        return Err(fail(
            libc::EACCES,
            "Cache object permissions are not private",
        ));
    }
    Ok(info)
}
pub fn regular(path: &Path, flags: i32) -> Result<File> {
    if flags & !libc::O_ACCMODE != 0 {
        return Err(fail(libc::EINVAL, "Invalid cache open flags"));
    }
    let (parent, name) = parent(path)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    check_regular(&file, true)?;
    Ok(file)
}
fn same(before: &Metadata, after: &Metadata) -> bool {
    (
        before.dev(),
        before.ino(),
        before.len(),
        before.mtime(),
        before.mtime_nsec(),
        before.ctime(),
        before.ctime_nsec(),
    ) == (
        after.dev(),
        after.ino(),
        after.len(),
        after.mtime(),
        after.mtime_nsec(),
        after.ctime(),
        after.ctime_nsec(),
    )
}
pub fn digest(file: &File) -> Result<(String, u64, Metadata)> {
    let before = check_regular(file, true)?;
    let mut sha = Sha1::new();
    let mut offset = 0;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = file.read_at(&mut buffer, offset)?;
        if count == 0 {
            break;
        }
        sha.update(&buffer[..count]);
        offset += count as u64;
    }
    let after = file.metadata()?;
    if !same(&before, &after) {
        return Err(fail(
            libc::EBUSY,
            "Cache object changed during verification",
        ));
    }
    Ok((format!("{:x}", sha.finalize()), offset, after))
}
pub fn verified(path: &Path, node: &Value) -> Result<File> {
    let file = regular(path, libc::O_RDONLY)?;
    let (digest, size, _) = digest(&file)?;
    if digest != field(node, "sha1").to_lowercase() || Some(size) != node["size"].as_u64() {
        return Err(fail(
            libc::EIO,
            "Cached content does not match the remote SHA1 and size",
        ));
    }
    Ok(file)
}
pub fn read_json(path: &Path) -> Result<Value> {
    let file = regular(path, libc::O_RDONLY)?;
    const LIMIT: u64 = 32 * 1024 * 1024;
    if file.metadata()?.len() > LIMIT {
        return Err(fail(libc::EFBIG, "Metadata file is too large"));
    }
    let mut bytes = vec![];
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(fail(libc::EFBIG, "Metadata file is too large"));
    }
    serde_json::from_slice(&bytes).map_err(|_| fail(libc::EIO, "Saved mount metadata is invalid"))
}
pub fn write_json(path: &Path, value: Value) -> Result<()> {
    let (parent, target) = parent(path)?;
    let info = parent.metadata()?;
    if info.uid() != unsafe { libc::getuid() } || info.mode() & 0o077 != 0 {
        return Err(fail(libc::EACCES, "Metadata directory is not private"));
    }
    let bytes =
        serde_json::to_vec(&value).map_err(|_| fail(libc::EINVAL, "Invalid cache metadata"))?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(fail(libc::EFBIG, "Metadata file is too large"));
    }
    let mut random = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut random)?;
    let name = std::ffi::CString::new(format!(
        ".metadata-{}",
        random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
    .unwrap();
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    struct Temporary<'a>(&'a File, std::ffi::CString);
    impl Drop for Temporary<'_> {
        fn drop(&mut self) {
            unsafe {
                libc::unlinkat(self.0.as_raw_fd(), self.1.as_ptr(), 0);
            }
        }
    }
    let temporary = Temporary(&parent, name);
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(&bytes)?;
    file.sync_all()?;
    if unsafe {
        libc::renameat(
            parent.as_raw_fd(),
            temporary.1.as_ptr(),
            parent.as_raw_fd(),
            target.as_ptr(),
        )
    } < 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    parent.sync_all()?;
    let current = storage::directory(path.parent().unwrap(), false, true)
        .map_err(private_failure)?
        .metadata()?;
    if (current.dev(), current.ino()) != (info.dev(), info.ino()) {
        return Err(fail(libc::EBUSY, "Cache metadata directory was replaced"));
    }
    Ok(())
}

pub struct Stage {
    pub path: PathBuf,
    directory: File,
}
impl Stage {
    pub fn create_file(&self, name: &str) -> Result<File> {
        crate::backend::name(name)?;
        let name = cstring(std::ffi::OsStr::new(name))?;
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    pub fn install(&self, name: &str, destination: &Path, target: &str) -> Result<()> {
        crate::backend::name(name)?;
        crate::backend::name(target)?;
        let source = cstring(std::ffi::OsStr::new(name))?;
        let target = cstring(std::ffi::OsStr::new(target))?;
        let parent = storage::directory(destination, false, true).map_err(private_failure)?;
        let before = parent.metadata()?;
        if unsafe {
            libc::renameat(
                self.directory.as_raw_fd(),
                source.as_ptr(),
                parent.as_raw_fd(),
                target.as_ptr(),
            )
        } < 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        parent.sync_all()?;
        let after = storage::directory(destination, false, true)
            .map_err(private_failure)?
            .metadata()?;
        if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
            return Err(fail(
                libc::EBUSY,
                "Cache directory moved during installation",
            ));
        }
        Ok(())
    }
    pub fn new(parent: &Path) -> Result<Self> {
        let _parent = private_directory(parent)?;
        let mut bytes = [0u8; 16];
        File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        let path = parent.join(format!(
            "download-{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        let directory = storage::directory(&path, false, true).map_err(private_failure)?;
        Ok(Self { path, directory })
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        if let (Ok(current), Ok(owned)) = (
            std::fs::symlink_metadata(&self.path),
            self.directory.metadata(),
        ) {
            if (current.dev(), current.ino()) == (owned.dev(), owned.ino()) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
    }
}
pub(crate) fn hydrate(
    objects: &Path,
    staging: &Path,
    node: &Value,
    offline: bool,
    remote_root: &str,
    root_uid: &str,
    host: &mut impl Services,
) -> Result<File> {
    let target = objects.join(key(node)?);
    match std::fs::symlink_metadata(&target) {
        Ok(_) => return verified(&target, node),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    if offline {
        return Err(fail(
            libc::ENETDOWN,
            "File content is not available offline; pin it while online",
        ));
    }
    let root = host.request("info", json!({"path":remote_root}))?;
    if root["uid"] != root_uid || root["type"] != "folder" {
        return Err(fail(libc::EACCES,"Cloud account or mount root changed. Unmount before reconnecting; unsaved content remains in recovery."));
    }
    let before = host.request("info", json!({"path":node["path"]}))?;
    if revision(&before) != revision(node) {
        return Err(fail(
            libc::ESTALE,
            "Remote file changed; refresh the folder before opening it",
        ));
    }
    let stage = Stage::new(staging)?;
    host.request(
        "download",
        json!({"paths":[node["path"]],"destination":stage.path}),
    )?;
    let names = crate::filesystem::names(stage.directory.as_raw_fd())
        .map_err(|e| fail(libc::EIO, &e.message))?;
    if names.len() != 1 || names[0] != field(node, "name") {
        return Err(fail(
            libc::EIO,
            "Download did not produce exactly the expected file",
        ));
    }
    let name = cstring(std::ffi::OsStr::new(&names[0]))?;
    let fd = unsafe {
        libc::openat(
            stage.directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    check_regular(&file, false)?;
    if unsafe { libc::fchmod(fd, 0o600) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let (sha, size, proof) = digest(&file)?;
    if sha != field(node, "sha1").to_lowercase() || Some(size) != node["size"].as_u64() {
        return Err(fail(
            libc::EIO,
            "Downloaded content failed size/SHA1 verification",
        ));
    }
    let after = host.request("info", json!({"path":node["path"]}))?;
    if revision(&after) != revision(node) {
        return Err(fail(
            libc::ESTALE,
            "Remote revision changed during download",
        ));
    }
    if !same(&proof, &file.metadata()?) {
        return Err(fail(
            libc::EBUSY,
            "Downloaded content changed before installation",
        ));
    }
    let mut path_info: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::fstatat(
            stage.directory.as_raw_fd(),
            name.as_ptr(),
            &mut path_info,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } < 0
        || path_info.st_dev != proof.dev()
        || path_info.st_ino != proof.ino()
    {
        return Err(fail(
            libc::EBUSY,
            "Downloaded file was replaced before installation",
        ));
    }
    file.sync_all()?;
    let (parent, target_name) = parent(&target)?;
    if unsafe {
        libc::renameat(
            stage.directory.as_raw_fd(),
            name.as_ptr(),
            parent.as_raw_fd(),
            target_name.as_ptr(),
        )
    } < 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    parent.sync_all()?;
    verified(&target, node)
}
pub fn run(operation: Operation, host: &mut impl Services) -> std::result::Result<Value, Failure> {
    let result = (|| -> Result<Value> {
        Ok(match operation {
            Operation::Key { node } => json!(key(&node)?),
            Operation::PrivateDirectory { path } => {
                private_directory(&path)?;
                json!(path)
            }
            Operation::Regular { path, flags } => json!(regular(&path, flags)?.into_raw_fd()),
            Operation::Digest { fd } => {
                let fd = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
                if fd < 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                let file = unsafe { File::from_raw_fd(fd) };
                let (sha, size, _) = digest(&file)?;
                json!([sha, size])
            }
            Operation::ReadJson { path } => read_json(&path)?,
            Operation::WriteJson { path, value } => {
                write_json(&path, value)?;
                Value::Null
            }
            Operation::Verified { path, node } => json!(verified(&path, &node)?.into_raw_fd()),
            Operation::Hydrate {
                objects,
                staging,
                node,
                offline,
                remote_root,
                root_uid,
            } => json!(hydrate(
                &objects,
                &staging,
                &node,
                offline,
                &remote_root,
                &root_uid,
                host
            )?
            .into_raw_fd()),
        })
    })();
    match result {
        Ok(value) => Ok(json!({"value":value})),
        Err(Error::Fs(errno, message)) => Ok(json!({"errno":errno,"message":message})),
        Err(Error::Backend(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::PermissionsExt,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "nd-cache-io-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&p).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    struct NoHost;
    impl Services for NoHost {
        fn poll(&mut self) -> std::result::Result<(), Failure> {
            Ok(())
        }
        fn request(&mut self, _: &str, _: Value) -> std::result::Result<Value, Failure> {
            panic!("unexpected SDK call")
        }
    }
    #[test]
    fn keys_match_existing_python_cache_including_unicode() {
        let node = json!({"uid":"node-é😀","revision":"rev-1","sha1":"ABCDEF","size":42});
        assert_eq!(key(&node).unwrap(),"4a72dd198c73017522168b6741f488df680aa4eb98553063827a732f80085503-51288d4446eb62a66fbb499d01e0679579ba7dec3961a46c38f6c953319f8c5f");
    }
    #[test]
    fn private_json_and_borrowed_digest_do_not_leak_or_close_descriptors() {
        let temp = Temp::new();
        let path = temp.0.join("metadata.json");
        write_json(&path, json!({"unicode":"日本語"})).unwrap();
        assert_eq!(read_json(&path).unwrap()["unicode"], "日本語");
        let file = regular(&path, libc::O_RDONLY).unwrap();
        assert!(run(
            Operation::Digest {
                fd: file.as_raw_fd()
            },
            &mut NoHost
        )
        .unwrap()["value"][0]
            .is_string());
        assert!(file.metadata().is_ok());
        std::fs::hard_link(&path, temp.0.join("linked")).unwrap();
        assert!(matches!(
            regular(&path, libc::O_RDONLY),
            Err(Error::Fs(libc::EACCES, _))
        ));
        assert_eq!(std::fs::read_dir(&temp.0).unwrap().count(), 2);
    }
    fn node() -> Value {
        json!({"uid":"file","name":"data","path":"/my-files/file","revision":"r1","sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","size":3})
    }
    struct Host {
        infos: usize,
        path: Option<PathBuf>,
        replace: bool,
    }
    impl Services for Host {
        fn poll(&mut self) -> std::result::Result<(), Failure> {
            Ok(())
        }
        fn request(&mut self, method: &str, args: Value) -> std::result::Result<Value, Failure> {
            if method == "download" {
                let p = PathBuf::from(args["destination"].as_str().unwrap()).join("data");
                std::fs::write(&p, b"abc").unwrap();
                self.path = Some(p);
                return Ok(Value::Null);
            }
            assert_eq!(method, "info");
            self.infos += 1;
            if self.infos == 1 {
                return Ok(json!({"uid":"root","type":"folder"}));
            }
            if self.infos == 3 && self.replace {
                let path = self.path.as_ref().unwrap();
                std::fs::rename(path, path.with_extension("old")).unwrap();
                std::fs::write(path, b"abc").unwrap();
            }
            Ok(node())
        }
    }
    #[test]
    fn hydration_verifies_identity_after_final_remote_check() {
        for replace in [false, true] {
            let temp = Temp::new();
            let objects = temp.0.join("objects");
            let staging = temp.0.join("staging");
            private_directory(&objects).unwrap();
            private_directory(&staging).unwrap();
            let result = hydrate(
                &objects,
                &staging,
                &node(),
                false,
                "/my-files",
                "root",
                &mut Host {
                    infos: 0,
                    path: None,
                    replace,
                },
            );
            if replace {
                assert!(matches!(result, Err(Error::Fs(libc::EBUSY, _))));
                assert_eq!(std::fs::read_dir(&objects).unwrap().count(), 0);
            } else {
                let file = result.unwrap();
                assert_eq!(digest(&file).unwrap().1, 3);
            }
            assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
        }
    }
}
