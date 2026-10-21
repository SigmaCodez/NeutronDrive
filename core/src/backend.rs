//! SDK value contracts. Secrets and raw SDK errors are never copied to errors.
use crate::process::{self, Failure};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};

type Result<T> = std::result::Result<T, Failure>;

#[derive(Deserialize)]
pub struct Request {
    version: u32,
    #[serde(flatten)]
    command: Command,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    WatcherCreate {
        root: Vec<u8>,
    },
    WatcherRun {
        id: u64,
    },
    WatcherStop {
        id: u64,
    },
    Jobs {
        state: String,
        planned: Vec<String>,
        operation: crate::jobs::Action,
    },
    MountPaths,
    ValidateLocal {
        root: String,
        state: String,
        planned: Vec<String>,
        mounted: Vec<(String, String)>,
    },
    StoreReadPresent {
        path: String,
        limit: usize,
        private: bool,
    },
    PrivateDirectory {
        path: String,
    },
    StoreRead {
        path: String,
        default: Value,
        limit: usize,
        private: bool,
    },
    StoreWrite {
        path: String,
        value: Value,
    },
    ValidateRemote {
        path: String,
    },
    ValidateName {
        name: String,
    },
    JoinRemote {
        parent: String,
        name: String,
    },
    LocalPath {
        path: String,
        directory: bool,
        recursive: bool,
    },
    DataDir,
    Node {
        item: Value,
        parent: String,
    },
    CheckedJson {
        output: String,
    },
    FriendlyError {
        stderr: String,
    },
}

pub fn remote(path: &str) -> Result<&str> {
    if !path.starts_with('/') || path.contains('\0') {
        return Err("Ungültiger Cloud-Pfad.".into());
    }
    if !matches!(
        path.split('/').nth(1),
        Some("my-files" | "trash" | "shared-with-me" | "devices")
    ) {
        return Err("Dieser Cloud-Bereich wird noch nicht unterstützt.".into());
    }
    Ok(path)
}

fn name(value: &str) -> Result<&str> {
    if value.trim().is_empty()
        || matches!(value, "." | "..")
        || value.chars().any(|c| c < ' ' || matches!(c, '/' | '\\'))
    {
        return Err(
            "Der Name darf nicht leer sein oder Schrägstriche/Steuerzeichen enthalten.".into(),
        );
    }
    Ok(value)
}

fn join(parent: &str, leaf: &str) -> Result<String> {
    Ok(format!(
        "{}/{}",
        remote(parent)?.trim_end_matches('/'),
        name(leaf)?
    ))
}

fn uid(value: &str) -> bool {
    let parts: Vec<_> = value.split('~').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            (part.len() == 22
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)))
                || ((88..=108).contains(&part.len())
                    && part
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"=_-".contains(&c)))
        })
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::String(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
        Value::Number(v) => v.as_f64() != Some(0.0),
    }
}

