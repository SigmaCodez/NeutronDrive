//! Computer registration and folder setup. Failed setup never removes cloud data.
use crate::{
    backend, computer,
    drive::{self, Operation},
    filesystem, jobs,
    process::Failure,
    storage,
};
use serde_json::{json, Value};
use std::{
    fs::File,
    io::Read,
    os::fd::{AsRawFd, FromRawFd},
    os::unix::fs::MetadataExt,
    path::Path,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Failure>;
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Invalid computer setup response".into())
}
fn folder(value: &Value) -> bool {
    value["safe"] == true
        && value["type"] == "folder"
        && value["raw"]["isTrashed"] != true
        && value["uid"].as_str().is_some_and(|v| !v.is_empty())
}
fn unique_name() -> Result<String> {
    let mut hostname = [0u8; 256];
    if unsafe { libc::gethostname(hostname.as_mut_ptr().cast(), hostname.len()) } != 0 {
        return Err("Cannot determine computer name".into());
    }
    let end = hostname
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(hostname.len());
    let mut host: String = String::from_utf8_lossy(&hostname[..end])
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_".contains(c) {
                c
            } else {
                '-'
            }
        })
        .take(80)
        .collect();
    if host.is_empty() {
        host = "Linux".into();
    }
    let mut bytes = [0; 6];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|_| "Cannot generate computer identity")?;
    Ok(format!(
        "{host} — {}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}
fn lock(state: &Path, callback: &mut impl FnMut(Value) -> i32) -> Result<File> {
    let directory = storage::directory(state, true, true)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            c"computer.lock".as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err("Computer setup lock is unavailable".into());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let info = file.metadata().map_err(|_| "Invalid computer setup lock")?;
    if !info.is_file()
        || info.nlink() != 1
        || info.uid() != unsafe { libc::getuid() }
        || info.mode() & 0o077 != 0
    {
        return Err("Unsafe computer setup lock".into());
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        filesystem::poll(callback)?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            break;
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock
            || Instant::now() >= deadline
        {
            return Err("Computer setup is busy; retry later".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let current = storage::directory(state, false, true)?
        .metadata()
        .map_err(|_| "Computer state changed")?;
    let original = directory.metadata().map_err(|_| "Computer state changed")?;
    if (current.dev(), current.ino()) != (original.dev(), original.ino()) {
        return Err("Computer state directory was replaced".into());
    }
    Ok(file)
}
pub fn run(
    executable: String,
    state: &Path,
    root: &Path,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    setup(state, root, callback, &mut |operation, callback| {
        drive::run(executable.clone(), operation, callback)
    })
}
fn setup<C, T>(state: &Path, root: &Path, callback: &mut C, cloud: &mut T) -> Result<Value>
where
    C: FnMut(Value) -> i32,
    T: FnMut(Operation, &mut C) -> Result<Value>,
{
    let _lock = lock(state, callback)?;
    let background = state.join("background");
    let identity = jobs::local_identity(root, &background, &[], &jobs::mounts()?)?;
    let list = jobs::transact(&background, &[], jobs::Action::List, callback)?;
    for job in list.as_array().ok_or("Invalid sync job list")? {
        let other = Path::new(text(job, "local_root")?);
        if root.starts_with(other) || other.starts_with(root) {
            return Err("This folder overlaps an existing synchronization job.".into());
        }
    }
    let account = cloud(
        Operation::Info {
            path: "/my-files".into(),
        },
        callback,
    )?;
    if !folder(&account) {
        return Err("The Proton account identity could not be verified.".into());
    }
    let account_uid = text(&account, "uid")?;
    let record_path = state.join("computer.json");
    let saved = storage::read(&record_path, Value::Null, 16384, true)?;
    let computer = if saved.is_null() {
        let name = unique_name()?;
        backend::name(&name)?;
        let raw = cloud(Operation::ComputerCreate { name }, callback)?;
        let computer = computer::record(raw, account_uid)?;
        storage::write(&record_path, computer.clone())?;
        computer
    } else {
        let computer = computer::record(saved, account_uid)?;
        let raw = cloud(
            Operation::ComputerInfo {
                uid: text(&computer, "uid")?.into(),
            },
            callback,
        )?;
        let current = computer::record(raw, account_uid)?;
        if ["uid", "rootFolderUid", "name"]
            .iter()
            .any(|k| computer[*k] != current[*k])
        {
            return Err(
                "Computer identity changed. Review the registered computer before continuing."
                    .into(),
            );
        }
        computer
    };
    let path = text(&computer, "path")?;
    let node = cloud(Operation::Info { path: path.into() }, callback)?;
    if !folder(&node) || node["uid"] != computer["rootFolderUid"] {
        return Err("The computer root could not be verified.".into());
    }
    if jobs::local_identity(root, &background, &[], &jobs::mounts()?)? != identity {
        return Err("The selected local folder was replaced during setup.".into());
    }
    let name = root
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid local folder name")?;
    backend::name(name)?;
    let children = cloud(Operation::List { path: path.into() }, callback)?;
    if children
        .as_array()
        .ok_or("Invalid computer folder list")?
        .iter()
        .any(|n| n["name"] == name)
    {
        return Err(
            "A folder with this name already exists on this computer. No files were merged.".into(),
        );
    }
    let node = cloud(
        Operation::Mkdir {
            parent: path.into(),
            name: name.into(),
        },
        callback,
    )?;
    if !folder(&node) {
        return Err("The new computer folder could not be verified.".into());
    }
    let remote = text(&node, "path")?.to_owned();
    let job = jobs::transact(
        &background,
        &[],
        jobs::Action::Add {
            local_root: root.to_str().ok_or("Unsupported local path")?.into(),
            remote_root: remote.clone(),
            name: name.into(),
            paused: true,
        },
        callback,
    )?;
    if job["local_device"] != json!(identity.0) || job["local_inode"] != json!(identity.1) {
        return Err("The local folder changed; the new job remains paused.".into());
    }
    let id = text(&job, "id")?.to_owned();
    jobs::transact(
        &background,
        &[],
        jobs::Action::Pin {
            id: id.clone(),
            uid: text(&node, "uid")?.into(),
            account_uid: account_uid.into(),
            remote_path: remote,
        },
        callback,
    )?;
    filesystem::poll(callback)?;
    jobs::transact(
        &background,
        &[],
        jobs::Action::Update {
            id,
            name: None,
            paused: Some(false),
        },
        callback,
    )
}
