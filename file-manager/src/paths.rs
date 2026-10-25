//! Metadata-only menu eligibility. The application revalidates against the live mount.
use serde_json::Value;
use std::{
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

pub fn relative(root: &Path, path: &Path) -> Option<String> {
    if !root.is_absolute() || !path.is_absolute() || root == Path::new("/") {
        return None;
    }
    if root
        .components()
        .chain(path.components())
        .any(|c| matches!(c, Component::ParentDir))
    {
        return None;
    }
    let suffix = path.strip_prefix(root).ok()?;
    if suffix.as_os_str().is_empty() {
        return None;
    }
    if !suffix
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return None;
    }
    Some(suffix.to_str()?.to_string())
}
pub fn state_directory() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state")))?;
    base.is_absolute().then(|| base.join("neutrondrive"))
}
pub fn configured_root() -> Option<PathBuf> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(state_directory()?.join("ondemand-service/mount.json"))
        .ok()?;
    let m = file.metadata().ok()?;
    if !m.is_file() || m.uid() != unsafe { libc::getuid() } || m.len() > 16384 {
        return None;
    }
    let mut text = String::new();
    file.take(16385).read_to_string(&mut text).ok()?;
    let config: Value = serde_json::from_str(&text).ok()?;
    let root = PathBuf::from(config["mountpoint"].as_str()?);
    (root.is_absolute() && root != Path::new("/")).then_some(root)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn containment_is_component_based_and_preserves_literal_names() {
        let root = Path::new("/home/test/ProtonDrive");
        assert_eq!(
            relative(root, Path::new("/home/test/ProtonDrive/a b/'$(x);.txt")),
            Some("a b/'$(x);.txt".into())
        );
        for path in [
            "/home/test/ProtonDrive-other/a",
            "/home/test/ProtonDrive/../private",
            "/home/test/ProtonDrive",
            "relative",
        ] {
            assert!(relative(root, Path::new(path)).is_none());
        }
    }
}
