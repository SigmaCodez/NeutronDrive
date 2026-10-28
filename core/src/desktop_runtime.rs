//! Native session-bus server and owned online-drive workers. No Python callbacks
//! enter worker threads; the external callback only polls this run invocation.
use crate::{
    desktop_service::{self, Operation as Core},
    drive,
    event_source::{self, Operation as Events},
    host::Services,
    mount_manager::{self, Operation as Mount},
    process::Failure,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
type Result<T> = std::result::Result<T, Failure>;
const PATH: &str = "/io/github/SigmaCodez/NeutronDrive/Service";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub state_dir: PathBuf,
    pub executable: String,
}
struct Worker {
    id: u64,
    thread: Option<JoinHandle<()>>,
}
struct Shared {
    core: u64,
    manager: u64,
    config: Config,
    cancel: AtomicBool,
    stopping: AtomicBool,
    update_prepared: AtomicBool,
    cleaned: AtomicBool,
    mount: Mutex<Option<JoinHandle<()>>>,
    events: Mutex<Option<Worker>>,
}
fn unwrap_mount(value: Value) -> Result<Value> {
    if let Some(message) = value["message"]
        .as_str()
        .filter(|_| value["errno"].is_number())
    {
        return Err(message.into());
    }
    if let Some(message) = value["backend_error"].as_str() {
        return Err(Failure {
            message: message.into(),
            cancelled: value["cancelled"] == true,
        });
    }
    value
        .get("value")
        .cloned()
        .ok_or_else(|| "Ungültiger Laufwerksstatus.".into())
}
fn mount(
    operation: Mount,
    executable: Option<String>,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    unwrap_mount(mount_manager::run(operation, executable, callback)?)
}
struct Native(Arc<Shared>);
impl Native {
    fn mount(&self, operation: Mount) -> Result<Value> {
        mount(
            operation,
            Some(self.0.config.executable.clone()),
            &mut |_| {
                i32::from(
                    self.0.cancel.load(Ordering::Acquire)
                        || self.0.stopping.load(Ordering::Acquire),
                )
            },
        )
    }
    fn events(&self, operation: Events) -> Result<Value> {
        event_source::run(operation, None, false, &mut |_| 0)
    }
}
impl Services for Native {
    fn poll(&mut self) -> Result<()> {
        Ok(())
    }
    fn request(&mut self, method: &str, args: Value) -> Result<Value> {
        let shared = self.0.clone();
        let id = shared.manager;
        match method {
            "manager_status" => self.mount(Mount::Status { id }),
            "manager_mount" => {
                let point = args["mountpoint"]
                    .as_str()
                    .ok_or("Ungültiger Einhängepfad.")?;
                self.mount(Mount::Prepare {
                    id,
                    mountpoint: point.into(),
                    remote_root: args["remote_root"]
                        .as_str()
                        .ok_or("Ungültiger Cloud-Ordner.")?
                        .into(),
                    writable: args["writable"] == true,
                    offline: args["offline"] == true,
                })?;
                let mut slot = shared
                    .mount
                    .lock()
                    .map_err(|_| "Laufwerks-Thread nicht verfügbar.")?;
                if let Some(old) = slot.take() {
                    let _ = old.join();
                }
                let executable = shared.config.executable.clone();
                match thread::Builder::new()
                    .name("neutrondrive-fuse".into())
                    .spawn(move || {
                        let _ = mount(Mount::Run { id }, Some(executable), &mut |_| 0);
                    }) {
                    Ok(worker) => *slot = Some(worker),
                    Err(_) => {
                        let _ = self.mount(Mount::Abort { id });
                        return Err("Laufwerks-Thread konnte nicht gestartet werden.".into());
                    }
                }
                drop(slot);
                self.mount(Mount::Wait { id })
            }
            "manager_unmount" => {
                let result = self.mount(Mount::Unmount { id })?;
                if let Some(worker) = shared
                    .mount
                    .lock()
                    .map_err(|_| "Laufwerks-Thread nicht verfügbar.")?
                    .take()
                {
                    let _ = worker.join();
                }
                Ok(result)
            }
            "manager_pin" => self.mount(Mount::Pin {
                id,
                path: args["path"].as_str().ok_or("Ungültiger Dateipfad.")?.into(),
                pinned: true,
            }),
            "manager_evict" => self.mount(Mount::Evict {
                id,
                path: args["path"].as_str().ok_or("Ungültiger Dateipfad.")?.into(),
            }),
            "manager_refresh" => self.mount(Mount::Refresh { id }),
            "save" => desktop_service::run(
                Core::Save {
                    id: shared.core,
                    value: args,
                },
                self,
            ),
            "reset_cancel" => {
                shared.cancel.store(false, Ordering::Release);
                Ok(Value::Null)
            }
            "login" => {
                let mut callback = |_: Value| {
                    i32::from(
                        shared.cancel.load(Ordering::Acquire)
                            || shared.stopping.load(Ordering::Acquire),
                    )
                };
                drive::run(
                    shared.config.executable.clone(),
                    drive::Operation::Login,
                    &mut callback,
                )?;
                drive::run(
                    shared.config.executable.clone(),
                    drive::Operation::Info {
                        path: "/my-files".into(),
                    },
                    &mut callback,
                )?;
                Ok(Value::Null)
            }
            "events_snapshot" => {
                let slot = shared
                    .events
                    .lock()
                    .map_err(|_| "Ereignis-Thread nicht verfügbar.")?;
                match slot.as_ref() {
                    Some(worker) => {
                        let snapshot = self.events(Events::Snapshot { id: worker.id })?;
                        Ok(
                            json!({"exists":true,"running":snapshot["running"],"status":snapshot["status"]}),
                        )
                    }
                    None => Ok(json!({"exists":false,"running":false})),
                }
            }
            "events_stop" => {
                let event_id = shared
                    .events
                    .lock()
                    .map_err(|_| "Ereignis-Thread nicht verfügbar.")?
                    .as_ref()
                    .map(|w| w.id);
                if let Some(id) = event_id {
                    self.events(Events::Stop { id, timeout: 5.0 })?;
                }
                Ok(Value::Null)
            }
            "events_drop" => {
                let mut slot = shared
                    .events
                    .lock()
                    .map_err(|_| "Ereignis-Thread nicht verfügbar.")?;
                if let Some(worker) = slot.as_ref() {
                    self.events(Events::Forget { id: worker.id })?;
                }
                if let Some(mut worker) = slot.take() {
                    if let Some(thread) = worker.thread.take() {
                        let _ = thread.join();
                    }
                }
                Ok(Value::Null)
            }
            "events_start" => {
                let root = args["root"].as_str().ok_or("Ungültiger Ereignisordner.")?;
                let mut slot = shared
                    .events
                    .lock()
                    .map_err(|_| "Ereignis-Thread nicht verfügbar.")?;
                if slot.is_some() {
                    return Err("Ereignisüberwachung läuft bereits.".into());
                }
                let event_id = self
                    .events(Events::Create {
                        state_dir: shared.config.state_dir.join("events"),
                        roots: vec![root.into()],
                        interval: 30.0,
                        dynamic_roots: false,
                    })?
                    .as_u64()
                    .ok_or("Ungültige Ereignisquelle.")?;
                *slot = Some(Worker {
                    id: event_id,
                    thread: None,
                });
                self.events(Events::Start { id: event_id })?;
                let executable = shared.config.executable.clone();
                let event_shared = shared.clone();
                match thread::Builder::new()
                    .name("neutrondrive-events".into())
                    .spawn(move || {
                        let _ = event_source::run(
                            Events::Run { id: event_id },
                            Some(executable.clone()),
                            true,
                            &mut |value| {
                                if value["kind"] == "host_request" && value["method"] == "event_accept" {
                                    let request_id = value["id"].as_u64().unwrap();
                                    let event = &value["arguments"];
                                    let result = if event["kind"] == "error" {
                                        Ok(Value::Null)
                                    } else {
                                        // Delivery follows the SDK query and its lock release.
                                        // Pinned refreshes need a real SDK, not an empty callback.
                                        mount(
                                            Mount::Invalidate {
                                                id,
                                                root_uid: event["root_uid"].as_str().map(String::from),
                                                kind: event["kind"].as_str().unwrap_or("").into(),
                                            },
                                            Some(executable.clone()),
                                            &mut |_| i32::from(event_shared.stopping.load(Ordering::Acquire)),
                                        ).and_then(|result| {
                                            if result["pending"] == true {
                                                Err("Pinned refresh is pending; retry this cloud event.".into())
                                            } else {
                                                Ok(result)
                                            }
                                        })
                                    };
                                    // Failed refreshes retain their cursor and are retried;
                                    // returning cancellation here would stop the watcher.
                                    match result {
                                        Ok(result) => { let _ = crate::host::reply(request_id, result); }
                                        Err(error) => { let _ = crate::host::fail(request_id, error.message, error.cancelled); }
                                    }
                                }
                                0
                            },
                        );
                    }) {
                    Ok(thread) => slot.as_mut().unwrap().thread = Some(thread),
                    Err(_) => {
                        let _ = self.events(Events::AbortStart { id: event_id });
                        return Err("Ereignis-Thread konnte nicht gestartet werden.".into());
                    }
                }
                Ok(Value::Null)
            }
            _ => Err("Unbekannter nativer Dienstaufruf.".into()),
        }
    }
}
impl Shared {
    fn current(self: &Arc<Self>) -> Result<Value> {
        desktop_service::run(Core::Current { id: self.core }, &mut Native(self.clone()))
    }
    fn status(self: &Arc<Self>) -> Result<Value> {
        desktop_service::run(Core::Status { id: self.core }, &mut Native(self.clone()))
    }
    fn request(self: &Arc<Self>, request: Value) -> Result<Value> {
        desktop_service::run(
            Core::Request {
                id: self.core,
                request,
            },
            &mut Native(self.clone()),
        )
    }
    fn shutdown(self: &Arc<Self>) -> Result<()> {
        if self.cleaned.load(Ordering::Acquire) {
            return Ok(());
        }
        self.stopping.store(true, Ordering::Release);
        self.cancel.store(true, Ordering::Release);
        let mut host = Native(self.clone());
        host.request("events_stop", json!({}))?;
        host.request("events_drop", json!({}))?;
        let unmount = mount(Mount::Unmount { id: self.manager }, None, &mut |_| 0);
        if unmount.is_err() {
            mount(Mount::Abort { id: self.manager }, None, &mut |_| 0)?;
        }
        if let Some(worker) = self
            .mount
            .lock()
            .map_err(|_| "Laufwerks-Thread nicht verfügbar.")?
            .take()
        {
            let _ = worker.join();
        }
        mount(Mount::Forget { id: self.manager }, None, &mut |_| 0)?;
        desktop_service::run(Core::Forget { id: self.core }, &mut host)?;
        self.cleaned.store(true, Ordering::Release);
        Ok(())
    }
}
struct NoHost;
impl Services for NoHost {
    fn poll(&mut self) -> Result<()> {
        Ok(())
    }
    fn request(&mut self, _: &str, _: Value) -> Result<Value> {
        Err("Unerwarteter Dienstaufruf.".into())
    }
}
struct Job {
    request: Value,
    reply: mpsc::SyncSender<Result<Value>>,
}
#[derive(Clone)]
struct Server {
    shared: Arc<Shared>,
    sender: mpsc::SyncSender<Job>,
}
impl Server {
    async fn request(
        &self,
        raw: &str,
        connection: &zbus::Connection,
        header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<String> {
        let invalid = || zbus::fdo::Error::InvalidArgs("Ungültige Dienstanfrage.".into());
        if raw.len() > 16384 {
            return Err(invalid());
        }
        let sender = header.sender().ok_or_else(invalid)?;
        let identity = crate::desktop_bus::bounded(
            connection.call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetConnectionUnixUser",
                &(sender.as_str(),),
            ),
            Duration::from_secs(3),
            &mut |_| 0,
        )
        .await
        .map_err(|_| invalid())?
        .map_err(|_| invalid())?;
        let uid: u32 = identity.body().deserialize().map_err(|_| invalid())?;
        if uid != unsafe { libc::getuid() } {
            return Err(zbus::fdo::Error::AccessDenied(
                "Dienstzugriff verweigert.".into(),
            ));
        }
        let request: Value = serde_json::from_str(raw).map_err(|_| invalid())?;
        let action = request["action"].as_str().ok_or_else(invalid)?;
        let result = if action == "status" {
            self.shared.status()
        } else if action == "finish_update" {
            if !self.shared.update_prepared.load(Ordering::Acquire) {
                return Err(zbus::fdo::Error::Failed(
                    "Update shutdown was not prepared.".into(),
                ));
            }
            self.shared.stopping.store(true, Ordering::Release);
            Ok(json!({}))
        } else if action == "cancel" {
            if self
                .shared
                .current()
                .is_ok_and(|v| v.as_str().is_some_and(|s| !s.is_empty()))
            {
                self.shared.cancel.store(true, Ordering::Release);
            }
            Ok(json!({}))
        } else {
            if self.shared.stopping.load(Ordering::Acquire) {
                return Err(zbus::fdo::Error::Failed("Dienst wird beendet.".into()));
            }
            let (tx, rx) = mpsc::sync_channel(1);
            self.sender
                .try_send(Job { request, reply: tx })
                .map_err(|_| zbus::fdo::Error::LimitsExceeded("Dienst ist ausgelastet.".into()))?;
            let runtime = zruntime::SharedRuntime::current().map_err(|_| invalid())?;
            loop {
                match rx.try_recv() {
                    Ok(value) => break value,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        break Err("Dienstaktion wurde beendet.".into())
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        runtime.sleep(Duration::from_millis(10)).await
                    }
                }
            }
        };
        let response = match result {
            Ok(value) => json!({"ok":true,"result":value}),
            Err(error) => json!({"ok":false,"error":error.message}),
        };
        let response = response.to_string();
        if response.len() > 8 * 1024 * 1024 {
            return Err(zbus::fdo::Error::LimitsExceeded(
                "Dienstantwort ist zu groß.".into(),
            ));
        }
        Ok(response)
    }
}
struct NativeServer(Server);
#[zbus::interface(name = "io.github.SigmaCodez.NeutronDrive.Service")]
impl NativeServer {
    async fn request(
        &self,
        raw: &str,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<String> {
        self.0.request(raw, connection, header).await
    }
}
struct SnapServer(Server);
#[zbus::interface(name = "io.github.SigmaCodez.NeutronDrive.SnapService")]
impl SnapServer {
    async fn request(
        &self,
        raw: &str,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<String> {
        self.0.request(raw, connection, header).await
    }
}
pub fn run(config: Config, callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    if std::env::var_os("FLATPAK_ID").is_some_and(|v| !v.is_empty()) {
        return Err("Der Laufwerkshelfer muss nativ außerhalb von Flatpak laufen.".into());
    }
    let core = desktop_service::run(
        Core::Create {
            state_dir: config.state_dir.clone(),
        },
        &mut NoHost,
    )?
    .as_u64()
    .ok_or("Ungültiger Dienst.")?;
    let manager = match mount(
        Mount::Create {
            state_dir: config.state_dir.clone(),
        },
        None,
        &mut |_| 0,
    ) {
        Ok(value) => value.as_u64().ok_or("Ungültiges Laufwerk.")?,
        Err(error) => {
            let _ = desktop_service::run(Core::Forget { id: core }, &mut NoHost);
            return Err(error);
        }
    };
    let shared = Arc::new(Shared {
        core,
        manager,
        config,
        cancel: AtomicBool::new(false),
        stopping: AtomicBool::new(false),
        update_prepared: AtomicBool::new(false),
        cleaned: AtomicBool::new(false),
        mount: Mutex::new(None),
        events: Mutex::new(None),
    });
    let (sender, receiver) = mpsc::sync_channel::<Job>(16);
    let owned = shared.clone();
    let worker = match thread::Builder::new()
        .name("ondemand-service".into())
        .spawn(move || {
            while !owned.stopping.load(Ordering::Acquire) {
                let job = match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(job) => job,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                if owned.stopping.load(Ordering::Acquire) {
                    let _ = job.reply.send(Err("Dienst wird beendet.".into()));
                    continue;
                }
                if job.request["action"] == "restore" {
                    // Recovery after an interrupted updater can release the reservation.
                    owned.update_prepared.store(false, Ordering::Release);
                }
                if owned.update_prepared.load(Ordering::Acquire) {
                    let _ = job
                        .reply
                        .send(Err("Service is reserved for an update.".into()));
                    continue;
                }
                let prepare = job.request["action"] == "prepare_update";
                let result = owned.request(job.request);
                if prepare && result.is_ok() {
                    owned.update_prepared.store(true, Ordering::Release);
                }
                let _ = job.reply.send(result);
            }
        }) {
        Ok(worker) => worker,
        Err(_) => {
            let _ = shared.shutdown();
            return Err("Dienst-Thread konnte nicht gestartet werden.".into());
        }
    };
    let server = Server {
        shared: shared.clone(),
        sender,
    };
    let result: Result<Value> = zbus::block_on(async {
        let snap = std::env::var_os("SNAP").is_some_and(|v| !v.is_empty());
        let builder = zbus::connection::Builder::session()
            .allow_name_replacements(false)
            .replace_existing_names(false);
        let builder = if snap {
            builder
                .name("io.github.SigmaCodez.NeutronDrive.SnapService")
                .serve_at(PATH, SnapServer(server))
        } else {
            builder
                .name("io.github.SigmaCodez.NeutronDrive.Service")
                .serve_at(PATH, NativeServer(server))
        };
        let connection =
            crate::desktop_bus::bounded(builder.build(), Duration::from_secs(5), callback)
                .await?
                .map_err(|_| "Laufwerksdienst konnte nicht am Sitzungsbus registriert werden.")?;
        // Optional host metadata bridge. It is isolated from the FUSE pump and
        // exits when this service process ends; missing GVfs never blocks sync.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                if let Ok(mut child) =
                    std::process::Command::new(parent.join("neutrondrive-file-manager"))
                        .args(["--watch-emblems", &std::process::id().to_string()])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                {
                    let _ = thread::Builder::new()
                        .name("emblem-helper".into())
                        .spawn(move || {
                            let _ = child.wait();
                        });
                }
            }
        }
        let runtime = zruntime::SharedRuntime::current()
            .map_err(|_| "D-Bus-Laufzeit ist nicht verfügbar.")?;
        while !connection.is_closed() && !shared.stopping.load(Ordering::Acquire) {
            if callback(json!({"kind":"poll"})) != 0 {
                break;
            }
            runtime.sleep(Duration::from_millis(100)).await;
        }
        shared.stopping.store(true, Ordering::Release);
        shared.cancel.store(true, Ordering::Release);
        if shared.update_prepared.load(Ordering::Acquire) {
            shared.shutdown()?;
        }
        let _ = connection.close().await;
        Ok(Value::Null)
    });
    shared.stopping.store(true, Ordering::Release);
    shared.cancel.store(true, Ordering::Release);
    let _ = worker.join();
    let cleanup = shared.shutdown();
    result?;
    cleanup?;
    Ok(Value::Null)
}
