//! Local status only: never stat or read a file inside the FUSE mount.
use crate::paths;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};
#[derive(Default)]
struct Index {
    checked: Option<Instant>,
    root: PathBuf,
    available: BTreeSet<String>,
}
static INDEX: LazyLock<Mutex<Index>> = LazyLock::new(|| Mutex::new(Index::default()));
fn json(path: &Path, limit: u64) -> Option<Value> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let m = f.metadata().ok()?;
    if !m.is_file() || m.uid() != unsafe { libc::getuid() } || m.len() > limit {
        return None;
    }
    let mut b = Vec::new();
    f.take(limit + 1).read_to_end(&mut b).ok()?;
    if b.len() as u64 > limit {
        return None;
    }
    serde_json::from_slice(&b).ok()
}
pub fn available(path: &Path) -> bool {
    let mut index = INDEX.lock().unwrap();
    if index
        .checked
        .is_none_or(|t| t.elapsed() >= Duration::from_secs(1))
    {
        index.checked = Some(Instant::now());
        index.available.clear();
        index.root = PathBuf::new();
        if let Some(state) = paths::state_directory() {
            if let Some(config) = json(&state.join("ondemand-service/mount.json"), 16384) {
                if let (Some(root), Some(remote)) = (
                    config["mountpoint"].as_str(),
                    config["remote_root"].as_str(),
                ) {
                    index.root = PathBuf::from(root);
                    let key = format!(
                        "{:x}",
                        Sha256::digest(format!("{root}\0{remote}").as_bytes())
                    );
                    if let Some(data) = json(
                        &state
                            .join("ondemand-service/ondemand")
                            .join(key)
                            .join("availability.json"),
                        16 * 1024 * 1024,
                    ) {
                        if data["version"] == 1 {
                            if let Some(entries) = data["available"].as_object() {
                                index.available = entries
                                    .iter()
                                    .filter(|(_, v)| **v == true)
                                    .map(|(k, _)| k.clone())
                                    .collect();
                            }
                        }
                    }
                }
            }
        }
    }
    paths::relative(&index.root, path).is_some_and(|p| index.available.contains(&p))
}

pub fn all_available() -> BTreeSet<PathBuf> {
    let _ = available(Path::new("/"));
    let index = INDEX.lock().unwrap();
    index
        .available
        .iter()
        .filter(|p| {
            !p.is_empty()
                && !Path::new(p).is_absolute()
                && Path::new(p)
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
        })
        .map(|p| index.root.join(p))
        .collect()
}
