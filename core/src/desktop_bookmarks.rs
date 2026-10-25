//! Keep a normal GTK file-manager bookmark; native integrations can hide the redundant FUSE volume.
use std::{os::unix::ffi::OsStrExt, path::Path};
fn uri(path: &Path) -> String {
    let mut s = String::from("file://");
    for b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(b) {
            s.push(*b as char)
        } else {
            s.push_str(&format!("%{b:02X}"))
        }
    }
    s
}
fn updated(text: &str, path: &Path) -> String {
    let target = uri(path);
    let mut lines: Vec<String> = text
        .lines()
        .filter(|line| line.split_whitespace().next() != Some(target.as_str()))
        .map(str::to_owned)
        .collect();
    lines.push(format!("{target} Proton Drive"));
    format!("{}\n", lines.join("\n"))
}
pub(crate) fn ensure(path: &Path) -> std::io::Result<()> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .ok_or_else(|| std::io::Error::other("No configuration directory"))?;
    if !config.is_absolute() {
        return Err(std::io::Error::other(
            "Configuration directory must be absolute",
        ));
    }
    let folder = config.join("gtk-3.0");
    std::fs::create_dir_all(&folder)?;
    let file = folder.join("bookmarks");
    let text = match std::fs::symlink_metadata(&file) {
        Ok(m) if m.is_file() && !m.file_type().is_symlink() && m.len() <= 1024 * 1024 => {
            std::fs::read_to_string(&file)?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        _ => return Err(std::io::Error::other("Unsafe bookmark file")),
    };
    let next = updated(&text, path);
    if next == text {
        return Ok(());
    }
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let temp = folder.join(format!(".neutrondrive-bookmarks-{}", std::process::id()));
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    let result = (|| {
        out.write_all(next.as_bytes())?;
        out.sync_all()?;
        std::fs::rename(&temp, &file)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bookmark_preserves_other_entries_and_is_idempotent() {
        let p = Path::new("/home/a/Proton Drive/ü");
        let old = "file:///home/a/Documents Documents\n";
        let n = updated(old, p);
        assert!(n.starts_with(old));
        assert!(n.contains("Proton%20Drive/%C3%BC Proton Drive"));
        assert_eq!(updated(&n, p), n);
    }
}
