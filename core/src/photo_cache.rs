//! Private, bounded thumbnail cache. Metadata and access are verified before reuse.
use crate::{process::Failure, storage};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
const LIMIT: u64 = 512 * 1024 * 1024;
const AGE: u64 = 30 * 24 * 3600;
const ENTRY_LIMIT: usize = 720000;
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn root() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
        .filter(|p| p.is_absolute())
        .map(|p| p.join("neutrondrive/photo-thumbnails-v1"))
}
fn key(account: &str, photo: &Value) -> Option<String> {
    let uid = photo["uid"].as_str().filter(|s| !s.is_empty())?;
    let revision = photo["revision"].as_str().filter(|s| !s.is_empty())?;
    Some(format!(
        "{:x}.json",
        Sha256::digest(serde_json::to_vec(&(account, uid, revision)).ok()?)
    ))
}
fn thumbnail(v: &Value) -> Option<&str> {
    v.as_str()
        .filter(|s| s.starts_with("data:image/jpeg;base64,/9") && s.len() <= 710000)
}
fn cache_file(name: &str) -> bool {
    name.len() == 69 && name.ends_with(".json") && name[..64].bytes().all(|b| b.is_ascii_hexdigit())
}
fn trim(root: &Path, limit: u64) {
    if storage::directory(root, false, true).is_err() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        if !cache_file(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let Ok(meta) = entry.path().symlink_metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(UNIX_EPOCH);
        if now().saturating_sub(
            modified
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        ) > AGE
        {
            let _ = std::fs::remove_file(entry.path());
        } else {
            files.push((modified, meta.len(), entry.path()));
        }
    }
    files.sort_by_key(|(time, _, _)| *time);
    let mut size: u64 = files.iter().map(|(_, size, _)| size).sum();
    for (_, bytes, path) in files {
        if size <= limit {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            size = size.saturating_sub(bytes);
        }
    }
}
pub fn clear() {
    if let Some(root) = root() {
        trim(&root, 0)
    }
}
/// `call` stays on the worker thread. Cache failures never prevent photo access.
pub fn details(
    request: Value,
    root: Option<&Path>,
    call: &mut impl FnMut(Value) -> Result<Value, Failure>,
) -> Result<Value, Failure> {
    let ids = request["ids"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 48)
        .ok_or_else(|| Failure::from("Invalid photo batch"))?;
    let ids = ids.clone();
    // Recheck authentication/account on every batch, including after an external CLI login.
    let account = match call(json!({"action":"account_storage"})) {
        Ok(value) => value,
        Err(error) if error.cancelled => return Err(error),
        Err(_) => Value::Null, // Quota/account-label failures must not break Photos.
    };
    let account = account["account"]["email"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    let mut metadata = request.clone();
    metadata["thumbnails"] = json!(false);
    let mut result = call(json!({"action":"photo_library","request":metadata}))?;
    let photos = result["photos"]
        .as_array_mut()
        .filter(|a| a.len() <= 48)
        .ok_or_else(|| Failure::from("Invalid photo metadata"))?;
    if photos.iter().any(|p| !ids.contains(&p["uid"])) {
        return Err("Invalid photo identity".into());
    }
    let root = root.filter(|p| !account.is_empty() && storage::directory(p, true, true).is_ok());
    let mut missing = Vec::new();
    for photo in photos.iter_mut() {
        photo["thumbnail"] = json!("");
        let cached = root.and_then(|root| {
            key(account, photo).and_then(|key| {
                storage::read_optional(&root.join(key), ENTRY_LIMIT, true)
                    .ok()
                    .flatten()
            })
        });
        if let Some(cached) = cached.filter(|v| {
            v["saved"]
                .as_u64()
                .is_some_and(|t| t <= now() && now() - t <= AGE)
        }) {
            if let Some(image) = thumbnail(&cached["thumbnail"]) {
                photo["thumbnail"] = json!(image);
                continue;
            }
        }
        missing.push(photo["uid"].clone());
    }
    if !missing.is_empty() {
        let fresh =
            call(json!({"action":"photo_library","request":{"action":"details","ids":missing}}))?;
        let fresh = fresh["photos"]
            .as_array()
            .filter(|p| p.len() <= 48)
            .ok_or_else(|| Failure::from("Invalid photo thumbnails"))?;
        for photo in photos.iter_mut() {
            if let Some(new) = fresh
                .iter()
                .find(|p| p["uid"] == photo["uid"] && missing.contains(&p["uid"]))
            {
                // A photo may have changed between metadata and thumbnail reads.
                *photo = new.clone();
                photo["thumbnail"] = json!(thumbnail(&new["thumbnail"]).unwrap_or(""));
                if let (Some(root), Some(key), Some(image)) =
                    (root, key(account, photo), thumbnail(&photo["thumbnail"]))
                {
                    let _ =
                        storage::write(&root.join(key), json!({"saved":now(),"thumbnail":image}));
                }
            }
        }
    }
    if let Some(root) = root {
        trim(root, LIMIT);
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "nd-photo-cache-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn load(root: &Path, account: &str, revision: &str, downloads: &mut usize) -> Value {
        details(
            json!({"action":"details","ids":["p"]}),
            Some(root),
            &mut |r| {
                if r["action"] == "account_storage" {
                    return Ok(json!({"account":{"email":account}}));
                }
                let image = if r["request"]["thumbnails"] == false {
                    ""
                } else {
                    *downloads += 1;
                    "data:image/jpeg;base64,/9g="
                };
                Ok(json!({"photos":[{"uid":"p","revision":revision,"thumbnail":image}]}))
            },
        )
        .unwrap()
    }
    #[test]
    fn survives_reopening_and_separates_accounts_and_revisions() {
        let temp = Temp::new();
        let mut downloads = 0;
        let first = load(&temp.0, "a", "r1", &mut downloads);
        assert_eq!(load(&temp.0, "a", "r1", &mut downloads), first);
        assert_eq!(downloads, 1);
        load(&temp.0, "b", "r1", &mut downloads);
        load(&temp.0, "a", "r2", &mut downloads);
        assert_eq!(downloads, 3);
        trim(&temp.0, 0);
        load(&temp.0, "a", "r1", &mut downloads);
        assert_eq!(downloads, 4);
    }
    #[test]
    fn corruption_expiry_and_missing_revision_are_cache_misses() {
        let temp = Temp::new();
        let mut downloads = 0;
        load(&temp.0, "a", "r1", &mut downloads);
        let path = temp
            .0
            .join(key("a", &json!({"uid":"p","revision":"r1"})).unwrap());
        storage::write(
            &path,
            json!({"saved":0,"thumbnail":"data:image/jpeg;base64,/9g="}),
        )
        .unwrap();
        load(&temp.0, "a", "r1", &mut downloads);
        assert_eq!(downloads, 2);
        std::fs::write(&path, "broken").unwrap();
        load(&temp.0, "a", "r1", &mut downloads);
        assert_eq!(downloads, 3);
        load(&temp.0, "a", "", &mut downloads);
        load(&temp.0, "a", "", &mut downloads);
        assert_eq!(downloads, 5);
    }
    #[test]
    fn denied_access_never_returns_cached_photos() {
        let temp = Temp::new();
        load(&temp.0, "a", "r1", &mut 0);
        let result = details(
            json!({"action":"details","ids":["p"]}),
            Some(&temp.0),
            &mut |_| Err("Access denied".into()),
        );
        assert!(result.is_err());
    }
}
