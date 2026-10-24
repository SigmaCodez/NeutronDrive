//! File-manager requests run in the existing GUI worker and use the live service.
use crate::{
    headless,
    runtime::{self, Failure},
};
use serde_json::{json, Value};
use std::path::{Component, Path};
#[allow(dead_code)]
#[path = "../../file-manager/src/paths.rs"]
pub mod paths;
fn fail(message: &str) -> Failure {
    Failure {
        message: message.into(),
        cancelled: false,
    }
}
pub fn request(args: &[String]) -> Result<Value, Failure> {
    if args.len() < 4 || args[0] != "--file-action" || args[2] != "--" {
        return Err(fail("Invalid file-manager request"));
    }
    let value = json!({"action":args[1],"paths":args[3..]});
    validate(&value)?;
    Ok(value)
}
pub fn validate(value: &Value) -> Result<(), Failure> {
    let action = value["action"].as_str().unwrap_or("");
    let paths = value["paths"]
        .as_array()
        .ok_or_else(|| fail("Select a sync file or folder"))?;
    if !["pin", "evict", "share"].contains(&action)
        || paths.is_empty()
        || paths.len() > 128
        || (action == "share" && paths.len() != 1)
    {
        return Err(fail("Invalid file-manager selection"));
    }
    for path in paths {
        let text = path.as_str().ok_or_else(|| fail("Invalid local path"))?;
        if text.len() > 4096
            || text.contains('\0')
            || !Path::new(text).is_absolute()
            || Path::new(text)
                .components()
                .any(|p| matches!(p, Component::ParentDir))
        {
            return Err(fail("Invalid local path"));
        }
    }
    if value.to_string().len() > 65536 {
        return Err(fail("Too many selected paths"));
    }
    Ok(())
}
pub fn run(
    sdk: &str,
    request: &Value,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value, Failure> {
    run_with(request, &mut |operation| {
        runtime::call(headless::desktop_request(sdk, operation)?, callback)
    })
}
fn run_with(
    request: &Value,
    call: &mut impl FnMut(Value) -> Result<Value, Failure>,
) -> Result<Value, Failure> {
    validate(request)?;
    let status = call(json!({"action":"status"}))?;
    if status["mounted"] != true {
        return Err(fail(
            "Start your sync folder before changing offline availability",
        ));
    }
    let root = Path::new(
        status["mountpoint"]
            .as_str()
            .ok_or_else(|| fail("Sync folder unavailable"))?,
    );
    let remote = status["configuration"]["remote_root"]
        .as_str()
        .ok_or_else(|| fail("Sync root unavailable"))?;
    let mut selected = Vec::new();
    // Validate the entire selection before any pin/evict changes.
    for item in request["paths"].as_array().unwrap() {
        let path = Path::new(item.as_str().unwrap());
        let relative = paths::relative(root, path)
            .ok_or_else(|| fail("The selected file is outside the active sync folder"))?;
        // Never resolve a symlink to a different cloud object. Metadata only.
        let mut current = root.to_path_buf();
        for component in Path::new(&relative).components() {
            current.push(component);
            let info = std::fs::symlink_metadata(&current)
                .map_err(|_| fail("The selected sync file is no longer available"))?;
            if info.file_type().is_symlink() {
                return Err(fail("Symbolic links cannot be shared or pinned"));
            }
        }
        selected.push(relative);
    }
    if request["action"] == "share" {
        let path = format!("{}/{}", remote.trim_end_matches('/'), selected[0]);
        return Ok(
            json!({"share":{"path":path,"name":Path::new(&selected[0]).file_name().and_then(|s|s.to_str()).unwrap_or("")}}),
        );
    }
    for (i, path) in selected.iter().enumerate() {
        if let Err(e) = call(json!({"action":request["action"],"path":path})) {
            return Err(fail(&format!(
                "{i}/{} completed: {}",
                selected.len(),
                e.message
            )));
        }
    }
    Ok(json!({"completed":selected.len()}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_arguments_and_invalid_requests() {
        let args = [
            "--file-action",
            "share",
            "--",
            "/home/a/ProtonDrive/a '$();.txt",
        ]
        .map(str::to_string);
        assert_eq!(request(&args).unwrap()["paths"][0], args[3]);
        for r in [
            json!({"action":"delete","paths":["/x"]}),
            json!({"action":"share","paths":["/x","/y"]}),
            json!({"action":"pin","paths":["/x/../y"]}),
            json!({"action":"pin","paths":["file:///x"]}),
        ] {
            assert!(validate(&r).is_err());
        }
    }
    #[test]
    fn maps_selected_file_without_creating_a_link_and_rejects_symlinks() {
        let dir = std::env::temp_dir().join(format!("nd-file-menu-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("literal $;.txt");
        std::fs::write(&path, b"test").unwrap();
        let status =
            json!({"mounted":true,"mountpoint":dir,"configuration":{"remote_root":"/my-files"}});
        let mut calls = Vec::new();
        let mut call = |op: Value| {
            calls.push(op);
            Ok(status.clone())
        };
        let result = run_with(&json!({"action":"share","paths":[path]}), &mut call).unwrap();
        assert_eq!(result["share"]["path"], "/my-files/literal $;.txt");
        assert_eq!(calls.len(), 1);
        std::os::unix::fs::symlink(&path, dir.join("link")).unwrap();
        assert!(run_with(
            &json!({"action":"pin","paths":[dir.join("link")]}),
            &mut |_| Ok(status.clone())
        )
        .is_err());
        assert!(
            run_with(&json!({"action":"pin","paths":["/outside"]}), &mut |_| Ok(
                status.clone()
            ))
            .is_err()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
