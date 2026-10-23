//! Same-filesystem trash selection, with no symlink traversal or copy-delete fallback.
use crate::{process::Failure, storage};
use serde_json::{json, Value};
use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, Failure>;

fn mount_top(path: &Path, table: &str) -> Result<PathBuf> {
    let mut best = None;
    for line in table.lines() {
        let Some(encoded) = line.split_whitespace().nth(4) else {
            continue;
        };
        let bytes = encoded.as_bytes();
        let mut decoded = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\'
                && i + 3 < bytes.len()
                && bytes[i + 1..i + 4]
                    .iter()
                    .all(|c| (b'0'..=b'7').contains(c))
            {
                let octal = (bytes[i + 1] - b'0') as u16 * 64
                    + (bytes[i + 2] - b'0') as u16 * 8
                    + (bytes[i + 3] - b'0') as u16;
                decoded.push(u8::try_from(octal).map_err(|_| "Invalid mount path escape")?);
                i += 4;
            } else {
                decoded.push(bytes[i]);
                i += 1;
            }
        }
        use std::os::unix::ffi::OsStringExt;
        let mount = PathBuf::from(std::ffi::OsString::from_vec(decoded));
        if mount.is_absolute()
            && path.starts_with(&mount)
            && best
                .as_ref()
                .is_none_or(|b: &PathBuf| mount.components().count() > b.components().count())
        {
            best = Some(mount);
        }
    }
    best.ok_or_else(|| "Cannot determine filesystem mount point".into())
}
fn candidate(
    path: PathBuf,
    root: &Path,
    device: u64,
    backend: &str,
    top: Option<&Path>,
) -> Result<Value> {
    if path.starts_with(root) {
        return Err("Trash must be outside the synchronized folder".into());
    }
    for directory in [path.clone(), path.join("files"), path.join("info")] {
        let fd = storage::directory(&directory, true, true)?;
        if fd
            .metadata()
            .map_err(|e| Failure::from(e.to_string()))?
            .dev()
            != device
        {
            return Err("Trash is on a different filesystem; copy-delete is disabled".into());
        }
    }
    Ok(json!({"path":path, "backend":backend, "top":top}))
}
pub fn choose(root: &str, data_home: &str, device: u64, desktop: bool) -> Result<Value> {
    let table = if desktop {
        std::fs::read_to_string("/proc/self/mountinfo").map_err(|e| Failure::from(e.to_string()))?
    } else {
        String::new()
    };
    choose_with_table(
        Path::new(root),
        Path::new(data_home),
        device,
        desktop,
        &table,
    )
}
fn choose_with_table(
    root: &Path,
    home: &Path,
    device: u64,
    desktop: bool,
    table: &str,
) -> Result<Value> {
    // Validate lexical paths without requiring the XDG directory to exist yet.
    for path in [root, home] {
        if !path.is_absolute()
            || path
                .as_os_str()
                .as_encoded_bytes()
                .split(|c| *c == b'/')
                .any(|s| s == b"." || s == b"..")
        {
            return Err("An absolute path without dot components is required".into());
        }
    }
    let uid = unsafe { libc::getuid() };
    if desktop {
        let top = mount_top(root, table)?;
        let home_top = mount_top(home, table)?;
        if top == home_top {
            if let Ok(value) = candidate(home.join("Trash"), root, device, "desktop", None) {
                return Ok(value);
            }
        } else if !top.join(format!(".Trash-{uid}")).starts_with(root) {
            let shared = top.join(".Trash");
            if storage::directory(&shared, false, false)
                .and_then(|f| f.metadata().map_err(|e| Failure::from(e.to_string())))
                .is_ok_and(|m| m.mode() & libc::S_ISVTX != 0)
            {
                if let Ok(value) = candidate(
                    shared.join(uid.to_string()),
                    root,
                    device,
                    "desktop",
                    Some(&top),
                ) {
                    return Ok(value);
                }
            }
            if let Ok(value) = candidate(
                top.join(format!(".Trash-{uid}")),
                root,
                device,
                "desktop",
                Some(&top),
            ) {
                return Ok(value);
            }
        }
    }
    candidate(
        root.parent()
            .ok_or("No safe trash location outside root")?
            .join(format!(".neutrondrive-trash-{uid}")),
        root,
        device,
        "managed",
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::{symlink, PermissionsExt},
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    #[test]
    fn external_mount_selection_and_symlink_fallback() {
        let base = std::env::temp_dir().join(format!(
            "nd-trash-location-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = base.join("sync");
        let fd = storage::directory(&root, true, true).unwrap();
        let device = fd.metadata().unwrap().dev();
        let table = format!(
            "1 0 0:1 / / rw - ext4 none rw\n2 1 0:2 / {} rw - ext4 none rw",
            base.display()
        );
        let choose = || {
            choose_with_table(&root, Path::new("/different-home"), device, true, &table).unwrap()
        };
        let uid = unsafe { libc::getuid() };
        assert_eq!(choose()["path"], json!(base.join(format!(".Trash-{uid}"))));
        let outside = base.join("outside");
        std::fs::create_dir(&outside).unwrap();
        symlink(&outside, base.join(".Trash")).unwrap();
        assert_eq!(choose()["path"], json!(base.join(format!(".Trash-{uid}"))));
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
        std::fs::remove_file(base.join(".Trash")).unwrap();
        std::fs::create_dir(base.join(".Trash")).unwrap();
        std::fs::set_permissions(base.join(".Trash"), std::fs::Permissions::from_mode(0o1777))
            .unwrap();
        assert_eq!(choose()["path"], json!(base.join(format!(".Trash/{uid}"))));
        assert_eq!(choose()["top"], json!(base));
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn mount_table_uses_component_boundaries_and_decodes_spaces() {
        let table = "1 0 0:1 / / rw\n2 1 0:2 / /media/disk\\040one rw\n3 1 0:3 / /media/disk rw";
        assert_eq!(
            mount_top(Path::new("/media/disk one/sync"), table).unwrap(),
            Path::new("/media/disk one")
        );
        assert_eq!(
            mount_top(Path::new("/media/diskette"), table).unwrap(),
            Path::new("/")
        );
    }
}
