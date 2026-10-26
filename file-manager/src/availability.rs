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
    downloading: BTreeSet<String>,
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
pub fn status(path: &Path) -> u8 {
    let mut index = INDEX.lock().unwrap();
    if index
        .checked
        .is_none_or(|t| t.elapsed() >= Duration::from_secs(1))
    {
        index.checked = Some(Instant::now());
        index.available.clear();
        index.downloading.clear();
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
                            if owner_alive(&data) {
                                index.downloading = true_paths(&data["downloading"]);
                            }
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
    paths::relative(&index.root, path).map_or(0, |p| {
        if index.downloading.contains(&p) {
            2
        } else if index.available.contains(&p) {
            1
        } else {
            0
        }
    })
}

pub fn available(path: &Path) -> bool {
    status(path) == 1
}
fn true_paths(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, v)| **v == true)
        .map(|(k, _)| k.clone())
        .collect()
}
fn owner_alive(value: &Value) -> bool {
    let Some(pid) = value["owner_pid"]
        .as_u64()
        .filter(|pid| *pid > 1 && *pid <= i32::MAX as u64)
    else {
        return false;
    };
    let Some(expected) = value["owner_start"].as_str().filter(|s| !s.is_empty()) else {
        return false;
    };
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .is_some_and(|s| {
            s.rsplit_once(") ").is_some_and(|(_, fields)| {
                let fields: Vec<_> = fields.split_whitespace().collect();
                fields
                    .first()
                    .is_some_and(|state| !["Z", "X"].contains(state))
                    && fields.get(19).is_some_and(|start| *start == expected)
            })
        })
}
pub fn all_states() -> std::collections::BTreeMap<PathBuf, u8> {
    let _ = status(Path::new("/"));
    let index = INDEX.lock().unwrap();
    index
        .available
        .iter()
        .map(|p| (p, 1))
        .chain(index.downloading.iter().map(|p| (p, 2)))
        .filter(|(p, _)| {
            !p.is_empty()
                && !Path::new(p).is_absolute()
                && Path::new(p)
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
        })
        .map(|(p, state)| (index.root.join(p), state))
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_requires_the_original_live_process() {
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        let start = stat
            .rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap();
        let mut value = serde_json::json!({"owner_pid":std::process::id(),"owner_start":start});
        assert!(owner_alive(&value));
        value["owner_start"] = serde_json::json!("0");
        assert!(!owner_alive(&value));
        assert!(!owner_alive(
            &serde_json::json!({"owner_pid":0,"owner_start":start})
        ));
        assert!(!owner_alive(&serde_json::json!({})));
    }
}
