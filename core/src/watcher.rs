//! Owned inotify descriptors and recursive registration. Opaque numeric handles
//! are checked under a registry lock; active calls retain Arc ownership.
use crate::{inotify, jobs, process::Failure, storage};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ffi::{CString, OsString};
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use libc::{inotify_add_watch, inotify_init1, poll, pollfd, O_CLOEXEC, O_NONBLOCK, POLLIN};
type Result<T> = std::result::Result<T, Failure>;
const MASK: u32 = 8 | 4 | 64 | 128 | 256 | 512 | 1024 | 2048 | 0x02000000 | 0x01000000;
static NEXT: AtomicU64 = AtomicU64::new(1);
static WATCHERS: LazyLock<Mutex<BTreeMap<u64, Arc<Watcher>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

struct Watcher {
    state: Mutex<State>,
    stopping: AtomicBool,
    running: AtomicBool,
}
struct State {
    fd: File,
    root: PathBuf,
    paths: BTreeMap<i32, PathBuf>,
}

fn send(
    callback: &mut impl FnMut(Value) -> i32,
    relative: Option<&Path>,
    reason: &str,
) -> Result<()> {
    if callback(
        json!({"kind":"watch","relative":relative.map(|p| p.as_os_str().as_bytes()),"reason":reason}),
    ) != 0
    {
        return Err("Dateiüberwachung angehalten.".into());
    }
    Ok(())
}

impl State {
    fn add_tree(&mut self, root: &Path) -> Result<()> {
        let mounts = jobs::mounts()?;
        if mounts.iter().any(|(path, kind)| {
            (root.starts_with(path) || Path::new(path).starts_with(root))
                && (kind.starts_with("fuse") || kind == "autofs")
        }) {
            return Err("Ein FUSE-/On-demand-Mount wurde im überwachten Baum gefunden.".into());
        }
        let nested: Vec<PathBuf> = mounts
            .iter()
            .map(|(p, _)| PathBuf::from(p))
            .filter(|p| p != &self.root && p.starts_with(&self.root))
            .collect();
        if nested.iter().any(|p| root.starts_with(p)) {
            return Err("Ein neuer Mount liegt im überwachten Baum.".into());
        }
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            let info = match std::fs::symlink_metadata(&path) {
                Ok(info) => info,
                Err(_) => continue,
            };
            if !info.is_dir() || info.file_type().is_symlink() {
                continue;
            }
            // Reject links in ancestors before registering this directory.
            let directory = storage::directory(&path, false, false)?;
            let pinned = format!("/proc/self/fd/{}/.", directory.as_raw_fd());
            let name =
                CString::new(pinned.as_bytes()).map_err(|_| "Ungültiger Überwachungspfad.")?;
            let watch = unsafe { inotify_add_watch(self.fd.as_raw_fd(), name.as_ptr(), MASK) };
            if watch < 0 {
                return Err(
                    "inotify-Überwachung fehlgeschlagen; Watch-Limit oder Ordnerrechte prüfen."
                        .into(),
                );
            }
            self.paths.insert(watch, path.clone());
            for item in std::fs::read_dir(&pinned)
                .map_err(|_| "Überwachter Ordner konnte nicht gelesen werden.")?
            {
                let item = item.map_err(|_| "Überwachter Ordner konnte nicht gelesen werden.")?;
                if !item.file_name().as_bytes().starts_with(b".neutrondrive-")
                    && !nested.contains(&path.join(item.file_name()))
                    && item
                        .file_type()
                        .map_err(|_| "Überwachter Ordner wurde verändert.")?
                        .is_dir()
                {
                    pending.push(path.join(item.file_name()));
                }
            }
        }
        Ok(())
    }

    fn consume(&mut self, bytes: &[u8], callback: &mut impl FnMut(Value) -> i32) -> Result<()> {
        // Decode the entire batch before altering registrations or delivering.
        let events = inotify::decode(bytes)?;
        for event in events.as_array().ok_or("Ungültige inotify-Ereignisse.")? {
            if event["kind"] == "overflow" {
                send(callback, None, "inotify-overflow")?;
                continue;
            }
            let watch = event["watch"].as_i64().ok_or("Ungültige Watch-ID.")? as i32;
            let Some(parent) = self.paths.get(&watch).cloned() else {
                continue;
            };
            if event["kind"] == "ignored" {
                self.paths.remove(&watch);
                continue;
            }
            let name: Vec<u8> = serde_json::from_value(event["name"].clone())
                .map_err(|_| "Ungültiger inotify-Name.")?;
            let path = if name.is_empty() {
                parent
            } else {
                parent.join(OsString::from_vec(name))
            };
            if event["add_tree"] == true && self.add_tree(&path).is_err() {
                send(callback, None, "watch-error")?;
            }
            if event["rebuild"] == true {
                send(callback, None, "watch-rebuild")?;
            }
            match path.strip_prefix(&self.root) {
                Ok(relative) => send(
                    callback,
                    if relative.as_os_str().is_empty() {
                        None
                    } else {
                        Some(relative)
                    },
                    "local",
                )?,
                Err(_) => send(callback, None, "watch-rebuild")?,
            }
        }
        Ok(())
    }
}

