//! Typed Drive operations. The pinned SDK remains the cryptographic boundary.
//! Validation, command construction and result contracts live here, not in UI bindings.
use crate::{backend, process};
use process::Failure;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

type Result<T> = std::result::Result<T, Failure>;
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    TakeoutScan {
        root: String,
        #[serde(default)]
        folder_albums: bool,
    },
    TakeoutImport {
        plan: crate::takeout::Plan,
    },
    PhotoImport {
        request: Value,
    },
    ComputerRemove {
        uid: String,
        root_uid: String,
    },
    ComputerCreate {
        name: String,
    },
    ComputerInfo {
        uid: String,
    },
    Versions {
        path: String,
    },
    RestoreVersion {
        path: String,
        revision: String,
        expected: String,
    },
    PhotoLibrary {
        request: Value,
    },
    Photos {
        offset: u64,
    },
    DownloadPhoto {
        uid: String,
        destination: String,
    },
    AccountStorage,
    Version,
    Login,
    Logout,
    List {
        path: String,
    },
    Info {
        path: String,
    },
    Mkdir {
        parent: String,
        name: String,
    },
    Rename {
        path: String,
        name: String,
    },
    Move {
        paths: Vec<String>,
        destination: String,
    },
    Trash {
        paths: Vec<String>,
    },
    Restore {
        paths: Vec<String>,
    },
    Upload {
        paths: Vec<String>,
        parent: String,
        file_strategy: String,
        folder_strategy: String,
    },
    Download {
        paths: Vec<String>,
        destination: String,
        file_strategy: String,
        folder_strategy: String,
    },
    Invite {
        path: String,
        email: String,
        role: String,
    },
    Sharing {
        path: String,
    },
    SetLink {
        path: String,
        settings: Value,
    },
    CreateLink {
        path: String,
    },
    RemoveLink {
        path: String,
    },
}
fn selected(paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Err("Keine Dateien ausgewählt.".into());
    }
    for path in paths {
        backend::remote(path)?;
    }
    Ok(())
}
fn command(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|v| (*v).into()).collect()
}
fn nodes(value: Value, path: &str) -> Result<Value> {
    let values = value
        .as_array()
        .ok_or("Die Proton-CLI lieferte keine Dateiliste.")?;
    let mut nodes = values
        .iter()
        .map(|v| backend::node(v.clone(), path))
        .collect::<Result<Vec<_>>>()?;
    if matches!(path, "/trash" | "/shared-with-me" | "/devices") {
        let mut counts = HashMap::new();
        for node in &nodes {
            *counts
                .entry(node["name"].as_str().unwrap_or("").to_owned())
                .or_insert(0) += 1;
        }
        for node in &mut nodes {
            if counts[node["name"].as_str().unwrap_or("")] > 1 {
                node["safe"] = json!(false);
            }
        }
    }
    Ok(json!(nodes))
}

pub fn run(
    executable: String,
    operation: Operation,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    let operation = match operation {
        Operation::TakeoutScan {
            root,
            folder_albums,
        } => {
            return crate::takeout::scan_with_albums(
                std::path::Path::new(&root),
                folder_albums,
                callback,
            )
        }
        Operation::TakeoutImport { plan } => {
            return crate::takeout::import(plan, callback, |request, cb| {
                run_uncached(
                    executable.clone(),
                    Operation::PhotoImport { request },
                    &mut |v| cb(v),
                )
            })
        }
        other => other,
    };
    if let Operation::PhotoLibrary { request } = &operation {
        if request["action"] == "details" {
            return crate::photo_cache::details(
                request.clone(),
                crate::photo_cache::root().as_deref(),
                &mut |value| {
                    let operation = serde_json::from_value(value)
                        .map_err(|_| Failure::from("Invalid cached photo operation"))?;
                    run_uncached(executable.clone(), operation, callback)
                },
            );
        }
    }
    let logout = matches!(operation, Operation::Logout);
    let result = run_uncached(executable, operation, callback);
    if logout && result.is_ok() {
        crate::photo_cache::clear();
    }
    result
}
fn run_uncached(
    executable: String,
    operation: Operation,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    if let Operation::SetLink { path, settings } = &operation {
        backend::remote(path)?;
        let input = settings.to_string();
        if !settings.is_object() || input.len() > 4096 {
            return Err("Invalid link settings".into());
        }
        return process::run_with_input(
            process::Request {
                version: 1,
                executable,
                args: command(&["sharing", "configure", "--json", "--", path]),
                timeout: 120.,
                parse: true,
                login: false,
                max_output: 1024 * 1024,
            },
            Some(&input),
            callback,
        );
    }
    execute(
        operation,
        callback,
        &mut |args, timeout, parse, login, callback| {
            process::run(
                process::Request {
                    version: 1,
                    executable: executable.clone(),
                    args,
                    timeout,
                    parse,
                    login,
                    max_output: 64 * 1024 * 1024,
                },
                callback,
            )
        },
    )
}