fn trash_state(value: Option<&Value>) -> Result<bool> {
    let error = "Die Proton-CLI lieferte einen ungültigen Papierkorbstatus.";
    let value = match value {
        None | Some(Value::Null) => return Ok(false),
        Some(value) => value,
    };
    let stamp = value.as_str().ok_or(error)?.as_bytes();
    if stamp.len() != 24
        || [4, 7, 10, 13, 16, 19, 23]
            .iter()
            .zip(b"--T::.Z")
            .any(|(i, c)| stamp[*i] != *c)
        || stamp
            .iter()
            .enumerate()
            .any(|(i, c)| ![4, 7, 10, 13, 16, 19, 23].contains(&i) && !c.is_ascii_digit())
    {
        return Err(error.into());
    }
    let number = |start: usize, end: usize| {
        stamp[start..end]
            .iter()
            .fold(0u32, |n, c| n * 10 + u32::from(c - b'0'))
    };
    let (year, month, day) = (number(0, 4), number(5, 7), number(8, 10));
    let days = match month {
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    if year == 0
        || day == 0
        || day > days
        || number(11, 13) > 23
        || number(14, 16) > 59
        || number(17, 19) > 59
    {
        return Err(error.into());
    }
    Ok(true)
}

fn node(item: Value, parent: &str) -> Result<Value> {
    remote(parent)?;
    let mut raw = item
        .as_object()
        .ok_or("Unerwartetes Datenformat der Proton-CLI.")?
        .clone();
    let mut safe = item["name"]["ok"] == true;
    let mut label = if safe {
        item["name"]["value"].as_str().unwrap_or("")
    } else {
        "[Name nicht entschlüsselbar]"
    };
    if label.is_empty() {
        label = "[Ungültiger Name]";
        safe = false;
    }
    let id = item
        .get("rootFolderUid")
        .or_else(|| item.get("uid"))
        .unwrap_or(&Value::Null);
    let id = if id.is_null() && !raw.contains_key("uid") && !raw.contains_key("rootFolderUid") {
        ""
    } else {
        id.as_str().ok_or("Ungültige Datei-ID von Proton.")?
    };
    let empty = json!({});
    let revision = item
        .get("activeRevision")
        .filter(|v| truthy(v))
        .unwrap_or(&empty);
    if !revision.is_object() {
        return Err("Ungültige Revisionsdaten von Proton.".into());
    }
    let kind = if raw.contains_key("rootFolderUid") {
        "folder"
    } else {
        item.get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    };
    safe = safe && !item.get("errors").is_some_and(truthy) && matches!(kind, "file" | "folder");
    let root = parent.split('/').nth(1).unwrap_or("");
    let path = if root == "my-files" && uid(id) {
        format!("/my-files/{id}")
    } else if root != "trash" && parent.matches('/').count() > 1 && uid(id) {
        format!("{parent}/{id}")
    } else {
        match join(parent, label) {
            Ok(path) => path,
            Err(_) => {
                safe = false;
                String::new()
            }
        }
    };
    let size = revision.get("claimedSize").and_then(Value::as_u64);
    let digests = revision
        .get("claimedDigests")
        .filter(|v| truthy(v))
        .unwrap_or(&empty);
    if !digests.is_object() {
        return Err("Ungültige Prüfsummenmetadaten von Proton.".into());
    }
    let string = |v: Option<&Value>| -> Result<String> {
        match v {
            None | Some(Value::Null) => Ok(String::new()),
            Some(Value::String(s)) => Ok(s.clone()),
            _ => Err("Ungültige Dateimetadaten von Proton.".into()),
        }
    };
    let modified = revision
        .get("claimedModificationTime")
        .filter(|v| truthy(v))
        .or_else(|| item.get("modificationTime"));
    let trashed = trash_state(item.get("trashTime"))?;
    raw.insert("isTrashed".into(), json!(trashed));
    Ok(
        json!({"uid":id,"name":label,"type":kind,"size":size,"modified":string(modified)?,
        "revision":string(revision.get("uid"))?,"sha1":string(digests.get("sha1"))?,"path":path,
        "raw":raw,"safe":safe,"media_type":string(item.get("mediaType"))?}),
    )
}

fn local_path(value: &str, directory: bool) -> Result<PathBuf> {
    let path = if value == "~" || value.starts_with("~/") {
        PathBuf::from(std::env::var_os("HOME").ok_or("Das Home-Verzeichnis fehlt.")?)
            .join(value.strip_prefix("~/").unwrap_or(""))
    } else {
        PathBuf::from(value)
    };
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|_| "Das Arbeitsverzeichnis fehlt.")?
            .join(path)
    };
    if path
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .any(|c| b"*?[{".contains(c))
    {
        return Err("Die Proton-CLI behandelt *, ?, [ und { in lokalen Pfaden als Suchmuster. Bitte einen anderen Pfad verwenden.".into());
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => (),
            part => normalized.push(part.as_os_str()),
        }
    }
    for part in normalized.ancestors() {
        if std::fs::symlink_metadata(part).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(
                "Symbolische Links werden für Dateiübertragungen nicht unterstützt.".into(),
            );
        }
    }
    let metadata = std::fs::metadata(&normalized)
        .map_err(|_| "Der lokale Pfad existiert nicht oder ist kein Ordner.")?;
    if directory && !metadata.is_dir() {
        return Err("Der lokale Pfad existiert nicht oder ist kein Ordner.".into());
    }
    if !metadata.is_file() && !metadata.is_dir() {
        return Err("Nur reguläre Dateien und Ordner werden unterstützt.".into());
    }
    Ok(normalized)
}