pub fn create(root: Vec<u8>) -> Result<u64> {
    let root = PathBuf::from(OsString::from_vec(root));
    let _directory = storage::directory(&root, false, false)?;
    let fd = unsafe { inotify_init1(O_NONBLOCK | O_CLOEXEC) };
    if fd < 0 {
        return Err("inotify konnte nicht gestartet werden.".into());
    }
    let mut state = State {
        fd: unsafe { File::from_raw_fd(fd) },
        root: root.clone(),
        paths: BTreeMap::new(),
    };
    state.add_tree(&root)?;
    if state.paths.is_empty() {
        return Err("Der überwachte Ordner ist nicht mehr vorhanden.".into());
    }
    let mut registry = WATCHERS
        .lock()
        .map_err(|_| "inotify-Registry ist nicht verfügbar.")?;
    if registry.len() >= 256 {
        return Err("Zu viele aktive Dateiüberwachungen.".into());
    }
    let id = NEXT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| "Watch-ID-Bereich erschöpft.")?;
    registry.insert(
        id,
        Arc::new(Watcher {
            state: Mutex::new(state),
            stopping: AtomicBool::new(false),
            running: AtomicBool::new(false),
        }),
    );
    Ok(id)
}

pub fn stop(id: u64) -> Result<()> {
    let watcher = WATCHERS
        .lock()
        .map_err(|_| "inotify-Registry ist nicht verfügbar.")?
        .remove(&id);
    if let Some(watcher) = watcher {
        watcher.stopping.store(true, Ordering::Release);
    }
    Ok(())
}

pub fn run(id: u64, callback: &mut impl FnMut(Value) -> i32) -> Result<()> {
    let watcher = WATCHERS
        .lock()
        .map_err(|_| "inotify-Registry ist nicht verfügbar.")?
        .get(&id)
        .cloned()
        .ok_or("Unbekannte Dateiüberwachung.")?;
    if watcher.running.swap(true, Ordering::AcqRel) {
        return Err("Dateiüberwachung läuft bereits.".into());
    }
    let result = (|| {
        let mut state = watcher
            .state
            .lock()
            .map_err(|_| "Dateiüberwachung ist nicht verfügbar.")?;
        let mut bytes = vec![0u8; 1024 * 1024];
        while !watcher.stopping.load(Ordering::Acquire) {
            if callback(json!({"kind":"poll"})) != 0 {
                break;
            }
            match state.fd.read(&mut bytes) {
                Ok(0) => return Err("inotify hat den Ereignisstrom geschlossen.".into()),
                Ok(count) => state.consume(&bytes[..count], callback)?,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    let mut descriptor = pollfd {
                        fd: state.fd.as_raw_fd(),
                        events: POLLIN,
                        revents: 0,
                    };
                    if unsafe { poll(&mut descriptor, 1, 100) } < 0
                        && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
                    {
                        return Err("inotify-Warten fehlgeschlagen.".into());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => (),
                Err(_) => return Err("inotify-Ereignisse konnten nicht gelesen werden.".into()),
            }
        }
        Ok(())
    })();
    watcher.stopping.store(true, Ordering::Release);
    let _ = stop(id);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> State {
        State {
            fd: File::open("/dev/null").unwrap(),
            root: PathBuf::from("/synthetic"),
            paths: BTreeMap::from([(4, PathBuf::from("/synthetic"))]),
        }
    }
    fn event(watch: i32, mask: u32, name: &[u8]) -> Vec<u8> {
        [
            watch.to_ne_bytes().to_vec(),
            mask.to_ne_bytes().to_vec(),
            0u32.to_ne_bytes().to_vec(),
            (name.len() as u32).to_ne_bytes().to_vec(),
            name.to_vec(),
        ]
        .concat()
    }
    #[test]
    fn malformed_read_has_no_partial_delivery_or_registration() {
        let mut watcher = state();
        let mut bytes = event(4, 0x40000100, b"directory\0");
        bytes.push(0);
        assert!(watcher
            .consume(&bytes, &mut |_| panic!("partial delivery"))
            .is_err());
        assert_eq!(watcher.paths.len(), 1);
    }
    #[test]
    fn byte_names_overflow_and_removal() {
        let mut watcher = state();
        let mut deliveries = Vec::new();
        let bytes = [
            event(4, 8, b"bad-\xff\0"),
            event(-1, 0x4000, b""),
            event(4, 0x8000, b""),
            event(4, 8, b"later\0"),
        ]
        .concat();
        watcher
            .consume(&bytes, &mut |e| {
                deliveries.push(e);
                0
            })
            .unwrap();
        assert_eq!(deliveries.len(), 2);
        assert_eq!(deliveries[0]["relative"], json!(b"bad-\xff".as_slice()));
        assert_eq!(deliveries[1]["reason"], "inotify-overflow");
        assert!(watcher.paths.is_empty());
    }
}
