//! Online-drive service policy and serialized operation ownership.
//! Host calls are scoped adapters for mount/event workers while the Qt launcher migrates.
use crate::{
    host::Services,
    jobs,
    process::{self, Failure},
    storage,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
};
type Result<T> = std::result::Result<T, Failure>;
const LIMIT: usize = 16384;
const EVENT_ERROR: &str =
    "Drive-Ereignisse sind nicht verfügbar. Das Laufwerk bleibt über Aktualisieren nutzbar.";
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Create { state_dir: PathBuf },
    Configuration { id: u64 },
    Save { id: u64, value: Value },
    Status { id: u64 },
    Current { id: u64 },
    Request { id: u64, request: Value },
    Forget { id: u64 },
}
struct Core {
    directory: PathBuf,
    serial: Mutex<()>,
    operation: Mutex<String>,
    event_error: Mutex<String>,
}
static CORES: LazyLock<Mutex<BTreeMap<u64, Arc<Core>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static NEXT: AtomicU64 = AtomicU64::new(1);
fn call(host: &mut impl Services, method: &str, args: Value) -> Result<Value> {
    host.request(method, args)
}
impl Core {
    fn configuration(&self) -> Result<Value> {
        let value = storage::read(&self.directory.join("mount.json"), json!({}), LIMIT, true)?;
        let object = value
            .as_object()
            .ok_or("Ungültige Laufwerkskonfiguration.")?;
        if object.keys().any(|k| {
            ![
                "mountpoint",
                "remote_root",
                "writable",
                "offline",
                "restore",
            ]
            .contains(&k.as_str())
        }) {
            return Err("Ungültige Laufwerkskonfiguration.".into());
        }
        // Reject corrupt persisted flags before they can trigger a restore.
        if ["writable", "offline", "restore"]
            .iter()
            .any(|k| object.get(*k).is_some_and(|v| !v.is_boolean()))
            || ["mountpoint", "remote_root"]
                .iter()
                .any(|k| object.get(*k).is_some_and(|v| !v.is_string()))
        {
            return Err("Ungültige Laufwerkskonfiguration.".into());
        }
        Ok(value)
    }
    fn status(&self, host: &mut impl Services) -> Result<Value> {
        let mut status = call(host, "manager_status", json!({}))?;
        if !status.is_object() {
            return Err("Ungültiger Laufwerksstatus.".into());
        }
        status["available"] = json!(true);
        status["configuration"] = self.configuration()?;
        let operation = self
            .operation
            .lock()
            .map_err(|_| "Dienststatus nicht verfügbar.")?
            .clone();
        status["busy"] = json!(!operation.is_empty());
        status["operation"] = json!(operation);
        let events = call(host, "events_snapshot", json!({}))?;
        status["events"] = if events["exists"] == true {
            events["status"].clone()
        } else {
            let error = self
                .event_error
                .lock()
                .map_err(|_| "Ereignisstatus nicht verfügbar.")?
                .clone();
            json!({"state":if error.is_empty() {"stopped"} else {"error"},"error":error})
        };
        Ok(status)
    }
    fn stop_events(&self, host: &mut impl Services) -> Result<()> {
        if call(host, "events_snapshot", json!({}))?["exists"] == true {
            call(host, "events_stop", json!({}))?;
            if call(host, "events_snapshot", json!({}))?["running"] == true {
                return Err("Die Ereignisüberwachung beendet noch einen laufenden Abruf. Bitte erneut versuchen.".into());
            }
            call(host, "events_drop", json!({}))?;
        }
        Ok(())
    }
    fn start_events(&self, host: &mut impl Services, root: &str) -> Result<()> {
        *self
            .event_error
            .lock()
            .map_err(|_| "Ereignisstatus nicht verfügbar.")? = String::new();
        if call(host, "events_start", json!({"root":root})).is_err() {
            *self
                .event_error
                .lock()
                .map_err(|_| "Ereignisstatus nicht verfügbar.")? = EVENT_ERROR.into();
            // Keep an uncooperative live worker reachable so a later stop can retry.
            let _ = self.stop_events(host);
        }
        Ok(())
    }
    fn execute(&self, request: &Value, host: &mut impl Services) -> Result<Value> {
        let action = request["action"]
            .as_str()
            .ok_or("Ungültige Dienstanfrage.")?;
        match action {
            "mount" => {
                let object = request.as_object().ok_or("Ungültige Laufwerksoptionen.")?;
                if object.keys().any(|k| {
                    !["action", "mountpoint", "remote_root", "writable", "offline"]
                        .contains(&k.as_str())
                }) || !request["mountpoint"].is_string()
                    || ["writable", "offline"]
                        .iter()
                        .any(|k| object.get(*k).is_some_and(|v| !v.is_boolean()))
                {
                    return Err("Ungültige Laufwerksoptionen.".into());
                }
                let root = object.get("remote_root").unwrap_or(&Value::Null);
                let root = if object.contains_key("remote_root") {
                    root.as_str().unwrap_or("")
                } else {
                    "/my-files"
                };
                if root != "/my-files" && !root.starts_with("/my-files/") {
                    return Err("Online-Laufwerke benötigen einen Ordner unter /my-files.".into());
                }
                let config = json!({"mountpoint":request["mountpoint"],"remote_root":root,
                    "writable":request.get("writable").unwrap_or(&json!(false)),
                    "offline":request.get("offline").unwrap_or(&json!(false))});
                call(host, "reset_cancel", json!({}))?;
                let point = std::path::absolute(request["mountpoint"].as_str().unwrap())
                    .map_err(|_| "Ungültiger Einhängepfad.")?;
                let items = jobs::transact(
                    &process::data_dir()?.join("background"),
                    &[],
                    jobs::Action::List,
                    &mut |_| 0,
                )?;
                for job in items.as_array().ok_or("Ungültige Sync-Aufträge.")? {
                    let local = std::path::Path::new(
                        job["local_root"]
                            .as_str()
                            .ok_or("Ungültiger Sync-Ordner.")?,
                    );
                    if point.starts_with(local) || local.starts_with(&point) {
                        return Err(
                            "Online-Laufwerk und Sync-Ordner dürfen sich nicht überlappen.".into(),
                        );
                    }
                }
                if call(host, "manager_status", json!({}))?["mounted"] == true {
                    return Err("Es ist bereits ein Online-Laufwerk eingehängt.".into());
                }
                self.stop_events(host)?;
                call(host, "manager_mount", config.clone())?;
                let mut saved = config.clone();
                saved["restore"] = json!(true);
                if let Err(error) = call(host, "save", saved) {
                    call(host, "manager_unmount", json!({}))?;
                    return Err(error);
                }
                if config["offline"] != true {
                    self.start_events(host, root)?;
                }
                self.status(host)
            }
            "unmount" => {
                call(host, "manager_unmount", json!({}))?;
                let mut config = self.configuration()?;
                config["restore"] = json!(false);
                call(host, "save", config)?;
                self.stop_events(host)?;
                self.status(host)
            }
            "pin" | "evict" => {
                let path = request["path"]
                    .as_str()
                    .filter(|p| p.chars().count() <= 4096)
                    .ok_or("Ungültiger relativer Dateipfad.")?;
                call(host, "reset_cancel", json!({}))?;
                call(
                    host,
                    if action == "pin" {
                        "manager_pin"
                    } else {
                        "manager_evict"
                    },
                    json!({"path":path}),
                )?;
                call(host, "manager_status", json!({}))
            }
            "refresh" => {
                call(host, "reset_cancel", json!({}))?;
                call(host, "manager_refresh", json!({}))?;
                call(host, "manager_status", json!({}))
            }
            "restore" => {
                let mut config = self.configuration()?;
                let restore =
                    config.as_object_mut().unwrap().remove("restore") == Some(json!(true));
                if restore && call(host, "manager_status", json!({}))?["mounted"] != true {
                    config["action"] = json!("mount");
                    self.execute(&config, host)
                } else {
                    call(host, "manager_status", json!({}))
                }
            }
            "login" => {
                call(host, "reset_cancel", json!({}))?;
                if call(host, "manager_status", json!({}))?["mounted"] == true {
                    return Err("Vor einem Kontowechsel das Online-Laufwerk aushängen.".into());
                }
                call(host, "login", json!({}))?;
                Ok(json!({"authenticated":true}))
            }
            _ => Err("Unbekannte Dienstaktion.".into()),
        }
    }
    fn request(&self, request: Value, host: &mut impl Services) -> Result<Value> {
        let action = request["action"]
            .as_str()
            .ok_or("Ungültige Dienstanfrage.")?;
        if action == "status" {
            return self.status(host);
        }
        let _serial = self
            .serial
            .lock()
            .map_err(|_| "Dienstaktion nicht verfügbar.")?;
        *self
            .operation
            .lock()
            .map_err(|_| "Dienststatus nicht verfügbar.")? = action.into();
        struct Clear<'a>(&'a Mutex<String>);
        impl Drop for Clear<'_> {
            fn drop(&mut self) {
                if let Ok(mut op) = self.0.lock() {
                    op.clear();
                }
            }
        }
        let _clear = Clear(&self.operation);
        let result = self.execute(&request, host);
        let reset = call(host, "reset_cancel", json!({}));
        let mut result = result?;
        reset?;
        if result.get("busy").is_some() {
            result["busy"] = json!(false);
            result["operation"] = json!("");
        }
        Ok(result)
    }
}
pub fn run(operation: Operation, host: &mut impl Services) -> Result<Value> {
    if let Operation::Create { state_dir } = operation {
        storage::directory(&state_dir, true, true)?;
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        if id == 0 {
            return Err("Dienstidentitäten erschöpft.".into());
        }
        CORES
            .lock()
            .map_err(|_| "Dienstregister nicht verfügbar.")?
            .insert(
                id,
                Arc::new(Core {
                    directory: state_dir,
                    serial: Mutex::new(()),
                    operation: Mutex::new(String::new()),
                    event_error: Mutex::new(String::new()),
                }),
            );
        return Ok(json!(id));
    }
    let id = match &operation {
        Operation::Configuration { id }
        | Operation::Save { id, .. }
        | Operation::Status { id }
        | Operation::Current { id }
        | Operation::Request { id, .. }
        | Operation::Forget { id } => *id,
        Operation::Create { .. } => unreachable!(),
    };
    let core = CORES
        .lock()
        .map_err(|_| "Dienstregister nicht verfügbar.")?
        .get(&id)
        .cloned()
        .ok_or("Unbekannter Dienst.")?;
    match operation {
        Operation::Configuration { .. } => core.configuration(),
        Operation::Save { value, .. } => {
            storage::write(&core.directory.join("mount.json"), value)?;
            Ok(Value::Null)
        }
        Operation::Status { .. } => core.status(host),
        Operation::Current { .. } => Ok(json!(core
            .operation
            .lock()
            .map_err(|_| "Dienststatus nicht verfügbar.")?
            .clone())),
        Operation::Request { request, .. } => core.request(request, host),
        Operation::Forget { .. } => {
            CORES
                .lock()
                .map_err(|_| "Dienstregister nicht verfügbar.")?
                .remove(&id);
            Ok(Value::Null)
        }
        Operation::Create { .. } => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nd-desktop-core-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            storage::directory(&path, true, true).unwrap();
            Self(path)
        }
        fn core(&self) -> Core {
            Core {
                directory: self.0.clone(),
                serial: Mutex::new(()),
                operation: Mutex::new(String::new()),
                event_error: Mutex::new(String::new()),
            }
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[derive(Default)]
    struct Host {
        calls: Vec<String>,
        busy_unmount: bool,
        fail_start: bool,
        running: bool,
    }
    impl Services for Host {
        fn poll(&mut self) -> Result<()> {
            Ok(())
        }
        fn request(&mut self, method: &str, _: Value) -> Result<Value> {
            self.calls.push(method.into());
            match method {
                "events_snapshot" => {
                    Ok(json!({"exists":true,"running":self.running,"status":{"state":"ready"}}))
                }
                "manager_status" => Ok(json!({"mounted":true})),
                "manager_unmount" if self.busy_unmount => Err("busy".into()),
                "events_start" if self.fail_start => Err("private SDK diagnostic".into()),
                _ => Ok(Value::Null),
            }
        }
    }
    #[test]
    fn busy_unmount_preserves_watcher_and_clears_operation() {
        let tmp = Temp::new();
        let core = tmp.core();
        let mut host = Host {
            busy_unmount: true,
            ..Host::default()
        };
        assert_eq!(
            core.request(json!({"action":"unmount"}), &mut host)
                .unwrap_err()
                .message,
            "busy"
        );
        assert_eq!(host.calls, ["manager_unmount", "reset_cancel"]);
        assert!(core.operation.lock().unwrap().is_empty());
    }
    #[test]
    fn running_event_worker_stays_reachable_after_failed_start_or_stop() {
        let tmp = Temp::new();
        let core = tmp.core();
        let mut host = Host {
            fail_start: true,
            running: true,
            ..Host::default()
        };
        core.start_events(&mut host, "/my-files").unwrap();
        assert_eq!(*core.event_error.lock().unwrap(), EVENT_ERROR);
        assert!(!host.calls.iter().any(|m| m == "events_drop"));
        assert!(core.stop_events(&mut host).is_err());
        host.running = false;
        core.stop_events(&mut host).unwrap();
        assert_eq!(host.calls.last().unwrap(), "events_drop");
    }
    #[test]
    fn corrupt_restore_flags_never_activate_mounts() {
        let tmp = Temp::new();
        let core = tmp.core();
        let mut host = Host::default();
        for value in [
            json!({"restore":"false"}),
            json!({"restore":true,"mountpoint":7}),
            json!({"command":"sh"}),
        ] {
            storage::write(&tmp.0.join("mount.json"), value).unwrap();
            assert!(core
                .request(json!({"action":"restore"}), &mut host)
                .is_err());
        }
        assert!(host.calls.iter().all(|m| m == "reset_cancel"));
        assert!(core.operation.lock().unwrap().is_empty());
    }
    #[test]
    fn status_does_not_wait_for_mutation_lock() {
        let tmp = Temp::new();
        let core = Arc::new(tmp.core());
        let _held = core.serial.lock().unwrap();
        *core.operation.lock().unwrap() = "pin".into();
        let reader = core.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker =
            std::thread::spawn(move || tx.send(reader.status(&mut Host::default())).unwrap());
        let status = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap();
        assert_eq!(status["busy"], true);
        assert_eq!(status["operation"], "pin");
        worker.join().unwrap();
    }
}
