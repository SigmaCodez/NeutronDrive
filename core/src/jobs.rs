//! Locked job transactions. Existing version-1 JSON remains readable.
use crate::{process::Failure, storage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, Failure>;
use libc::{flock, getuid, LOCK_EX, LOCK_NB, O_NOFOLLOW, O_NONBLOCK};

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Job {
    id: String,
    name: String,
    local_root: String,
    remote_root: String,
    #[serde(default)]
    paused: bool,
    #[serde(default)]
    remote_uid: String,
    #[serde(default)]
    account_root_uid: String,
    #[serde(default)]
    local_device: u64,
    #[serde(default)]
    local_inode: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    version: u32,
    jobs: Vec<Job>,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    List,
    Add {
        local_root: String,
        remote_root: String,
        name: String,
        paused: bool,
    },
    Update {
        id: String,
        name: Option<String>,
        paused: Option<bool>,
    },
    Remove {
        id: String,
    },
    Pin {
        id: String,
        uid: String,
        account_uid: String,
        remote_path: String,
    },
}

fn overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
fn valid_remote(path: &str) -> bool {
    crate::computer::boundary(path).is_ok()
}

fn validate(jobs: &[Job]) -> Result<()> {
    if jobs.len() > 64 || jobs.iter().map(|j| &j.id).collect::<BTreeSet<_>>().len() != jobs.len() {
        return Err("Zu viele oder doppelte Sync-Jobs.".into());
    }
    for (i, job) in jobs.iter().enumerate() {
        if job.id.len() != 32
            || !job.id.bytes().all(|c| b"0123456789abcdef".contains(&c))
            || !Path::new(&job.local_root).is_absolute()
            || job.local_root.contains('\0')
            || job.local_root.split('/').any(|p| matches!(p, "." | ".."))
            || !valid_remote(&job.remote_root)
        {
            return Err("Ungültige Sync-Jobkonfiguration; keine automatische Reparatur.".into());
        }
        for other in &jobs[i + 1..] {
            if overlap(Path::new(&job.local_root), Path::new(&other.local_root)) {
                return Err("Lokale Sync-Ordner dürfen sich nicht überschneiden.".into());
            }
            if overlap(Path::new(&job.remote_root), Path::new(&other.remote_root)) {
                return Err("Remote-Sync-Ordner dürfen sich nicht überschneiden.".into());
            }
        }
    }
    Ok(())
}

pub fn mounts() -> Result<Vec<(String, String)>> {
    let text = std::fs::read_to_string("/proc/self/mountinfo")
        .map_err(|_| "Linux-Mountinformationen sind nicht verfügbar.")?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let mut path = left.split_whitespace().nth(4)?.to_string();
            for (escaped, literal) in [
                ("\\040", " "),
                ("\\011", "\t"),
                ("\\012", "\n"),
                ("\\134", "\\"),
            ] {
                path = path.replace(escaped, literal);
            }
            Some((path, right.split_whitespace().next()?.into()))
        })
        .collect())
}

pub fn local_identity(
    root: &Path,
    state: &Path,
    planned: &[String],
    mounted: &[(String, String)],
) -> Result<(u64, u64)> {
    if overlap(root, state) {
        return Err(
            "Sync-Ordner und privates Dienstverzeichnis dürfen sich nicht überschneiden.".into(),
        );
    }
    if planned.iter().any(|p| overlap(root, Path::new(p))) {
        return Err(
            "Ein On-demand-Mount darf kein Hintergrund-Sync-Ordner sein oder darin liegen.".into(),
        );
    }
    for (path, kind) in mounted {
        let path = Path::new(path);
        if kind == "autofs" || kind.starts_with("fuse") {
            if overlap(root, path) {
                return Err("FUSE-/On-demand-/Autofs-Dateisysteme sind vom Hintergrund-Sync ausgeschlossen.".into());
            }
        } else if path != root && path.starts_with(root) {
            return Err("Der Sync-Ordner enthält einen weiteren Mount; bitte einen einzelnen Ordner wählen.".into());
        }
    }
    let directory = storage::directory(root, false, false)?;
    let info = directory
        .metadata()
        .map_err(|_| "Lokaler Sync-Ordner ist nicht sicher zugänglich.")?;
    Ok((info.dev(), info.ino()))
}

