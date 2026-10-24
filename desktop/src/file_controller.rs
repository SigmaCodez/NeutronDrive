//! Owned worker boundary for the Rust file controller, with Qt-thread-only state changes.
use crate::{
    files::{Effect, Files, Target, Work},
    localization::Localization,
    runtime::{self, Failure},
};
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::{json, Value};
use std::{
    path::Path,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
};
#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, state_json, READ, NOTIFY, cxx_name = "stateJson")]
        #[qproperty(QString, files_json, READ, NOTIFY, cxx_name = "filesJson")]
        type NativeFiles = super::Controller;
        #[qsignal]
        #[cxx_name = "dialogRequested"]
        fn dialog_requested(self: Pin<&mut NativeFiles>, value: QString);
        #[qsignal]
        #[cxx_name = "clipboardRequested"]
        fn clipboard_requested(self: Pin<&mut NativeFiles>, value: QString);
        #[qsignal]
        #[cxx_name = "windowAction"]
        fn window_action(self: Pin<&mut NativeFiles>, action: QString);
        #[qinvokable]
        fn notify(self: Pin<&mut NativeFiles>, message: &QString, error: bool);
        #[qinvokable]
        #[cxx_name = "snapshotWindow"]
        fn snapshot_window(self: &NativeFiles, name: &QString, path: &QString) -> bool;
        #[qinvokable]
        #[cxx_name = "windowKey"]
        fn window_key(self: &NativeFiles, key: i32, control: bool) -> bool;
        #[qinvokable]
        #[cxx_name = "clipboardText"]
        fn clipboard_text(self: &NativeFiles) -> QString;
        #[qinvokable]
        #[cxx_name = "windowClick"]
        fn window_click(self: &NativeFiles, x: f64, y: f64, double_click: bool) -> bool;
        #[qinvokable]
        fn configure(
            self: Pin<&mut NativeFiles>,
            catalog: &QString,
            language: &QString,
            executable: &QString,
            demo: bool,
        ) -> bool;
        #[qinvokable]
        fn dispatch(self: Pin<&mut NativeFiles>, method: &QString, arguments: &QString);
        #[qinvokable]
        fn cancel(self: Pin<&mut NativeFiles>);
        #[qinvokable]
        fn shutdown(self: Pin<&mut NativeFiles>);
        #[qinvokable]
        #[cxx_name = "workerRunning"]
        fn worker_running(self: &NativeFiles) -> bool;
    }
    impl cxx_qt::Threading for NativeFiles {}
}
pub struct Controller {
    state_json: QString,
    files_json: QString,
    model: Option<Files>,
    executable: String,
    demo: bool,
    closed: bool,
    worker: Option<JoinHandle<()>>,
    status_worker: Option<JoinHandle<()>>,
    status_pending: bool,
    status_cancel: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    files_revision: u64,
}
impl Default for Controller {
    fn default() -> Self {
        Self {
            state_json: QString::from("{}"),
            files_json: QString::from("[]"),
            model: None,
            executable: String::new(),
            demo: false,
            closed: false,
            worker: None,
            status_worker: None,
            status_pending: false,
            status_cancel: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            files_revision: 0,
        }
    }
}
impl Controller {
    fn stop(&mut self) {
        self.closed = true;
        self.cancel.store(true, Ordering::Release);
        self.status_cancel.store(true, Ordering::Release);
        if let Some(worker) = self.status_worker.take() {
            let _ = worker.join();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.stop();
    }
}
impl qobject::NativeFiles {
    fn clipboard_text(&self) -> QString {
        crate::platform::qt::clipboard_text()
    }
    fn window_key(&self, key: i32, control: bool) -> bool {
        (self.rust().demo || std::env::args().nth(1).as_deref() == Some("--check-window-fixture"))
            && crate::platform::qt::window_key(&QString::from("mainWindow"), key, control)
    }
    fn window_click(&self, x: f64, y: f64, double_click: bool) -> bool {
        (self.rust().demo || std::env::args().nth(1).as_deref() == Some("--check-window-fixture"))
            && crate::platform::qt::window_click(&QString::from("mainWindow"), x, y, double_click)
    }
    fn snapshot_window(&self, name: &QString, path: &QString) -> bool {
        crate::platform::qt::snapshot_window(name, path)
    }
    fn notify(mut self: Pin<&mut Self>, message: &QString, error: bool) {
        if self.rust().closed {
            return;
        }
        if let Some(model) = self.as_mut().rust_mut().model.as_mut() {
            model.message(&message.to_string(), error);
        }
        self.publish();
    }
    fn configure(
        mut self: Pin<&mut Self>,
        catalog: &QString,
        language: &QString,
        executable: &QString,
        demo: bool,
    ) -> bool {
        if self.rust().closed || self.rust().model.is_some() {
            return false;
        }
        let Ok(locale) = Localization::load(Path::new(&catalog.to_string()), &language.to_string())
        else {
            return false;
        };
        let mut model = Files::new(locale, &executable.to_string(), demo);
        let store = crate::preferences::Store::interface();
        model.state["setupNeeded"] = json!(!demo && store.read("setup", "") != "complete");
        model.state["setupFolder"] = json!(std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default()
            .join("ProtonDrive"));
        self.as_mut().rust_mut().model = Some(model);
        self.as_mut().rust_mut().executable = executable.to_string();
        self.as_mut().rust_mut().demo = demo;
        self.publish();
        true
    }
    fn publish(mut self: Pin<&mut Self>) {
        let Some(model) = self.rust().model.as_ref() else {
            return;
        };
        let state = QString::from(model.state.to_string());
        let revision = model.files_revision;
        let entries = QString::from(model.entries.to_string());
        if self.rust().state_json != state {
            self.as_mut().rust_mut().state_json = state;
            self.as_mut().state_json_changed();
        }
        if self.rust().files_revision != revision {
            self.as_mut().rust_mut().files_revision = revision;
            self.as_mut().rust_mut().files_json = entries;
            self.files_json_changed();
        }
    }
    fn dispatch(mut self: Pin<&mut Self>, method: &QString, args: &QString) {
        if self.rust().closed {
            return;
        }
        let Ok(arguments) = serde_json::from_str::<Value>(&args.to_string()) else {
            return;
        };
        if !arguments.is_array() {
            return;
        }
        if method.to_string() == "action" && arguments[0] == "cancel" {
            self.cancel();
            return;
        }
        let effects = match self.as_mut().rust_mut().model.as_mut() {
            Some(model) => model.dispatch(&method.to_string(), &arguments),
            None => return,
        };
        self.as_mut().publish();
        self.effects(effects);
    }
    fn effects(mut self: Pin<&mut Self>, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Dialog(value) => self
                    .as_mut()
                    .dialog_requested(QString::from(value.to_string())),
                Effect::Clipboard(value) => {
                    crate::platform::qt::set_clipboard(&QString::from(value.as_str()));
                    self.as_mut().clipboard_requested(QString::from(value));
                }
                Effect::Cancel => self.as_mut().cancel(),
                Effect::Close => self.as_mut().window_action(QString::from("close")),
                Effect::Hide => self.as_mut().window_action(QString::from("hide")),
                Effect::OpenUrl(url) => {
                    if !crate::platform::qt::open_url(&QString::from(url.as_str())) {
                        if let Some(model) = self.as_mut().rust_mut().model.as_mut() {
                            let message = format!(
                                "{}{}",
                                model
                                    .locale
                                    .translate("Browser konnte nicht geöffnet werden: "),
                                url.split('?').next().unwrap_or("")
                            );
                            model.message(&message, true);
                        }
                        self.as_mut().publish();
                    }
                }
                Effect::PollService(target) => self.as_mut().poll_service(target),
                Effect::Work(work) => self.as_mut().start(work),
            }
        }
    }
    fn poll_service(mut self: Pin<&mut Self>, target: Target) {
        if self.rust().closed || self.rust().status_pending {
            return;
        }
        if let Some(worker) = self.as_mut().rust_mut().status_worker.take() {
            let _ = worker.join();
        }
        let Some(model) = self.rust().model.as_ref() else {
            return;
        };
        let generation = model.service_generation();
        let qt = self.qt_thread();
        let executable = self.rust().executable.clone();
        let cancel = self.rust().status_cancel.clone();
        self.as_mut().rust_mut().status_pending = true;
        let worker = std::thread::Builder::new()
            .name("qt-status".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut callback = |_| i32::from(cancel.load(Ordering::Acquire));
                    match target {
                        Target::Background => crate::background::run(
                            &executable,
                            &json!({"action":"status"}),
                            &mut callback,
                        ),
                        Target::Mount => runtime::call(
                            crate::headless::desktop_request(
                                &executable,
                                json!({"action":"status"}),
                            )?,
                            &mut callback,
                        ),
                        _ => Err(Failure {
                            message: "Invalid polling target".into(),
                            cancelled: false,
                        }),
                    }
                }))
                .unwrap_or_else(|_| {
                    Err(Failure {
                        message: "Status worker failed".into(),
                        cancelled: false,
                    })
                });
                let _ = qt.queue(move |mut object| {
                    object.as_mut().rust_mut().status_pending = false;
                    if !object.rust().closed {
                        if let (Some(model), Ok(value)) =
                            (object.as_mut().rust_mut().model.as_mut(), result)
                        {
                            model.service_polled(target, generation, &value);
                        }
                        object.publish();
                    }
                });
            });
        match worker {
            Ok(worker) => self.as_mut().rust_mut().status_worker = Some(worker),
            Err(_) => self.as_mut().rust_mut().status_pending = false,
        }
    }
    fn start(mut self: Pin<&mut Self>, work: Work) {
        if let Some(worker) = self.as_mut().rust_mut().worker.take() {
            let _ = worker.join();
        }
        let id = work.id;
        let cancel = Arc::new(AtomicBool::new(false));
        self.as_mut().rust_mut().cancel = cancel.clone();
        let qt = self.qt_thread();
        let executable = self.rust().executable.clone();
        let demo = self.rust().demo;
        let worker=std::thread::Builder::new().name("qt-drive".into()).spawn(move || {
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut callback=|event:Value| {
                    if cancel.load(Ordering::Acquire) {return 1;}
                    if event["kind"] == "login" {
                        if let Some(url)=event["url"].as_str() {
                            let url=url.to_string();
                            if qt.queue(move |object| object.login_ready(id,url)).is_err() {return 1;}
                        }
                    }
                    0
                };
                if demo {demo_operation(&work.operation)} else if work.target == Target::Background {
                    crate::background::run(&executable,&work.operation,&mut callback)
                } else if work.target == Target::Mount && matches!(work.operation["action"].as_str(),Some("pin_remote"|"evict_remote")) {
                    crate::setup::availability(&executable,&work.operation,&mut callback)
                } else if work.target == Target::Mount && work.operation["action"] == "setup" {
                    crate::setup::run(&executable,&work.operation,&mut callback)
                } else if work.target == Target::Mount {
                    runtime::call(crate::headless::desktop_request(&executable,work.operation)?,&mut callback)
                } else if work.target == Target::Autostart {
                    crate::autostart::run(&executable,work.operation["enabled"] == true,&mut callback)
                } else if work.target == Target::Trash {
                    crate::local_trash::run(&work.operation,&mut callback)
                } else if work.operation["action"] == "logout" {
                    crate::account::logout(&executable,&mut callback)
                } else {
                    runtime::call(json!({"op":"drive","executable":executable,"operation":work.operation}),&mut callback)
                }
            })).unwrap_or_else(|_|Err(Failure{message:"Native worker failed; no success assumed".into(),cancelled:false}));
            let _=qt.queue(move |object| object.complete(id,result));
        });
        match worker {
            Ok(worker) => self.as_mut().rust_mut().worker = Some(worker),
            Err(_) => self.complete(
                id,
                Err(Failure {
                    message: "Could not start native worker".into(),
                    cancelled: false,
                }),
            ),
        }
    }
    fn login_ready(mut self: Pin<&mut Self>, id: u64, url: String) {
        if self.rust().closed {
            return;
        }
        if let Some(model) = self.as_mut().rust_mut().model.as_mut() {
            model.login_ready(id, url);
        }
        self.publish();
    }
    fn complete(mut self: Pin<&mut Self>, id: u64, result: Result<Value, Failure>) {
        if self.rust().closed {
            return;
        }
        let effects = self
            .as_mut()
            .rust_mut()
            .model
            .as_mut()
            .map(|m| m.complete(id, result))
            .unwrap_or_default();
        self.as_mut().publish();
        self.effects(effects);
    }
    fn cancel(self: Pin<&mut Self>) {
        self.rust().cancel.store(true, Ordering::Release);
    }
    fn shutdown(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().stop();
    }
    fn worker_running(&self) -> bool {
        self.rust()
            .worker
            .as_ref()
            .is_some_and(|w| !w.is_finished())
            || self
                .rust()
                .status_worker
                .as_ref()
                .is_some_and(|w| !w.is_finished())
    }
}
fn demo_operation(operation: &Value) -> Result<Value, Failure> {
    match operation["action"].as_str().unwrap_or("") {
        "version" => Ok(json!(
            "Demo-Modus · keine Verbindung und keine Übertragungen"
        )),
        "account_storage" => Ok(json!({"total":10737418240_u64,"used":3221225472_u64})),
        "photo_library" => match operation["request"]["action"].as_str().unwrap_or("") {
            "index" => {
                Ok(json!({"items":[{"uid":"demo-photo-one"},{"uid":"demo-photo-two"}],"total":2}))
            }
            "details" => Ok(json!({"photos":[
                {"uid":"demo-photo-one","name":"Sample image 1.jpg","safe":true,"location":{"latitude":46.6,"longitude":8.2},"thumbnail":format!("data:image/jpeg;base64,{}",include_str!("../data/demo-photo.base64").trim())},
                {"uid":"demo-photo-two","name":"Sample image 2.jpg","safe":true,"thumbnail":""}]})),
            "albums" => Ok(
                json!({"albums":[{"uid":"demo-album","name":"Mountain memories","safe":true,"count":2}]}),
            ),
            _ => Err(Failure {
                message: "Demo changes are disabled".into(),
                cancelled: false,
            }),
        },
        "photos" => Ok(json!({"offset":operation["offset"],"more":false,"photos":[
            {"uid":"demo-photo-one","name":"Sample image 1.jpg","safe":true,"thumbnail":format!("data:image/jpeg;base64,{}",include_str!("../data/demo-photo.base64").trim())},
            {"uid":"demo-photo-two","name":"Sample image 2.jpg","safe":true,"thumbnail":""}]})),
        "versions" => Ok(json!({"active":"demo-current","revisions":[
            {"uid":"demo-current","state":"active","created":"2026-10-24T08:00:00Z","size":4096},
            {"uid":"demo-old","state":"superseded","created":"2026-10-23T08:00:00Z","size":2048}]})),
        "list" if operation["path"] != "/my-files" => Ok(json!([])),
        "list" => Ok(json!([
            {"uid":"demo-docs","name":"Documents","type":"folder","path":"/my-files/Documents","safe":true},
            {"uid":"demo-photos","name":"Photography","type":"folder","path":"/my-files/Photography","safe":true},
            {"uid":"demo-project","name":"Projects","type":"folder","path":"/my-files/Projects","safe":true},
            {"uid":"demo-one","name":"Project overview.pdf","type":"file","size":2483200,"modified":"2026-10-20T12:00:00Z","path":"/my-files/Project overview.pdf","safe":true},
            {"uid":"demo-two","name":"Mountain lake.jpg","type":"file","size":8514432,"modified":"2026-10-18T09:30:00Z","path":"/my-files/Mountain lake.jpg","safe":true},
            {"uid":"demo-three","name":"Notes.md","type":"file","size":4096,"modified":"2026-10-17T15:45:00Z","path":"/my-files/Notes.md","safe":true}
        ])),
        _ => Err(Failure {
            message:
                "Die Demo zeigt Beispieldaten. Für echte Aktionen NeutronDrive ohne --demo starten."
                    .into(),
            cancelled: false,
        }),
    }
}
