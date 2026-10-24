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
    DesktopRuntime {
        config: crate::desktop_runtime::Config,
    },
    DesktopService {
        operation: crate::desktop_service::Operation,
    },
    Portal {
        operation: crate::portals::Operation,
    },
    DesktopBus {
        operation: crate::desktop_bus::Operation,
    },
    Autostart {
        operation: crate::autostart::Operation,
    },
    UpdateRuntime {
        operation: crate::update_runtime::Operation,
    },
    EventSource {
        operation: crate::event_source::Operation,
        executable: Option<String>,
        #[serde(default)]
        host_delivery: bool,
    },
    MountManager {
        operation: crate::mount_manager::Operation,
        executable: Option<String>,
    },
    FuseSession {
        operation: crate::fuse::Config,
        executable: Option<String>,
    },
    OndemandFs {
        operation: Box<crate::ondemand_fs::Operation>,
        executable: Option<String>,
    },
    OndemandNamespace {
        operation: Box<crate::ondemand_namespace::Request>,
        executable: Option<String>,
    },
    OndemandReplace {
        operation: Box<crate::ondemand_replace::Request>,
        executable: Option<String>,
    },
    OndemandHandles {
        operation: crate::ondemand_handles::Operation,
    },
    OndemandTree {
        operation: crate::ondemand_tree::Operation,
    },
    OndemandWrite {
        operation: Box<crate::ondemand_write::Request>,
        executable: Option<String>,
    },
    OndemandIo {
        operation: crate::ondemand_io::Operation,
        executable: Option<String>,
    },
    Daemon {
        config: crate::daemon::Config,
    },
    DaemonControl {
        action: crate::daemon_control::Action,
        job_id: Option<String>,
        reload_only: bool,
    },
    ControlServer {
        operation: crate::control_server::Operation,
    },
    DaemonTransition {
        runtime: Box<crate::daemon_job::Runtime>,
        outcome: Box<crate::daemon_job::Outcome>,
        context: crate::daemon_job::Context,
    },
    ServiceController {
        state: String,
        operation: crate::service_controller::Operation,
    },
    JobRoots {
        jobs: Vec<Value>,
        selected: String,
    },
    EventAncestry {
        event: crate::event_ancestry::Event,
        jobs: Vec<crate::event_ancestry::Job>,
    },
    MetadataCache {
        operation: crate::metadata_cache::Operation,
    },
    SyncExecute {
        state: String,
        plan: Value,
        trash_state: String,
        trash_data: String,
        trash_desktop: bool,
        executable: Option<String>,
    },
    HostFailure {
        id: u64,
        message: String,
        cancelled: bool,
    },
    SyncPlan {
        state: String,
        root: String,
        remote_path: String,
        executable: Option<String>,
    },
    SyncProof {
        operation: crate::sync_proof::Operation,
        executable: Option<String>,
    },
    SyncState {
        operation: crate::sync_state::Operation,
    },
    HostReply {
        id: u64,
        value: Value,
    },
    SyncRemote {
        operation: crate::sync_remote::Operation,
        executable: Option<String>,
    },
    TrashInit {
        state: String,
    },
    TrashMove {
        request: crate::trash_move::Request,
    },
    TrashLocation {
        root: String,
        data_home: String,
        device: u64,
        desktop: bool,
    },
    TrashList {
        state: String,
    },
    TrashRestore {
        state: String,
        identifier: String,
        destination: Option<String>,
    },
    TrashJournal {
        operation: crate::trash_journal::Operation,
    },
    EchoMatches {
        root: String,
        relative: String,
        snapshot: Value,
    },
    ControlReceive {
        fd: i32,
        timeout: f64,
    },
    ControlRespond {
        fd: i32,
        response: Value,
        timeout: f64,
    },
    #[serde(rename = "control_command")]
    ControlExchange {
        path: String,
        action: String,
        job_id: Option<String>,
        timeout: f64,
    },
    Localization {
        operation: crate::localization::Operation,
    },
    TrashSnapshot {
        parent: i32,
        name: String,
    },
    ComputerSetup {
        executable: String,
        state: String,
        root: String,
    },
    StageUpload {
        root: i32,
        relative: String,
        expected: Value,
        folder: String,
    },
    InstallDownload {
        root: i32,
        relative: String,
        source: String,
        expected: Value,
        root_path: String,
    },
    FilesystemRead {
        fd: i32,
        output: Option<i32>,
    },
    FilesystemEntry {
        parent: i32,
        name: String,
    },
    FilesystemScan {
        root: i32,
    },
    EventsQuery {
        executable: String,
        paths: Vec<String>,
        cursors: Value,
    },
    EventsLoad {
        file: String,
        paths: Vec<String>,
    },
    EventsDeliver {
        delivery: crate::event_runtime::Delivery,
        batch: Value,
    },
    EventsPoll {
        executable: String,
        delivery: crate::event_runtime::Delivery,
        cursors: Value,
    },
    Drive {
        executable: String,
        operation: crate::drive::Operation,
    },
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
        Some("my-files" | "trash" | "shared-with-me" | "shared-by-me" | "devices")
    ) {
        return Err("Dieser Cloud-Bereich wird noch nicht unterstützt.".into());
    }
    Ok(path)
}

