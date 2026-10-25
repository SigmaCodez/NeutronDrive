//! Read-only Takeout planning; metadata is passed to the public Photos SDK.
use crate::process::Failure;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, Failure>;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    path: PathBuf,
    size: u64,
    sha1: String,
    capture_time: Option<i64>,
    location: Option<[f64; 2]>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    root: PathBuf,
    items: Vec<Item>,
    pub duplicates: usize,
    pub ignored: usize,
    pub warnings: usize,
    pub videos: usize,
    pub bytes: u64,
}
fn poll(cb: &mut impl FnMut(Value) -> i32) -> Result<()> {
    if cb(json!({"kind":"poll"})) != 0 {
        Err(Failure {
            message: "Import cancelled; completed uploads are kept.".into(),
            cancelled: true,
        })
    } else {
        Ok(())
    }
}
fn open(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| Failure::from("Cannot read Takeout file"))?;
    if !file
        .metadata()
        .map_err(|_| Failure::from("Cannot inspect Takeout file"))?
        .is_file()
    {
        return Err("Not a regular Takeout file".into());
    }
    Ok(file)
}
fn hash(path: &Path, cb: &mut impl FnMut(Value) -> i32) -> Result<(String, u64)> {
    let mut f = open(path)?;
    let before = f
        .metadata()
        .map_err(|_| Failure::from("Cannot inspect Takeout file"))?;
    let mut digest = Sha1::new();
    let mut buf = [0; 131072];
    loop {
        poll(cb)?;
        let n = f
            .read(&mut buf)
            .map_err(|_| Failure::from("Cannot read Takeout file"))?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    let after = f
        .metadata()
        .map_err(|_| Failure::from("Cannot inspect Takeout file"))?;
    if before.len() != after.len()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.mtime() != after.mtime()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err("Takeout file changed during inspection".into());
    }
    Ok((format!("{:x}", digest.finalize()), after.len()))
}
fn media(path: &Path) -> Option<bool> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if [
        "jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "avif", "tif", "tiff", "bmp", "dng",
        "cr2", "cr3", "nef", "arw", "raf", "rw2",
    ]
    .contains(&ext.as_str())
    {
        Some(false)
    } else if [
        "mp4", "mov", "m4v", "avi", "mkv", "webm", "mpg", "mpeg", "3gp", "mts", "m2ts",
    ]
    .contains(&ext.as_str())
    {
        Some(true)
    } else {
        None
    }
}
fn json_file(path: &Path) -> Result<Value> {
    let f = open(path)?;
    if f.metadata()
        .map_err(|_| Failure::from("Cannot inspect metadata"))?
        .len()
        > 1024 * 1024
    {
        return Err("Takeout metadata too large".into());
    }
    serde_json::from_reader(f.take(1024 * 1024 + 1)).map_err(|_| "Invalid Takeout metadata".into())
}
fn metadata(value: &Value) -> (Option<i64>, Option<[f64; 2]>) {
    let time = value["photoTakenTime"]["timestamp"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|n| (-62135596800..=253402300799).contains(n));
    let location = ["geoDataExif", "geoData"].iter().find_map(|key| {
        let lat = value[key]["latitude"].as_f64()?;
        let lon = value[key]["longitude"].as_f64()?;
        (lat.is_finite()
            && lon.is_finite()
            && lat.abs() <= 90.
            && lon.abs() <= 180.
            && (lat != 0. || lon != 0.))
            .then_some([lat, lon])
    });
    (time, location)
}
pub fn scan(root: &Path, cb: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    if fs::symlink_metadata(root)
        .map_err(|_| Failure::from("Choose an extracted Takeout folder"))?
        .file_type()
        .is_symlink()
    {
        return Err("Choose a real folder, not a symbolic link".into());
    }
    let root = root
        .canonicalize()
        .map_err(|_| Failure::from("Cannot open Takeout folder"))?;
    if !root.is_dir() {
        return Err("Extract all Takeout archives before importing".into());
    }
    let mut plan = Plan {
        root: root.clone(),
        items: vec![],
        duplicates: 0,
        ignored: 0,
        warnings: 0,
        videos: 0,
        bytes: 0,
    };
    let mut queue = vec![(root, 0)];
    let mut seen = BTreeMap::<(String, u64), usize>::new();
    let mut examined = 0;
    while let Some((dir, depth)) = queue.pop() {
        poll(cb)?;
        if depth > 64 {
            return Err("Takeout folder nesting exceeds the supported limit".into());
        }
        if dir.canonicalize().ok().as_ref() != Some(&dir) {
            return Err("Takeout directory changed during inspection".into());
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|_| Failure::from("Cannot list Takeout folder"))? {
            poll(cb)?;
            examined += 1;
            if examined > 500000 {
                return Err("Takeout export exceeds 500000 entries; import smaller parts".into());
            }
            entries.push(entry.map_err(|_| Failure::from("Cannot read Takeout entries"))?);
        }
        entries.sort_by_key(|e| e.file_name());
        let mut sidecars = BTreeMap::<String, Vec<(PathBuf, Value)>>::new();
        let mut metadata_bytes = 0u64;
        for entry in &entries {
            poll(cb)?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|_| Failure::from("Cannot inspect Takeout entry"))?;
            if kind.is_file() && path.extension().is_some_and(|s| s == "json") {
                metadata_bytes = metadata_bytes.saturating_add(
                    entry
                        .metadata()
                        .map_err(|_| Failure::from("Cannot inspect metadata"))?
                        .len(),
                );
                if metadata_bytes > 64 * 1024 * 1024 {
                    return Err(
                        "Too much Takeout metadata in one folder; import smaller parts".into(),
                    );
                }
                match json_file(&path) {
                    Ok(v) => {
                        let mut names = Vec::new();
                        if let Some(title) = v["title"]
                            .as_str()
                            .filter(|t| !t.contains('/') && !t.contains('\\'))
                        {
                            names.push(title.to_string());
                        }
                        if v["photoTakenTime"].is_object() {
                            if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                                if let Some(stem) = filename
                                    .strip_suffix(".supplemental-metadata.json")
                                    .or_else(|| filename.strip_suffix(".json"))
                                {
                                    if !names.iter().any(|n| n == stem) {
                                        names.push(stem.into());
                                    }
                                }
                            }
                        }
                        for name in names {
                            sidecars
                                .entry(name)
                                .or_default()
                                .push((path.clone(), v.clone()));
                        }
                    }
                    Err(_) => plan.warnings += 1,
                }
            }
        }
        for entry in entries {
            poll(cb)?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|_| Failure::from("Cannot inspect Takeout entry"))?;
            if kind.is_dir() {
                queue.push((path, depth + 1));
                continue;
            }
            if !kind.is_file() {
                plan.ignored += 1;
                continue;
            }
            let Some(video) = media(&path) else {
                if path.extension().is_none_or(|s| s != "json") {
                    plan.ignored += 1;
                }
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let candidates = sidecars.get(&name);
            let exact = [
                format!("{name}.json"),
                format!("{name}.supplemental-metadata.json"),
            ];
            let sidecar = candidates.and_then(|xs| {
                let direct: Vec<_> = xs
                    .iter()
                    .filter(|(p, _)| {
                        p.file_name()
                            .is_some_and(|n| exact.iter().any(|x| n == x.as_str()))
                    })
                    .collect();
                if direct.len() == 1 {
                    Some(&direct[0].1)
                } else if xs.len() == 1 {
                    Some(&xs[0].1)
                } else {
                    None
                }
            });
            if sidecar.is_none() {
                plan.warnings += 1;
            }
            let (capture_time, location) = sidecar.map(metadata).unwrap_or_default();
            let (sha1, size) = hash(&path, cb)?;
            if let Some(index) = seen.get(&(sha1.clone(), size)) {
                let old = &mut plan.items[*index];
                if old.capture_time.is_none() {
                    old.capture_time = capture_time;
                }
                if old.location.is_none() {
                    old.location = location;
                }
                plan.duplicates += 1;
                continue;
            }
            seen.insert((sha1.clone(), size), plan.items.len());
            plan.bytes = plan
                .bytes
                .checked_add(size)
                .ok_or("Takeout size overflow")?;
            plan.videos += usize::from(video);
            plan.items.push(Item {
                path,
                size,
                sha1,
                capture_time,
                location,
            });
            if plan.items.len() > 100000 {
                return Err("Import at most 100000 media files at a time".into());
            }
        }
    }
    Ok(json!(plan))
}
pub fn import(
    plan: Plan,
    cb: &mut impl FnMut(Value) -> i32,
    mut upload: impl FnMut(Value, &mut dyn FnMut(Value) -> i32) -> Result<Value>,
) -> Result<Value> {
    if plan.items.len() > 100000 {
        return Err("Invalid Takeout plan".into());
    }
    let mut uploaded = 0;
    let mut skipped = plan.duplicates;
    let mut failed = 0;
    let mut error = String::new();
    let mut root_uid = String::new();
    for item in &plan.items {
        poll(cb)?;
        if !item.path.starts_with(&plan.root)
            || item.path.canonicalize().ok().as_ref() != Some(&item.path)
        {
            return Err("Takeout source changed; scan again".into());
        }
        let (hash, size) = hash(&item.path, cb)?;
        if hash != item.sha1 || size != item.size {
            return Err("Takeout file changed; scan again".into());
        }
        let reply = upload(
            json!({"path":item.path,"sha1":item.sha1,"captureTime":item.capture_time,"location":item.location,"expectedRoot":root_uid}),
            cb,
        );
        match reply {
            Ok(value) => {
                let root = value["root"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or("Invalid photo import response")?;
                if !root_uid.is_empty() && root != root_uid {
                    return Err("Photo account changed; import stopped".into());
                }
                root_uid = root.into();
                match value["status"].as_str() {
                    Some("uploaded") => uploaded += 1,
                    Some("skipped") => skipped += 1,
                    _ => return Err("Invalid photo import response".into()),
                }
            }
            Err(e) if e.cancelled => return Err(e),
            Err(e) => {
                error = e.message;
                failed += 1;
                break;
            }
        }
    }
    Ok(
        json!({"uploaded":uploaded,"skipped":skipped,"failed":failed,"error":error,"remaining":plan.items.len().saturating_sub(uploaded+skipped-plan.duplicates+failed)}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_checks_ranges() {
        let (t, l) = metadata(
            &json!({"photoTakenTime":{"timestamp":"123"},"geoData":{"latitude":47.,"longitude":8.}}),
        );
        assert_eq!(t, Some(123));
        assert_eq!(l, Some([47., 8.]));
        assert_eq!(
            metadata(&json!({"geoData":{"latitude":999.,"longitude":8.}})).1,
            None
        );
    }
    #[test]
    fn scan_duplicates_sidecars_and_changed_source() {
        let root = std::env::temp_dir().join(format!("nd-takeout-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("a.jpg"), b"image").unwrap();
        fs::write(root.join("b.mp4"), b"video").unwrap();
        fs::write(root.join("copy.jpg"), b"image").unwrap();
        fs::write(
            root.join("a.jpg.supplemental-metadata.json"),
            br#"{"title":"a.jpg","photoTakenTime":{"timestamp":"123"}}"#,
        )
        .unwrap();
        std::os::unix::fs::symlink("/etc/passwd", root.join("unsafe.jpg")).unwrap();
        let value = scan(&root, &mut |_| 0).unwrap();
        let plan: Plan = serde_json::from_value(value).unwrap();
        assert_eq!(plan.items.len(), 2);
        assert_eq!(plan.duplicates, 1);
        assert_eq!(plan.ignored, 1);
        assert_eq!(plan.items[0].capture_time, Some(123));
        let result = import(plan.clone(), &mut |_| 0, |_, _| {
            Ok(json!({"root":"r","status":"uploaded"}))
        })
        .unwrap();
        assert_eq!(result["uploaded"], 2);
        let partial = import(plan.clone(), &mut |_| 0, |_, _| {
            Err("Synthetic upload failure".into())
        })
        .unwrap();
        assert_eq!(partial["failed"], 1);
        assert_eq!(partial["remaining"], 1);
        assert_eq!(partial["error"], "Synthetic upload failure");
        assert!(
            import(plan.clone(), &mut |_| 1, |_, _| panic!("cancelled upload"))
                .unwrap_err()
                .cancelled
        );
        fs::write(root.join("a.jpg"), b"changed").unwrap();
        assert!(import(plan, &mut |_| 0, |_, _| panic!(
            "changed source must not upload"
        ))
        .is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