fn scan_upload(root: &Path, callback: &mut impl FnMut(Value) -> i32) -> Result<()> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        if callback(json!({"kind":"poll"})) != 0 {
            return Err(Failure {
                message: "Upload abgebrochen.".into(),
                cancelled: true,
            });
        }
        let info = std::fs::symlink_metadata(&path)
            .map_err(|_| "Upload-Ordner konnte nicht vollständig geprüft werden.")?;
        if info.is_dir() {
            let entries = std::fs::read_dir(path)
                .map_err(|_| "Upload-Ordner konnte nicht vollständig geprüft werden.")?;
            for entry in entries {
                pending.push(
                    entry
                        .map_err(|_| "Upload-Ordner konnte nicht vollständig geprüft werden.")?
                        .path(),
                );
            }
        } else if !info.is_file() {
            return Err("Der Upload-Ordner enthält symbolische Links oder Spezialdateien. Bitte diese zuerst ausschließen.".into());
        }
    }
    Ok(())
}

pub fn dispatch(request: Request, callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    if request.version != 1 {
        return Err("Unsupported backend protocol".into());
    }
    match request.command {
        Command::WatcherCreate { root } => Ok(json!(crate::watcher::create(root)?)),
        Command::WatcherRun { id } => {
            crate::watcher::run(id, callback)?;
            Ok(Value::Null)
        }
        Command::WatcherStop { id } => {
            crate::watcher::stop(id)?;
            Ok(Value::Null)
        }
        Command::Jobs {
            state,
            planned,
            operation,
        } => crate::jobs::transact(Path::new(&state), &planned, operation, callback),
        Command::MountPaths => Ok(json!(crate::jobs::mounts()?)),
        Command::ValidateLocal {
            root,
            state,
            planned,
            mounted,
        } => Ok(json!(crate::jobs::local_identity(
            Path::new(&root),
            Path::new(&state),
            &planned,
            &mounted
        )?)),
        Command::StoreReadPresent {
            path,
            limit,
            private,
        } => {
            let value = crate::storage::read_optional(Path::new(&path), limit, private)?;
            Ok(json!({"present":value.is_some(), "value":value}))
        }
        Command::PrivateDirectory { path } => {
            crate::storage::directory(Path::new(&path), true, true)?;
            Ok(json!(path))
        }
        Command::StoreRead {
            path,
            default,
            limit,
            private,
        } => crate::storage::read(Path::new(&path), default, limit, private),
        Command::StoreWrite { path, value } => {
            crate::storage::write(Path::new(&path), value)?;
            Ok(Value::Null)
        }
        Command::ValidateRemote { path } => Ok(json!(remote(&path)?)),
        Command::ValidateName { name: value } => Ok(json!(name(&value)?)),
        Command::JoinRemote { parent, name } => Ok(json!(join(&parent, &name)?)),
        Command::LocalPath {
            path,
            directory,
            recursive,
        } => {
            let path = local_path(&path, directory)?;
            if recursive {
                scan_upload(&path, callback)?;
            }
            Ok(json!(path.to_str().ok_or(
                "Nicht-UTF-8-Pfade werden vom SDK nicht unterstützt."
            )?))
        }
        Command::DataDir => Ok(json!(process::data_dir()?
            .to_str()
            .ok_or("Nicht-UTF-8-Zustandsverzeichnis.")?)),
        Command::Node { item, parent } => node(item, &parent),
        Command::CheckedJson { output } => process::checked_json(&output),
        Command::FriendlyError { stderr } => Ok(json!(process::friendly_error(&stderr))),
    }
}