pub fn transact(
    state: &Path,
    planned: &[String],
    action: Action,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    let directory = storage::directory(state, true, true)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(O_NOFOLLOW | O_NONBLOCK)
        .open(state.join("jobs.lock"))
        .map_err(|_| "Sync-Job-Sperre ist nicht zugänglich.")?;
    let info = lock.metadata().map_err(|_| "Ungültige Sync-Job-Sperre.")?;
    if !info.is_file() || info.uid() != unsafe { getuid() } || info.nlink() != 1 {
        return Err("Ungültige Sync-Job-Sperre.".into());
    }
    let started = Instant::now();
    loop {
        if callback(json!({"kind":"poll"})) != 0 {
            return Err("Sync-Job-Auftrag abgebrochen.".into());
        }
        if unsafe { flock(lock.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0 {
            break;
        }
        let error = std::io::Error::last_os_error();
        if !matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
        ) || started.elapsed() > Duration::from_secs(5)
        {
            return Err("Sync-Jobkonfiguration ist noch beschäftigt.".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Reject a state directory replaced while the process waited for the lock.
    let current = storage::directory(state, false, true)?;
    let a = directory
        .metadata()
        .map_err(|_| "Dienstverzeichnis wurde verändert.")?;
    let b = current
        .metadata()
        .map_err(|_| "Dienstverzeichnis wurde verändert.")?;
    if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
        return Err("Dienstverzeichnis wurde verändert.".into());
    }
    let path = state.join("jobs.json");
    let config: Config = serde_json::from_value(storage::read(
        &path,
        json!({"version":1,"jobs":[]}),
        8 * 1024 * 1024,
        false,
    )?)
    .map_err(|_| "Ungültige Sync-Jobkonfiguration; keine automatische Reparatur.")?;
    if config.version != 1 {
        return Err("Ungültige Version der Sync-Konfiguration.".into());
    }
    let mut jobs = config.jobs;
    validate(&jobs)?;
    let result = match action {
        Action::List => return Ok(json!(jobs)),
        Action::Add {
            local_root,
            remote_root,
            name,
            paused,
        } => {
            if !valid_remote(&remote_root) {
                return Err("Ungültiger Remote-Sync-Pfad.".into());
            }
            let (local_device, local_inode) =
                local_identity(Path::new(&local_root), state, planned, &mounts()?)?;
            let mut bytes = [0u8; 16];
            File::open("/dev/urandom")
                .and_then(|mut f| f.read_exact(&mut bytes))
                .map_err(|_| "Sync-Job-ID konnte nicht erzeugt werden.")?;
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            let id = bytes.iter().map(|b| format!("{b:02x}")).collect();
            let title = if name.trim().is_empty() {
                Path::new(&local_root)
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("Sync")
            } else {
                name.trim()
            };
            let job = Job {
                id,
                name: title.chars().take(120).collect(),
                local_root,
                remote_root,
                paused,
                remote_uid: String::new(),
                account_root_uid: String::new(),
                local_device,
                local_inode,
            };
            jobs.push(job.clone());
            json!(job)
        }
        Action::Update { id, name, paused } => {
            let job = jobs
                .iter_mut()
                .find(|j| j.id == id)
                .ok_or("Sync-Job wurde nicht gefunden.")?;
            if let Some(paused) = paused {
                job.paused = paused;
            }
            if let Some(name) = name {
                if !name.trim().is_empty() {
                    job.name = name.trim().chars().take(120).collect();
                }
            }
            json!(job)
        }
        Action::Remove { id } => {
            let count = jobs.len();
            jobs.retain(|j| j.id != id);
            if count == jobs.len() {
                return Err("Sync-Job wurde nicht gefunden.".into());
            }
            Value::Null
        }
        Action::Pin {
            id,
            uid,
            account_uid,
            remote_path,
        } => {
            let job = jobs
                .iter_mut()
                .find(|j| j.id == id)
                .ok_or("Sync-Job wurde während der Vorbereitung entfernt.")?;
            if (!job.remote_uid.is_empty() && job.remote_uid != uid)
                || (!job.account_root_uid.is_empty() && job.account_root_uid != account_uid)
            {
                return Err(
                    "Proton-Konto oder Remote-Wurzel wurde geändert; Job bewusst neu anlegen."
                        .into(),
                );
            }
            let previous = job.clone();
            job.remote_uid = uid;
            job.account_root_uid = account_uid;
            job.remote_root = remote_path;
            if *job == previous {
                return Ok(json!(job));
            }
            json!(job)
        }
    };
    validate(&jobs)?;
    storage::write(&path, json!({"version":1,"jobs":jobs}))?;
    Ok(result)
}
