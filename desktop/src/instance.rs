//! Per-user window ownership, compatible with the existing Qt lock/socket paths.
use crate::{platform::qt, runtime::Failure};
use cxx_qt_lib::QString;
use std::{
    fs, io,
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
fn failure(message: &str) -> Failure {
    Failure {
        message: message.into(),
        cancelled: false,
    }
}
pub struct Instance {
    // Release socket before lock so a successor never loses its socket to our cleanup.
    listener: UnixListener,
    path: PathBuf,
    identity: (u64, u64),
    _lock: cxx::UniquePtr<qt::QLockFile>,
}
fn private_directory(path: &Path) -> Result<(), Failure> {
    if !path.is_absolute() {
        return Err(failure("Window activation requires an absolute directory"));
    }
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        match fs::symlink_metadata(ancestor) {
            Ok(info) if info.is_dir() && !info.file_type().is_symlink() => (),
            Ok(_) => return Err(failure("Unsafe window activation directory")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(ancestor)
                    .map_err(|_| failure("Could not create window activation directory"))?;
            }
            Err(_) => return Err(failure("Could not inspect window activation directory")),
        }
    }
    let info = fs::symlink_metadata(path)
        .map_err(|_| failure("Could not inspect window activation directory"))?;
    if info.uid() != unsafe { libc::getuid() } || info.mode() & 0o077 != 0 {
        return Err(failure("Window activation requires a private directory"));
    }
    Ok(())
}
fn connect(path: &Path) -> io::Result<UnixStream> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    };
    // Nonblocking connect also bounds a full listener backlog; no GUI-thread hang.
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as _;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid window socket path",
        ));
    }
    for (out, input) in address.sun_path.iter_mut().zip(bytes) {
        *out = *input as _;
    }
    if unsafe {
        libc::connect(
            stream.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&address) as _,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(stream)
}
impl Instance {
    /// None means an existing window was contacted successfully.
    pub fn claim(directory: &Path) -> Result<Option<Self>, Failure> {
        private_directory(directory)?;
        let path = directory.join("window.sock");
        let lockpath = directory.join("window.lock");
        let mut lock = qt::instance_lock(&QString::from(
            lockpath
                .to_str()
                .ok_or_else(|| failure("Invalid lock path"))?,
        ));
        if !qt::claim_lock(lock.pin_mut()) {
            let started = Instant::now();
            loop {
                if connect(&path).is_ok() {
                    return Ok(None);
                }
                if started.elapsed() >= Duration::from_millis(1500) {
                    return Err(failure(
                        "NeutronDrive is starting or not responding. Please try again.",
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        match fs::symlink_metadata(&path) {
            Ok(info) if info.file_type().is_socket() && info.uid() == unsafe { libc::getuid() } => {
                fs::remove_file(&path)
                    .map_err(|_| failure("Could not remove stale window socket"))?
            }
            Ok(_) => return Err(failure("Unsafe window activation socket")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(_) => return Err(failure("Could not inspect window activation socket")),
        }
        let listener =
            UnixListener::bind(&path).map_err(|_| failure("Could not start window activation"))?;
        let info =
            fs::symlink_metadata(&path).map_err(|_| failure("Could not inspect window socket"))?;
        let owned = Self {
            listener,
            path,
            identity: (info.dev(), info.ino()),
            _lock: lock,
        };
        fs::set_permissions(&owned.path, fs::Permissions::from_mode(0o600))
            .map_err(|_| failure("Could not protect window socket"))?;
        owned
            .listener
            .set_nonblocking(true)
            .map_err(|_| failure("Could not configure window activation"))?;
        Ok(Some(owned))
    }
    /// Called by the Qt activation timer. Bound each tick so flooding cannot starve UI events.
    pub fn pending(&self) -> bool {
        let mut activate = false;
        for _ in 0..16 {
            match self.listener.accept() {
                Ok((_stream, _)) => activate = true,
                Err(_) => break,
            }
        }
        activate
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path)
            .is_ok_and(|m| (m.dev(), m.ino()) == self.identity && m.file_type().is_socket())
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("nd-instance-{name}-{}", std::process::id()));
            fs::DirBuilder::new().mode(0o700).create(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn activates_existing_owner_and_releases_for_successor() {
        let temp = Temp::new("ownership");
        let owner = Instance::claim(&temp.0).unwrap().unwrap();
        assert!(!owner.pending());
        assert!(Instance::claim(&temp.0).unwrap().is_none());
        assert!(owner.pending());
        assert!(!owner.pending());
        drop(owner);
        assert!(!temp.0.join("window.sock").exists());
        assert!(!temp.0.join("window.lock").exists());
        let successor = Instance::claim(&temp.0).unwrap().unwrap();
        drop(successor);
    }
    #[test]
    fn rejects_symlinks_public_directory_and_non_socket_without_removing_them() {
        let temp = Temp::new("unsafe");
        let target = temp.0.join("target");
        fs::create_dir(&target).unwrap();
        let link = temp.0.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(Instance::claim(&link).is_err());
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Instance::claim(&target).is_err());
        fs::write(temp.0.join("window.sock"), b"preserve").unwrap();
        assert!(Instance::claim(&temp.0).is_err());
        assert_eq!(fs::read(temp.0.join("window.sock")).unwrap(), b"preserve");
    }
    #[test]
    fn replaces_only_stale_socket_and_never_unlinks_successor_object() {
        let temp = Temp::new("stale");
        let stale = UnixListener::bind(temp.0.join("window.sock")).unwrap();
        drop(stale);
        let owner = Instance::claim(&temp.0).unwrap().unwrap();
        fs::remove_file(temp.0.join("window.sock")).unwrap();
        fs::write(temp.0.join("window.sock"), b"replacement").unwrap();
        drop(owner);
        assert_eq!(
            fs::read(temp.0.join("window.sock")).unwrap(),
            b"replacement"
        );
    }
}