pub(crate) fn name(value: &str) -> Result<&str> {
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

pub(crate) fn uid(value: &str) -> bool {
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

pub(crate) fn node(item: Value, parent: &str) -> Result<Value> {
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

pub(crate) fn local_path(value: &str, directory: bool) -> Result<PathBuf> {
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

pub(crate) fn scan_upload(root: &Path, callback: &mut impl FnMut(Value) -> i32) -> Result<()> {
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
        Command::DesktopRuntime { config } => crate::desktop_runtime::run(config, callback),
        Command::DesktopService { operation } => {
            crate::desktop_service::run(operation, &mut crate::host::Callback(callback))
        }
        Command::Portal { operation } => crate::portals::run(operation, callback),
        Command::DesktopBus { operation } => crate::desktop_bus::run(operation, callback),
        Command::Autostart { operation } => crate::autostart::run(operation),
        Command::UpdateRuntime { operation } => crate::update_runtime::run(operation, callback),
        Command::EventSource {
            operation,
            executable,
            host_delivery,
        } => crate::event_source::run(operation, executable, host_delivery, callback),
        Command::MountManager {
            operation,
            executable,
        } => crate::mount_manager::run(operation, executable, callback),
        Command::FuseSession {
            operation,
            executable,
        } => crate::fuse::run(operation, executable, callback),
        Command::OndemandFs {
            operation,
            executable,
        } => match executable {
            Some(executable) => crate::ondemand_fs::run(
                *operation,
                &mut crate::sync_remote::Sdk {
                    executable,
                    callback,
                },
            ),
            None => crate::ondemand_fs::run(*operation, &mut crate::host::Callback(callback)),
        },
        Command::OndemandNamespace {
            operation,
            executable,
        } => match executable {
            Some(executable) => crate::ondemand_namespace::run(
                *operation,
                &mut crate::sync_remote::Sdk {
                    executable,
                    callback,
                },
            ),
            None => {
                crate::ondemand_namespace::run(*operation, &mut crate::host::Callback(callback))
            }
        },
        Command::OndemandReplace {
            operation,
            executable,
        } => match executable {
            Some(executable) => crate::ondemand_replace::run(
                *operation,
                &mut crate::sync_remote::Sdk {
                    executable,
                    callback,
                },
            ),
            None => crate::ondemand_replace::run(*operation, &mut crate::host::Callback(callback)),
        },
        Command::OndemandHandles { operation } => crate::ondemand_handles::run(operation),
        Command::OndemandTree { operation } => crate::ondemand_tree::run(operation),
        Command::OndemandWrite {
            operation,
            executable,
        } => match executable {
            Some(executable) => crate::ondemand_write::run(
                *operation,
                &mut crate::sync_remote::Sdk {
                    executable,
                    callback,
                },
            ),
            None => crate::ondemand_write::run(*operation, &mut crate::host::Callback(callback)),
        },
        Command::OndemandIo {
            operation,
            executable,
        } => match executable {
            Some(executable) => crate::ondemand_io::run(
                operation,
                &mut crate::sync_remote::Sdk {
                    executable,
                    callback,
                },
            ),
            None => crate::ondemand_io::run(operation, &mut crate::host::Callback(callback)),
        },
        Command::Daemon { config } => crate::daemon::run(config, callback),
        Command::DaemonControl {
            action,
            job_id,
            reload_only,
        } => {
            let mut host = crate::host::Callback(callback);
            if reload_only {
                crate::daemon_control::reload(&mut host)?;
                Ok(Value::Null)
            } else {
                crate::daemon_control::command(action, job_id, &mut host)
            }
        }
        Command::ControlServer { operation } => {
            crate::control_server::run(operation, &mut crate::host::Callback(callback))
        }
        Command::DaemonTransition {
            runtime,
            outcome,
            context,
        } => crate::daemon_job::transition(*runtime, *outcome, context),
        Command::ServiceController { state, operation } => {
            crate::service_controller::run(Path::new(&state), operation)
        }
        Command::JobRoots { jobs, selected } => {
            crate::job_roots::resolve(jobs, &selected, &mut crate::host::Callback(callback))
        }
        Command::EventAncestry { event, jobs } => {
            crate::event_ancestry::resolve(event, jobs, &mut crate::host::Callback(callback))
        }
        Command::MetadataCache { operation } => {
            crate::metadata_cache::run(operation, &mut crate::host::Callback(callback))
        }
        Command::SyncExecute {
            state,
            plan,
            trash_state,
            trash_data,
            trash_desktop,
            executable,
        } => {
            let request = crate::sync_execute::Request {
                state,
                plan,
                trash_state,
                trash_data,
                trash_desktop,
            };
            if let Some(executable) = executable {
                crate::sync_execute::run(
                    request,
                    &mut crate::sync_remote::Sdk {
                        executable,
                        callback,
                    },
                )
            } else {
                crate::sync_execute::run(request, &mut crate::host::Callback(callback))
            }
        }
        Command::HostFailure {
            id,
            message,
            cancelled,
        } => crate::host::fail(id, message, cancelled),
        Command::SyncPlan {
            state,
            root,
            remote_path,
            executable,
        } => {
            if let Some(executable) = executable {
                crate::sync_plan::run(
                    &state,
                    &root,
                    &remote_path,
                    &mut crate::sync_remote::Sdk {
                        executable,
                        callback,
                    },
                )
            } else {
                crate::sync_plan::run(
                    &state,
                    &root,
                    &remote_path,
                    &mut crate::host::Callback(callback),
                )
            }
        }
        Command::SyncProof {
            operation,
            executable,
        } => {
            if let Some(executable) = executable {
                crate::sync_proof::run(
                    operation,
                    &mut crate::sync_remote::Sdk {
                        executable,
                        callback,
                    },
                )
            } else {
                crate::sync_proof::run(operation, &mut crate::host::Callback(callback))
            }
        }
        Command::SyncState { operation } => crate::sync_state::run(operation),
        Command::HostReply { id, value } => crate::host::reply(id, value),
        Command::SyncRemote {
            operation,
            executable,
        } => {
            if let Some(executable) = executable {
                crate::sync_remote::run(
                    operation,
                    &mut crate::sync_remote::Sdk {
                        executable,
                        callback,
                    },
                )
            } else {
                crate::sync_remote::run(operation, &mut crate::host::Callback(callback))
            }
        }
        Command::TrashInit { state } => {
            crate::storage::directory(Path::new(&state), true, true)?;
            crate::storage::directory(&Path::new(&state).join("entries"), true, true)?;
            Ok(json!(true))
        }
        Command::TrashMove { request } => crate::trash_move::run(request, callback),
        Command::TrashLocation {
            root,
            data_home,
            device,
            desktop,
        } => crate::trash_location::choose(&root, &data_home, device, desktop),
        Command::TrashList { state } => crate::trash_restore::list(&state, callback),
        Command::TrashRestore {
            state,
            identifier,
            destination,
        } => crate::trash_restore::run(&state, &identifier, destination.as_deref(), callback),
        Command::TrashJournal { operation } => crate::trash_journal::run(operation, callback),
        Command::EchoMatches {
            root,
            relative,
            snapshot,
        } => Ok(json!(crate::filesystem::matches_snapshot(
            Path::new(&root),
            &relative,
            snapshot
        ))),
        Command::ControlReceive { fd, timeout } => crate::control::receive(fd, timeout),
        Command::ControlRespond {
            fd,
            response,
            timeout,
        } => crate::control::respond(fd, response, timeout),
        Command::ControlExchange {
            path,
            action,
            job_id,
            timeout,
        } => crate::control::command(Path::new(&path), &action, job_id.as_deref(), timeout),
        Command::Localization { operation } => crate::localization::run(operation),
        Command::TrashSnapshot { parent, name } => {
            crate::filesystem::snapshot(parent, &name, callback)
        }
        Command::ComputerSetup {
            executable,
            state,
            root,
        } => crate::computer_setup::run(executable, Path::new(&state), Path::new(&root), callback),
        Command::StageUpload {
            root,
            relative,
            expected,
            folder,
        } => {
            crate::filesystem::stage_upload(root, &relative, expected, Path::new(&folder), callback)
        }
        Command::InstallDownload {
            root,
            relative,
            source,
            expected,
            root_path,
        } => crate::filesystem::install_download(
            root,
            &relative,
            Path::new(&source),
            expected,
            Path::new(&root_path),
            callback,
        ),
        Command::FilesystemRead { fd, output } => crate::filesystem::read(fd, output, callback),
        Command::FilesystemEntry { parent, name } => {
            crate::filesystem::entry(parent, &name, callback)
        }
        Command::FilesystemScan { root } => crate::filesystem::scan(root, callback),
        Command::EventsQuery {
            executable,
            paths,
            cursors,
        } => crate::event_runtime::query(executable, &paths, cursors, callback),
        Command::EventsLoad { file, paths } => crate::event_runtime::load(Path::new(&file), &paths),
        Command::EventsDeliver { delivery, batch } => {
            crate::event_runtime::deliver(delivery, batch, callback)
        }
        Command::EventsPoll {
            executable,
            delivery,
            cursors,
        } => crate::event_runtime::poll(executable, delivery, cursors, callback),
        Command::Drive {
            executable,
            operation,
        } => crate::drive::run(executable, operation, callback),
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
