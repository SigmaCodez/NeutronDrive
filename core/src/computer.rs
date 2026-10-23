//! Namespace and identity policy for computer folder synchronization.
use serde_json::{json, Value};

pub fn boundary(path: &str) -> Result<String, String> {
    if !path.starts_with('/')
        || path.contains('\0')
        || path[1..].split('/').any(|p| matches!(p, "" | "." | ".."))
    {
        return Err("Invalid synchronization path".into());
    }
    let parts: Vec<_> = path.split('/').collect();
    match parts.get(1).copied() {
        Some("my-files") => Ok("/my-files".into()),
        Some("devices") if parts.len() >= 3 => Ok(format!("/devices/{}", parts[2])),
        _ => Err("Only My files and computer folders support synchronization".into()),
    }
}

pub fn record(value: Value, account: &str) -> Result<Value, String> {
    let text = |key: &str| -> Result<&str, String> {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
            .ok_or_else(|| "Invalid computer identity".to_string())
    };
    let uid = text("uid")?;
    let root = text("rootFolderUid")?;
    let name = text("name")?;
    if account.is_empty() || name.contains('/') || matches!(name, "." | "..") {
        return Err("Invalid computer identity".into());
    }
    if let Some(saved) = value.get("account_uid") {
        if saved.as_str() != Some(account) {
            return Err("Computer belongs to a different Proton account".into());
        }
    }
    Ok(
        json!({"uid":uid,"rootFolderUid":root,"name":name,"account_uid":account,"path":format!("/devices/{name}")}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boundaries_are_strict() {
        assert_eq!(
            boundary("/devices/Linux PC/sub/folder").unwrap(),
            "/devices/Linux PC"
        );
        assert_eq!(boundary("/my-files/sub").unwrap(), "/my-files");
        for bad in [
            "/devices",
            "/devices/../x",
            "/devices/a//b",
            "/my-filesx/a",
            "/shared-with-me/a",
            "/my-files/",
        ] {
            assert!(boundary(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn computer_record_pins_account_and_root() {
        let record = record(
            json!({"uid":"d", "rootFolderUid":"r", "name":"Linux PC"}),
            "a",
        )
        .unwrap();
        assert!(super::record(record.clone(), "b").is_err());
        assert_eq!(
            super::record(record, "a").unwrap()["path"],
            "/devices/Linux PC"
        );
        for name in ["", "../escape", "..", "a\0b"] {
            assert!(
                super::record(json!({"uid":"d", "rootFolderUid":"r", "name":name}), "a").is_err()
            );
        }
    }
}
