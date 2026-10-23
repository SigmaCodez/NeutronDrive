//! Owned daemon lock, Unix listener and control transport lifecycle.
use crate::{control, host::Services, process::Failure, storage};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::{File, Metadata},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{FileTypeExt, MetadataExt, PermissionsExt},
            net::UnixListener,
        },
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::Duration,
};
type Result<T> = std::result::Result<T, Failure>;
struct Server {
    parent: File,
    _lock: File,
    listener: UnixListener,
    socket: Metadata,
    pid: Option<Metadata>,
    stopped: AtomicBool,
    serving: AtomicBool,
}
fn owned_unlink(parent: &File, name: &std::ffi::CStr, expected: &Metadata) {
    let mut info: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            &mut info,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } == 0
        && info.st_dev == expected.dev()
        && info.st_ino == expected.ino()
    {
        unsafe {
            libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0);
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        owned_unlink(&self.parent, c"control.sock", &self.socket);
        if let Some(pid) = &self.pid {
            owned_unlink(&self.parent, c"daemon.pid", pid);
        }
    }
}
static SERVERS: LazyLock<Mutex<BTreeMap<u64, Arc<Server>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static NEXT: AtomicU64 = AtomicU64::new(1);
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Create { state: PathBuf },
    Serve { id: u64 },
    Stop { id: u64 },
    Release { id: u64 },
}
fn create(state: &Path) -> Result<Server> {
    let parent = storage::directory(state, true, true)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            c"daemon.lock".as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        return Err("Ungültige Dienstsperre.".into());
    }
    let lock = unsafe { File::from_raw_fd(fd) };
    let info = lock.metadata().map_err(|e| e.to_string())?;
    if !info.is_file()
        || info.nlink() != 1
        || info.uid() != unsafe { libc::getuid() }
        || info.mode() & 0o077 != 0
    {
        return Err("Ungültige private Dienstsperre.".into());
    }
    if unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) } < 0 {
        return Err("Der Hintergrunddienst läuft bereits.".into());
    }
    // Bind relative to the pinned directory via procfs, avoiding path replacement.
    let pinned = PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd()));
    let socket_path = pinned.join("control.sock");
    match std::fs::symlink_metadata(&socket_path) {
        Ok(old) => {
            if !old.file_type().is_socket() || old.uid() != unsafe { libc::getuid() } {
                return Err("Unsicherer Steuerungssocket; Start abgebrochen.".into());
            }
            owned_unlink(&parent, c"control.sock", &old);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string().into()),
    }
    let listener = UnixListener::bind(&socket_path).map_err(|e| e.to_string())?;
    let socket = std::fs::symlink_metadata(&socket_path).map_err(|e| e.to_string())?;
    let mut server = Server {
        parent,
        _lock: lock,
        listener,
        socket,
        pid: None,
        stopped: AtomicBool::new(false),
        serving: AtomicBool::new(false),
    };
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    server
        .listener
        .set_nonblocking(true)
        .map_err(|e| e.to_string())?;
    // Verify that the public state path still resolves to the pinned directory.
    let current = storage::directory(state, false, true)?;
    let a = current.metadata().map_err(|e| e.to_string())?;
    let b = server.parent.metadata().map_err(|e| e.to_string())?;
    if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
        return Err("Dienstverzeichnis wurde ersetzt.".into());
    }
    storage::write(&state.join("daemon.pid"), json!({"pid":std::process::id()}))?;
    server.pid =
        Some(std::fs::symlink_metadata(pinned.join("daemon.pid")).map_err(|e| e.to_string())?);
    Ok(server)
}
fn serve(server: &Server, host: &mut impl Services) -> Result<Value> {
    if server.serving.swap(true, Ordering::AcqRel) {
        return Err("Dienstsocket wird bereits bedient.".into());
    }
    struct Serving<'a>(&'a AtomicBool);
    impl Drop for Serving<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    let _serving = Serving(&server.serving);
    while !server.stopped.load(Ordering::Acquire) {
        let client = match server.listener.accept() {
            Ok((client, _)) => client,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
                continue;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string().into()),
        };
        let response = match control::receive(client.as_raw_fd(), 2.0)
            .and_then(|request| host.request("daemon_command", request))
        {
            Ok(result) => json!({"ok":true,"result":result}),
            Err(error) => json!({"ok":false,"error":error.message}),
        };
        let _ = control::respond(client.as_raw_fd(), response, 2.0);
    }
    Ok(Value::Null)
}
pub fn run(operation: Operation, host: &mut impl Services) -> Result<Value> {
    match operation {
        Operation::Create { state } => {
            let server = Arc::new(create(&state)?);
            let mut servers = SERVERS.lock().map_err(|_| "Dienstregister ist gesperrt.")?;
            if servers.len() >= 128 {
                return Err("Zu viele Dienstsockets.".into());
            }
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            servers.insert(id, server);
            Ok(json!(id))
        }
        Operation::Release { id } => {
            if let Some(server) = SERVERS
                .lock()
                .map_err(|_| "Dienstregister ist gesperrt.")?
                .remove(&id)
            {
                server.stopped.store(true, Ordering::Release);
            }
            Ok(Value::Null)
        }
        Operation::Stop { id } | Operation::Serve { id } => {
            let server = SERVERS
                .lock()
                .map_err(|_| "Dienstregister ist gesperrt.")?
                .get(&id)
                .cloned()
                .ok_or("Unbekannter Dienstsocket")?;
            if matches!(operation, Operation::Stop { .. }) {
                server.stopped.store(true, Ordering::Release);
                Ok(Value::Null)
            } else {
                serve(&server, host)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nd-server-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn exclusive_lock_and_owned_cleanup_preserve_replacements() {
        let state = Temp::new();
        let server = create(&state.0).unwrap();
        assert!(create(&state.0).is_err());
        assert_eq!(
            std::fs::metadata(state.0.join("control.sock"))
                .unwrap()
                .mode()
                & 0o777,
            0o600
        );
        std::fs::rename(state.0.join("control.sock"), state.0.join("old.sock")).unwrap();
        std::fs::write(state.0.join("control.sock"), b"replacement").unwrap();
        std::fs::rename(state.0.join("daemon.pid"), state.0.join("old.pid")).unwrap();
        std::fs::write(state.0.join("daemon.pid"), b"replacement").unwrap();
        drop(server);
        assert_eq!(
            std::fs::read(state.0.join("control.sock")).unwrap(),
            b"replacement"
        );
        assert_eq!(
            std::fs::read(state.0.join("daemon.pid")).unwrap(),
            b"replacement"
        );
    }
    #[test]
    fn unsafe_socket_and_linked_lock_never_open_service() {
        let state = Temp::new();
        std::fs::write(state.0.join("control.sock"), b"preserve").unwrap();
        assert!(create(&state.0).is_err());
        assert_eq!(
            std::fs::read(state.0.join("control.sock")).unwrap(),
            b"preserve"
        );
        std::fs::remove_file(state.0.join("control.sock")).unwrap();
        std::fs::hard_link(state.0.join("daemon.lock"), state.0.join("other.lock")).unwrap();
        assert!(create(&state.0).is_err());
        assert!(!state.0.join("control.sock").exists());
    }
    #[test]
    fn release_during_request_retains_socket_until_reply_and_loop_exit() {
        struct Host {
            id: u64,
        }
        impl Services for Host {
            fn poll(&mut self) -> Result<()> {
                Ok(())
            }
            fn request(&mut self, method: &str, args: Value) -> Result<Value> {
                assert_eq!(method, "daemon_command");
                assert_eq!(args["action"], "status");
                run(Operation::Release { id: self.id }, self)?;
                Ok(json!({"running":true}))
            }
        }
        let state = Temp::new();
        let mut host = Host { id: 0 };
        let id = run(
            Operation::Create {
                state: state.0.clone(),
            },
            &mut host,
        )
        .unwrap()
        .as_u64()
        .unwrap();
        host.id = id;
        let task = std::thread::spawn(move || run(Operation::Serve { id }, &mut host).unwrap());
        assert_eq!(
            control::command(&state.0.join("control.sock"), "status", None, 2.0).unwrap()
                ["running"],
            true
        );
        task.join().unwrap();
        assert!(!state.0.join("control.sock").exists());
        assert!(!state.0.join("daemon.pid").exists());
        let replacement = create(&state.0).unwrap();
        drop(replacement);
    }
}