// A transport seam permits deterministic SDK contract tests without accounts.
fn execute<C, T>(operation: Operation, callback: &mut C, transport: &mut T) -> Result<Value>
where
    C: FnMut(Value) -> i32,
    T: FnMut(Vec<String>, f64, bool, bool, &mut C) -> Result<Value>,
{
    let mut timeout = 3600.;
    let mut parse = true;
    let mut login = false;
    let mut node_parent = None;
    let mut list_path = None;
    let mut optional = false;
    let args = match operation {
        Operation::ComputerRemove { uid, root_uid } => {
            backend::name(&uid)?;
            backend::name(&root_uid)?;
            command(&["computers", "remove", "--json", "--", &uid, &root_uid])
        }
        Operation::ComputerCreate { name } => {
            backend::name(&name)?;
            command(&["computers", "create", "--json", "--", &name])
        }
        Operation::ComputerInfo { uid } => {
            if uid.is_empty() || uid.len() > 512 || uid.chars().any(char::is_control) {
                return Err("Invalid computer identity".into());
            }
            command(&["computers", "info", "--json", "--", &uid])
        }
        Operation::Versions { path } => {
            backend::remote(&path)?;
            command(&["versions", "list", "--json", "--", &path])
        }
        Operation::RestoreVersion {
            path,
            revision,
            expected,
        } => {
            backend::remote(&path)?;
            for value in [&revision, &expected] {
                if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
                    return Err("Invalid revision identity".into());
                }
            }
            command(&[
                "versions", "restore", "--json", "--", &path, &revision, &expected,
            ])
        }
        Operation::TakeoutScan { .. } | Operation::TakeoutImport { .. } => {
            return Err("Invalid Takeout transport".into())
        }
        Operation::PhotoImport { request } => {
            if !request.is_object() || request.to_string().len() > 65536 {
                return Err("Invalid photo import request".into());
            }
            command(&[
                "photo",
                "import-single",
                "--json",
                "--",
                &request.to_string(),
            ])
        }
        Operation::PhotoLibrary { request } => {
            if !request.is_object() || request.to_string().len() > 64 * 1024 {
                return Err("Invalid photo library request".into());
            }
            if !matches!(
                request["action"].as_str(),
                Some(
                    "index"
                        | "details"
                        | "albums"
                        | "create"
                        | "rename"
                        | "cover"
                        | "delete"
                        | "add"
                        | "remove"
                )
            ) {
                return Err("Unsupported photo library action".into());
            }
            command(&["photo", "library", "--json", "--", &request.to_string()])
        }
        Operation::Photos { offset } => {
            command(&["photo", "page", "--json", "--", &offset.to_string()])
        }
        Operation::DownloadPhoto { uid, destination } => {
            if !backend::uid(&uid) {
                return Err("Invalid photo identity".into());
            }
            let destination = backend::local_path(&destination, true)?;
            command(&[
                "photo",
                "download",
                "--json",
                "--conflict-strategy",
                "rename",
                "--",
                &format!("/photos/{uid}"),
                destination.to_str().ok_or("Unsupported local path")?,
            ])
        }
        Operation::AccountStorage => {
            timeout = 10.;
            command(&["account", "storage", "--json"])
        }
        Operation::Version => {
            timeout = 30.;
            parse = false;
            command(&["version"])
        }
        Operation::Login => {
            timeout = 600.;
            login = true;
            command(&["auth", "login", "--json"])
        }
        Operation::Logout => {
            timeout = 60.;
            command(&["auth", "logout", "--json"])
        }
        Operation::List { path } => {
            timeout = 10.;
            backend::remote(&path)?;
            list_path = Some(path.clone());
            command(&["filesystem", "list", "--json", "--", &path])
        }
        Operation::Info { path } => {
            timeout = 10.;
            backend::remote(&path)?;
            node_parent = Some(
                path.rsplit_once('/')
                    .map(|(p, _)| if p.is_empty() { path.as_str() } else { p })
                    .unwrap_or(&path)
                    .to_owned(),
            );
            command(&["filesystem", "info", "--json", "--", &path])
        }
        Operation::Mkdir { parent, name } => {
            backend::remote(&parent)?;
            backend::name(&name)?;
            node_parent = Some(parent.clone());
            command(&[
                "filesystem",
                "create-folder",
                "--json",
                "--",
                &parent,
                &name,
            ])
        }
        Operation::Rename { path, name } => {
            backend::remote(&path)?;
            backend::name(&name)?;
            command(&["filesystem", "rename", "--json", "--", &path, &name])
        }
        Operation::Move { paths, destination } => {
            selected(&paths)?;
            backend::remote(&destination)?;
            let mut args = command(&["filesystem", "move", "--json", "--"]);
            args.extend(paths);
            args.push(destination);
            args
        }
        Operation::Trash { paths } => {
            selected(&paths)?;
            let mut args = command(&["filesystem", "trash", "--json", "--"]);
            args.extend(paths);
            args
        }
        Operation::Restore { paths } => {
            selected(&paths)?;
            let current = execute(
                Operation::List {
                    path: "/trash".into(),
                },
                callback,
                transport,
            )?;
            for path in &paths {
                let matches: Vec<_> = current
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|node| node["path"] == *path)
                    .collect();
                if matches.len() != 1 || matches[0]["safe"] != true {
                    return Err("Mehrdeutiger Papierkorb-Eintrag. Bitte in der Proton-Web-App wiederherstellen.".into());
                }
            }
            let mut args = command(&["filesystem", "restore", "--json", "--"]);
            args.extend(paths);
            args
        }
        Operation::Upload {
            paths,
            parent,
            file_strategy,
            folder_strategy,
        } => {
            if paths.is_empty()
                || !["rename", "create-new-revision", "skip"].contains(&file_strategy.as_str())
                || !["merge", "rename", "skip"].contains(&folder_strategy.as_str())
            {
                return Err("Ungültige Upload-Einstellungen.".into());
            }
            backend::remote(&parent)?;
            let mut args = command(&[
                "filesystem",
                "upload",
                "--json",
                "--file-conflict-strategy",
                &file_strategy,
                "--folder-conflict-strategy",
                &folder_strategy,
                "--",
            ]);
            for path in paths {
                let path = backend::local_path(&path, false)?;
                backend::scan_upload(&path, callback)?;
                args.push(
                    path.to_str()
                        .ok_or("Nicht-UTF-8-Pfade werden vom SDK nicht unterstützt.")?
                        .to_owned(),
                );
            }
            args.push(parent);
            args
        }
        Operation::Download {
            paths,
            destination,
            file_strategy,
            folder_strategy,
        } => {
            if paths.is_empty()
                || !["rename", "skip"].contains(&file_strategy.as_str())
                || !["merge", "rename", "skip"].contains(&folder_strategy.as_str())
            {
                return Err("Ungültige Download-Einstellungen.".into());
            }
            selected(&paths)?;
            let destination = backend::local_path(&destination, true)?;
            let mut args = command(&[
                "filesystem",
                "download",
                "--json",
                "--file-conflict-strategy",
                &file_strategy,
                "--folder-conflict-strategy",
                &folder_strategy,
                "--",
            ]);
            args.extend(paths);
            args.push(
                destination
                    .to_str()
                    .ok_or("Nicht-UTF-8-Pfade werden vom SDK nicht unterstützt.")?
                    .to_owned(),
            );
            args
        }
        Operation::Invite { path, email, role } => {
            backend::remote(&path)?;
            if email.len() > 254
                || email.starts_with('-')
                || email.chars().any(|c| c.is_whitespace() || c.is_control())
                || email.split('@').count() != 2
                || email.split('@').any(str::is_empty)
                || !["viewer", "editor"].contains(&role.as_str())
            {
                return Err("Invalid sharing recipient or role".into());
            }
            command(&[
                "sharing", "invite", "--json", "--user", &email, "--role", &role, "--", &path,
            ])
        }
        Operation::Sharing { path } => {
            backend::remote(&path)?;
            parse = false;
            optional = true;
            command(&["sharing", "status", "--json", "--", &path])
        }
        Operation::RemoveLink { path } => {
            backend::remote(&path)?;
            parse = false;
            optional = true;
            command(&["sharing", "remove-url", "--json", "--", &path])
        }
        Operation::SetLink { .. } => return Err("Link settings require private transport".into()),
        Operation::CreateLink { path } => {
            backend::remote(&path)?;
            command(&[
                "sharing", "set-url", "--json", "--role", "viewer", "--", &path,
            ])
        }
    };
    if callback(json!({"kind":"poll"})) != 0 {
        return Err(Failure {
            message: "Vorgang abgebrochen.".into(),
            cancelled: true,
        });
    }
    let result = transport(args, timeout, parse, login, callback)?;
    if let Some(path) = list_path {
        return nodes(result, &path);
    }
    if let Some(parent) = node_parent {
        return backend::node(result, &parent);
    }
    if optional {
        let text = result
            .as_str()
            .ok_or("Die Proton-CLI lieferte keinen gültigen Freigabestatus.")?;
        if text == "undefined" {
            return Ok(Value::Null);
        }
        let value = process::checked_json(text)?;
        if !value.is_object() {
            return Err("Die Proton-CLI lieferte keinen gültigen Freigabestatus.".into());
        }
        return Ok(value);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_selection_never_reaches_transport() {
        for op in [
            Operation::Trash { paths: vec![] },
            Operation::Move {
                paths: vec!["/my-files/a".into()],
                destination: "/unknown".into(),
            },
            Operation::Rename {
                path: "/my-files/a".into(),
                name: "../b".into(),
            },
        ] {
            assert!(execute(op, &mut |_| 0, &mut |_, _, _, _, _| panic!(
                "unexpected transport"
            ))
            .is_err());
        }
    }
    #[test]
    fn invitations_validate_roles_and_preserve_literal_email_arguments() {
        for (email, role) in [
            ("bad address@example.org", "viewer"),
            ("x@example.org", "admin"),
            ("--option@example.org", "viewer"),
            ("x@@example.org", "viewer"),
        ] {
            assert!(execute(
                Operation::Invite {
                    path: "/my-files/a".into(),
                    email: email.into(),
                    role: role.into()
                },
                &mut |_| 0,
                &mut |_, _, _, _, _| panic!("invalid invitation reached transport")
            )
            .is_err());
        }
        execute(
            Operation::Invite {
                path: "/shared-by-me/uid".into(),
                email: "person+test@example.org".into(),
                role: "viewer".into(),
            },
            &mut |_| 0,
            &mut |args, _, _, _, _| {
                assert_eq!(
                    args,
                    command(&[
                        "sharing",
                        "invite",
                        "--json",
                        "--user",
                        "person+test@example.org",
                        "--role",
                        "viewer",
                        "--",
                        "/shared-by-me/uid"
                    ])
                );
                Ok(json!({}))
            },
        )
        .unwrap();
    }
    #[test]
    fn optional_account_storage_has_a_short_total_timeout() {
        execute(
            Operation::AccountStorage,
            &mut |_| 0,
            &mut |args, timeout, _, _, _| {
                assert_eq!(args, command(&["account", "storage", "--json"]));
                assert_eq!(timeout, 10.0);
                Ok(json!({"total":100,"used":25}))
            },
        )
        .unwrap();
    }
    #[test]
    fn cancellation_stops_before_transport() {
        let result = execute(Operation::Logout, &mut |_| 1, &mut |_, _, _, _, _| {
            panic!("unexpected transport")
        });
        assert!(result.unwrap_err().cancelled);
    }
    #[test]
    fn optional_sharing_and_required_json_remain_distinct() {
        let value = execute(
            Operation::Sharing {
                path: "/my-files/a".into(),
            },
            &mut |_| 0,
            &mut |args, _, parse, _, _| {
                assert_eq!(
                    args,
                    command(&["sharing", "status", "--json", "--", "/my-files/a"])
                );
                assert!(!parse);
                Ok(json!("undefined"))
            },
        )
        .unwrap();
        assert!(value.is_null());
        assert!(execute(
            Operation::Sharing {
                path: "/my-files/a".into()
            },
            &mut |_| 0,
            &mut |_, _, _, _, _| Ok(json!("[]"))
        )
        .is_err());
    }
    #[test]
    fn ambiguous_restore_only_lists_without_mutation() {
        let mut calls = 0;
        let result = execute(
            Operation::Restore {
                paths: vec!["/trash/same".into()],
            },
            &mut |_| 0,
            &mut |args, _, _, _, _| {
                calls += 1;
                assert_eq!(args[1], "list");
                let n = json!({"uid":format!("{}~{}","a".repeat(22),"b".repeat(22)),"name":{"ok":true,"value":"same"},"type":"file"});
                Ok(json!([n, n]))
            },
        );
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }
}
