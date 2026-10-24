//! First-run setup uses the existing mount service and its persisted restore policy.
use crate::{
    autostart, headless,
    runtime::{self, Failure},
};
use serde_json::{json, Value};
use std::path::Path;
fn failure(message: &str) -> Failure {
    Failure {
        message: message.into(),
        cancelled: false,
    }
}
pub fn run(
    sdk: &str,
    request: &Value,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value, Failure> {
    let folder = request["mountpoint"]
        .as_str()
        .ok_or_else(|| failure("Choose a sync folder"))?;
    let status = runtime::call(
        headless::desktop_request(sdk, json!({"action":"status"}))?,
        callback,
    )?;
    if status["mounted"] == true && status["mountpoint"].as_str() != Some(folder) {
        return Err(failure(
            "Another sync folder is already active. Review it in sync settings before continuing.",
        ));
    }
    let mount = json!({"action":"mount","mountpoint":folder,"remote_root":"/my-files","writable":true,"offline":false});
    let result = if status["mounted"] == true {
        if status["configuration"]["remote_root"] != "/my-files"
            || status["configuration"]["writable"] != true
            || status["configuration"]["offline"] == true
        {
            return Err(failure(
                "The active mount has different settings. Review sync settings before continuing.",
            ));
        }
        status
    } else {
        prepare(Path::new(folder))?;
        runtime::call(headless::desktop_request(sdk, mount)?, callback)?
    };
    if result["mounted"] != true {
        return Err(failure(
            "The sync folder was not mounted. Setup has not completed.",
        ));
    }
    if let Err(error) = autostart::run(sdk, true, callback) {
        // A failed autostart setup must not be presented as a completed wizard.
        // Keep the live mount intact: an application may already have opened a file.
        return Err(failure(&format!("Sync folder is active, but autostart failed: {}. Configure autostart in Settings before completing setup.",error.message)));
    }
    Ok(result)
}
fn prepare(path: &Path) -> Result<(), Failure> {
    if !path.is_absolute() || path.parent().is_none() {
        return Err(failure("Choose an absolute sync folder"));
    }
    if path
        .components()
        .any(|p| matches!(p, std::path::Component::ParentDir))
    {
        return Err(failure("The sync folder must not contain parent traversal"));
    }
    if path.exists() || path.is_symlink() {
        let info = std::fs::symlink_metadata(path)
            .map_err(|_| failure("Cannot inspect the sync folder"))?;
        if !info.is_dir() || info.file_type().is_symlink() {
            return Err(failure(
                "The sync folder must be a real directory, not a symbolic link",
            ));
        }
        if std::fs::read_dir(path)
            .map_err(|_| failure("Cannot read the sync folder"))?
            .next()
            .is_some()
        {
            return Err(failure("The sync folder is not empty. Choose an empty folder; existing files will not be hidden or moved."));
        }
    } else {
        let parent = path.parent().unwrap();
        if !parent.is_dir() {
            return Err(failure("The parent directory does not exist"));
        }
        std::fs::create_dir(path).map_err(|_| failure("Cannot create the sync folder"))?;
    }
    Ok(())
}
/// Resolve a captured cloud path against the currently active mount only.
pub fn availability(
    sdk: &str,
    request: &Value,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value, Failure> {
    let status = runtime::call(
        headless::desktop_request(sdk, json!({"action":"status"}))?,
        callback,
    )?;
    if status["mounted"] != true {
        return Err(failure(
            "Start your sync folder before changing offline availability",
        ));
    }
    let root = status["configuration"]["remote_root"]
        .as_str()
        .ok_or_else(|| failure("Sync root unavailable"))?;
    let path = request["path"]
        .as_str()
        .ok_or_else(|| failure("Invalid selected path"))?;
    let relative = Path::new(path)
        .strip_prefix(Path::new(root))
        .map_err(|_| failure("The selected item is outside the active sync folder"))?;
    if relative
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(failure("Invalid relative sync path"));
    }
    runtime::call(
        headless::desktop_request(
            sdk,
            json!({"action":if request["action"]=="pin_remote" {"pin"}else{"evict"},"path":relative.to_str().ok_or_else(||failure("Unsupported path"))?}),
        )?,
        callback,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_existing_content_and_rejects_links() {
        let root = std::env::temp_dir().join(format!("nd-setup-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let folder = root.join("ProtonDrive");
        prepare(&folder).unwrap();
        std::fs::write(folder.join("existing"), b"keep").unwrap();
        assert!(prepare(&folder).is_err());
        assert_eq!(std::fs::read(folder.join("existing")).unwrap(), b"keep");
        let link = root.join("link");
        std::os::unix::fs::symlink(&folder, &link).unwrap();
        assert!(prepare(&link).is_err());
        assert!(prepare(Path::new("relative")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
