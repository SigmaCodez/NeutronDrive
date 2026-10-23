//! Materialize the read-only AppImage payload before starting persistent services.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
};

type Manifest = BTreeMap<String, String>;
fn invalid(message: &str) -> io::Error {
    io::Error::other(message)
}
fn private(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.uid() != unsafe { libc::getuid() } || m.mode() & 0o077 != 0 {
        return Err(invalid(
            "AppImage runtime cache must be a private directory owned by you",
        ));
    }
    Ok(())
}
fn digest(path: &Path) -> io::Result<String> {
    let mut f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !f.metadata()?.is_file() {
        return Err(invalid("Runtime entry is not a regular file"));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = f.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn relative(name: &str) -> io::Result<&Path> {
    let path = Path::new(name);
    if name.is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(invalid("Unsafe runtime manifest path"));
    }
    Ok(path)
}
fn validate(root: &Path, manifest: &Manifest) -> io::Result<()> {
    for (name, expected) in manifest {
        let rel = relative(name)?;
        let mut ancestor = root.to_path_buf();
        if let Some(parent) = rel.parent() {
            for part in parent.components() {
                ancestor.push(part);
                if !fs::symlink_metadata(&ancestor)?.is_dir() {
                    return Err(invalid("Unsafe runtime directory"));
                }
            }
        }
        let path = root.join(rel);
        if let Some(link) = expected.strip_prefix("symlink:") {
            if fs::read_link(&path)? != Path::new(link) {
                return Err(invalid("Runtime link mismatch"));
            }
        } else if digest(&path)? != *expected {
            return Err(invalid("Runtime checksum mismatch"));
        }
    }
    Ok(())
}
fn copy_tree(source: &Path, target: &Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        let info = fs::symlink_metadata(&from)?;
        if info.is_dir() {
            fs::create_dir(&to)?;
            copy_tree(&from, &to)?;
            fs::set_permissions(&to, info.permissions())?;
        } else if info.file_type().is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(from)?, to)?;
        } else if info.is_file() {
            fs::copy(from, to)?;
        } else {
            return Err(invalid("Unsupported runtime file type"));
        }
    }
    Ok(())
}
struct Stage(PathBuf);
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
/// Return a stable cache path. Existing corrupt caches are never overwritten.
/// The manifest describes integrity, not publisher authenticity; callers must
/// obtain the AppImage from their trusted release channel.
pub fn prepare(source: &Path, cache: &Path) -> io::Result<PathBuf> {
    let raw = fs::read(source.join("runtime-manifest.json"))?;
    let manifest: Manifest = serde_json::from_slice(&raw)?;
    if !manifest.contains_key("AppRun") {
        return Err(invalid("Runtime manifest lacks AppRun"));
    }
    for name in manifest.keys() {
        relative(name)?;
    }
    let key = format!("{:x}", Sha256::digest(&raw));
    private(cache)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(cache.join(".lock"))?;
    let info = lock.metadata()?;
    if !info.is_file()
        || info.uid() != unsafe { libc::getuid() }
        || info.nlink() != 1
        || info.mode() & 0o077 != 0
    {
        return Err(invalid("Unsafe runtime cache lock"));
    }
    loop {
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } == 0 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    let target = cache.join(key);
    match fs::symlink_metadata(&target) {
        Ok(_) => {
            private(&target)?;
            validate(&target, &manifest).map_err(|_| invalid(&format!("AppImage runtime cache is damaged. Stop NeutronDrive, remove {} and launch again.", target.display())))?;
            if fs::read(target.join("runtime-manifest.json"))? != raw {
                return Err(invalid("Cached runtime manifest is damaged"));
            }
            return Ok(target);
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(e) => return Err(e),
    }
    let mut random = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut random)?;
    let name: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let stage = Stage(cache.join(format!(".stage-{name}")));
    fs::DirBuilder::new().mode(0o700).create(&stage.0)?;
    copy_tree(source, &stage.0)?;
    validate(&stage.0, &manifest)?;
    if fs::read(stage.0.join("runtime-manifest.json"))? != raw {
        return Err(invalid("Runtime manifest changed while copying"));
    }
    fs::rename(&stage.0, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let mut random = [0; 16];
            File::open("/dev/urandom")
                .unwrap()
                .read_exact(&mut random)
                .unwrap();
            let path =
                std::env::temp_dir().join(format!("nd-appimage-{:x}", Sha256::digest(random)));
            fs::create_dir(&path).unwrap();
            fs::create_dir(path.join("source")).unwrap();
            fs::write(path.join("source/AppRun"), b"launcher").unwrap();
            let manifest =
                BTreeMap::from([("AppRun", digest(&path.join("source/AppRun")).unwrap())]);
            fs::write(
                path.join("source/runtime-manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            Self(path)
        }
        fn source(&self) -> PathBuf {
            self.0.join("source")
        }
        fn cache(&self) -> PathBuf {
            self.0.join("cache")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn stable_reuse_and_corruption() {
        let f = Fixture::new();
        let target = prepare(&f.source(), &f.cache()).unwrap();
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o700);
        fs::remove_dir_all(f.source()).unwrap();
        assert_eq!(prepare(&target, &f.cache()).unwrap(), target);
        fs::write(target.join("AppRun"), "changed").unwrap();
        assert!(prepare(&target, &f.cache())
            .unwrap_err()
            .to_string()
            .contains("damaged"));
        assert_eq!(fs::read(target.join("AppRun")).unwrap(), b"changed");
    }
    #[test]
    fn rejects_shared_cache_and_symlink_lock() {
        let f = Fixture::new();
        fs::create_dir(f.cache()).unwrap();
        fs::set_permissions(f.cache(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(prepare(&f.source(), &f.cache()).is_err());
        fs::set_permissions(f.cache(), fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(f.source().join("AppRun"), f.cache().join(".lock")).unwrap();
        assert!(prepare(&f.source(), &f.cache()).is_err());
    }
    #[test]
    fn rejects_escape_and_ancestor_links() {
        let f = Fixture::new();
        for name in ["../escape", "/absolute", ""] {
            assert!(relative(name).is_err());
        }
        std::os::unix::fs::symlink(&f.0, f.source().join("link")).unwrap();
        assert!(validate(
            &f.source(),
            &BTreeMap::from([(
                "link/source/AppRun".into(),
                digest(&f.source().join("AppRun")).unwrap()
            )])
        )
        .is_err());
    }
    #[test]
    fn failed_verification_does_not_publish_cache() {
        let f = Fixture::new();
        fs::write(f.source().join("AppRun"), "corrupt").unwrap();
        assert!(prepare(&f.source(), &f.cache()).is_err());
        assert_eq!(fs::read_dir(f.cache()).unwrap().count(), 1);
    }
    #[test]
    fn replaced_cache_file_cannot_block_verification_with_fifo() {
        use std::os::unix::ffi::OsStrExt;
        let f = Fixture::new();
        let target = prepare(&f.source(), &f.cache()).unwrap();
        let file = target.join("AppRun");
        fs::remove_file(&file).unwrap();
        let path = std::ffi::CString::new(file.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(prepare(&f.source(), &f.cache())
            .unwrap_err()
            .to_string()
            .contains("damaged"));
    }
    #[test]
    fn concurrent_starts_publish_once_and_preserve_links() {
        let f = Fixture::new();
        std::os::unix::fs::symlink("AppRun", f.source().join("launcher-link")).unwrap();
        let manifest = BTreeMap::from([
            ("AppRun", digest(&f.source().join("AppRun")).unwrap()),
            ("launcher-link", "symlink:AppRun".into()),
        ]);
        fs::write(
            f.source().join("runtime-manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let results = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..4)
                .map(|_| scope.spawn(|| prepare(&f.source(), &f.cache()).unwrap()))
                .collect();
            threads
                .into_iter()
                .map(|t| t.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(results.iter().all(|path| *path == results[0]));
        assert_eq!(
            fs::read_link(results[0].join("launcher-link")).unwrap(),
            Path::new("AppRun")
        );
        assert_eq!(fs::read_dir(f.cache()).unwrap().count(), 2);
    }
}
