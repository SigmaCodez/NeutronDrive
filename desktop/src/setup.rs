//! First-run setup uses the existing mount service and its persisted restore policy.
use crate::{
    autostart, background, headless,
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
    if ["FLATPAK_ID", "SNAP"]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()))
    {
        return sandbox_setup(folder, &mut |operation| match operation["kind"].as_str() {
            Some("autostart") => autostart::run(sdk, true, callback),
            Some("background") => background::run(sdk, &operation["request"], callback),
            _ => runtime::call(operation, callback),
        });
    }
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
// Store packages synchronize real files within their own sandbox. No host helper,
// FUSE mount, host D-Bus activation or automatic native-package installation.
fn sandbox_setup(
    folder: &str,
    call: &mut impl FnMut(Value) -> Result<Value, Failure>,
) -> Result<Value, Failure> {
    let root = call(json!({"op":"data_dir"}))?;
    let state = Path::new(
        root.as_str()
            .ok_or_else(|| failure("Invalid state directory"))?,
    )
    .join("background");
    let jobs = call(json!({"op":"jobs","state":state,"planned":[],"operation":{"action":"list"}}))?;
    let jobs = jobs
        .as_array()
        .ok_or_else(|| failure("Invalid sync job list"))?;
    let existing = jobs
        .iter()
        .find(|j| j["local_root"].as_str() == Some(folder));
    if let Some(job) = existing {
        if job["remote_root"] != "/my-files" {
            return Err(failure(
                "This folder belongs to another sync job. Review sync settings.",
            ));
        }
        if job["id"].as_str().is_none_or(str::is_empty) {
            return Err(failure("Invalid sync job identity"));
        }
    } else {
        prepare(Path::new(folder))?;
    }
    // Obtain background/autostart consent before registering a job or transferring files.
    call(json!({"kind":"autostart"}))?;
    let request = if let Some(job) = existing {
        json!({"action":"resume","id":job["id"]})
    } else {
        json!({"action":"add","local":folder,"remote":"/my-files"})
    };
    let result = call(json!({"kind":"background","request":request}))?;
    if result["running"] != true {
        return Err(failure(
            "The sync service did not start. Setup has not completed.",
        ));
    }
    Ok(json!({"mode":"download","running":true}))
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
    fn sandbox_new_folder_requires_empty_directory_and_confirms_before_job_creation() {
        let root = std::env::temp_dir().join(format!("nd-store-setup-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let folder = root.join("new");
        let mut calls = vec![];
        let result = sandbox_setup(folder.to_str().unwrap(), &mut |r| {
            calls.push(r.clone());
            Ok(match r["op"].as_str() {
                Some("data_dir") => json!("/state"),
                Some("jobs") => json!([]),
                _ if r["kind"] == "background" => json!({"running":true}),
                _ => json!(true),
            })
        })
        .unwrap();
        assert_eq!(result["mode"], "download");
        assert!(folder.is_dir());
        assert_eq!(calls[2]["kind"], "autostart");
        assert_eq!(calls[3]["request"]["action"], "add");
        assert_eq!(calls[3]["request"]["remote"], "/my-files");
        std::fs::write(folder.join("keep"), "existing content").unwrap();
        let mut count = 0;
        assert!(sandbox_setup(folder.to_str().unwrap(), &mut |r| {
            count += 1;
            Ok(if r["op"] == "data_dir" {
                json!("/state")
            } else {
                json!([])
            })
        })
        .is_err());
        assert_eq!(count, 2);
        assert_eq!(
            std::fs::read_to_string(folder.join("keep")).unwrap(),
            "existing content"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn sandbox_retry_reuses_job_and_requires_background_start() {
        for running in [false, true] {
            let mut calls = vec![];
            let result = sandbox_setup("/existing", &mut |r| {
                calls.push(r.clone());
                Ok(match r["op"].as_str() {
                    Some("data_dir") => json!("/state"),
                    Some("jobs") => {
                        json!([{"id":"existing-job","local_root":"/existing","remote_root":"/my-files"}])
                    }
                    _ if r["kind"] == "background" => json!({"running":running}),
                    _ => json!(true),
                })
            });
            assert_eq!(result.is_ok(), running);
            assert_eq!(calls[2]["kind"], "autostart");
            assert_eq!(
                calls[3]["request"],
                json!({"action":"resume","id":"existing-job"})
            );
        }
    }
    #[test]
    fn sandbox_denied_autostart_does_not_register_or_start_sync() {
        let mut calls = vec![];
        let result = sandbox_setup("/existing", &mut |r| {
            calls.push(r.clone());
            match r["op"].as_str() {
                Some("data_dir") => Ok(json!("/state")),
                Some("jobs") => {
                    Ok(json!([{"id":"job","local_root":"/existing","remote_root":"/my-files"}]))
                }
                _ => Err(failure("Permission denied")),
            }
        });
        assert!(result.is_err());
        assert_eq!(calls.len(), 3);
        assert!(!calls.iter().any(|r| r["kind"] == "background"));
    }
    #[test]
    fn sandbox_rejects_conflicting_existing_job_before_enabling_autostart() {
        let mut count = 0;
        assert!(sandbox_setup("/existing", &mut |r| {
            count += 1;
            Ok(if r["op"] == "data_dir" {
                json!("/state")
            } else {
                json!([{"id":"job","local_root":"/existing","remote_root":"/my-files/Other"}])
            })
        })
        .is_err());
        assert_eq!(count, 2);
    }
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
