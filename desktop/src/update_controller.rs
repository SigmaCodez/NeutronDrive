//! Qt-thread state, owned worker and explicit shutdown for native updates.
use crate::{
    localization::Localization,
    platform::qt,
    preferences::Store,
    runtime::Failure,
    updates::{self, Model, Outcome, Task},
};
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::json;
use std::{
    path::Path,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{SystemTime, UNIX_EPOCH},
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
        type NativeUpdates = super::UpdateController;
        #[qsignal]
        fn notice(self: Pin<&mut NativeUpdates>, message: QString);
        #[qsignal]
        #[cxx_name = "restartRequested"]
        fn restart_requested(self: Pin<&mut NativeUpdates>);
        #[qinvokable]
        fn configure(
            self: Pin<&mut NativeUpdates>,
            catalog: &QString,
            language: &QString,
            payload: &QString,
            demo: bool,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "setOption"]
        fn set_option(self: Pin<&mut NativeUpdates>, name: &QString, value: bool);
        #[qinvokable]
        fn dispatch(self: Pin<&mut NativeUpdates>, action: &QString, owner_busy: bool);
        #[qinvokable]
        fn tick(self: Pin<&mut NativeUpdates>);
        #[qinvokable]
        fn shutdown(self: Pin<&mut NativeUpdates>) -> bool;
        #[qinvokable]
        #[cxx_name = "workerRunning"]
        fn worker_running(self: &NativeUpdates) -> bool;
    }
    impl cxx_qt::Threading for NativeUpdates {}
}
pub struct UpdateController {
    state_json: QString,
    store: Store,
    model: Option<Model>,
    locale: Option<Localization>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Default for UpdateController {
    fn default() -> Self {
        Self {
            state_json: QString::from("{}"),
            store: Store::interface(),
            model: None,
            locale: None,
            cancel: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }
}
impl Drop for UpdateController {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        // A privileged installer, once launched, must finish before its package is removed.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(model) = self.model.as_mut() {
            model.close();
        }
    }
}
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
impl qobject::NativeUpdates {
    fn configure(
        mut self: Pin<&mut Self>,
        catalog: &QString,
        language: &QString,
        payload: &QString,
        demo: bool,
    ) -> bool {
        if self.rust().model.is_some() {
            return false;
        }
        let Ok(locale) = Localization::load(Path::new(&catalog.to_string()), &language.to_string())
        else {
            return false;
        };
        let Ok(format) = updates::call(
            json!({"action":"install_format","payload":payload.to_string()}),
            &mut |_| 0,
        ) else {
            return false;
        };
        let mut model = Model::new(format.as_str().unwrap_or("other").into(), demo);
        let store = &self.rust().store;
        model.automatic = store
            .read("updates/automatic", "true")
            .eq_ignore_ascii_case("true");
        model.auto_download = store
            .read("updates/download", "false")
            .eq_ignore_ascii_case("true");
        model.previews = store
            .read(
                "updates/previews",
                if model.previews { "true" } else { "false" },
            )
            .eq_ignore_ascii_case("true");
        model.last_attempt = store
            .read("updates/lastAttempt", "0")
            .parse()
            .unwrap_or(0.0);
        self.as_mut().rust_mut().locale = Some(locale);
        self.as_mut().rust_mut().model = Some(model);
        self.publish();
        true
    }
    fn publish(mut self: Pin<&mut Self>) {
        let Some(model) = self.rust().model.as_ref() else {
            return;
        };
        let status = self
            .rust()
            .locale
            .as_ref()
            .unwrap()
            .translate(&model.status)
            .replace("{p0}", model.release["tag"].as_str().unwrap_or(""));
        let value = json!({"automatic":model.automatic,"autoDownload":model.auto_download,"previews":model.previews,"busy":model.active.is_some(),"installing":matches!(model.active,Some(Task::Install{..}|Task::Restart(_))),"restartPending":model.restart.is_some(),"status":status,"packageFormat":model.format,"available":!model.release.is_null(),"downloadable":model.downloadable(),"ready":model.download.is_some()});
        self.as_mut().rust_mut().state_json = QString::from(value.to_string());
        self.state_json_changed();
    }
    fn set_option(mut self: Pin<&mut Self>, name: &QString, value: bool) {
        let name = name.to_string();
        if !["automatic", "download", "previews"].contains(&name.as_str())
            || self
                .rust()
                .model
                .as_ref()
                .is_none_or(|m| m.closed || m.active.is_some())
        {
            return;
        }
        let current = self.rust().model.as_ref().unwrap();
        let previous = match name.as_str() {
            "automatic" => current.automatic,
            "download" => current.auto_download,
            _ => current.previews,
        };
        let saved = self.as_mut().rust_mut().store.save(
            &format!("updates/{name}"),
            if value { "true" } else { "false" },
            if previous { "true" } else { "false" },
        );
        let mut model = self.as_mut().rust_mut();
        let model = model.model.as_mut().unwrap();
        if saved {
            model.option(&name, value);
        } else {
            model.status = "Einstellung konnte nicht gespeichert werden.".into();
        }
        self.publish();
    }
    fn tick(mut self: Pin<&mut Self>) {
        if self.rust().model.as_ref().is_some_and(|m| m.due(now())) {
            self.as_mut().dispatch(&QString::from("check"), false);
        }
    }
    fn dispatch(mut self: Pin<&mut Self>, action: &QString, owner_busy: bool) {
        let action = action.to_string();
        let Some(model) = self.rust().model.as_ref() else {
            return;
        };
        if model.closed {
            return;
        }
        if action == "openRelease" {
            if let Some(url) = model.release["url"].as_str() {
                qt::open_url(&QString::from(url));
            }
            return;
        }
        let time = now();
        let task = self
            .as_mut()
            .rust_mut()
            .model
            .as_mut()
            .unwrap()
            .begin(&action, owner_busy, time);
        if task.is_some() && action == "check" {
            self.as_mut()
                .rust_mut()
                .store
                .save("updates/lastAttempt", &time.to_string(), "0");
        }
        self.as_mut().publish();
        if let Some(task) = task {
            self.start(task);
        }
    }
    fn start(mut self: Pin<&mut Self>, task: Task) {
        if let Some(worker) = self.as_mut().rust_mut().worker.take() {
            let _ = worker.join();
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.as_mut().rust_mut().cancel = cancel.clone();
        let cache = std::path::PathBuf::from(qt::cache_location().to_string()).join("updates");
        let qt = self.qt_thread();
        let worker = std::thread::Builder::new()
            .name("qt-updates".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    updates::execute(&task, &cache, &mut |_| {
                        i32::from(cancel.load(Ordering::Acquire))
                    })
                }))
                .unwrap_or_else(|_| Err(updates::failure("Native update worker failed")));
                // A rejected/late Qt callback drops its owned download automatically.
                let _ = qt.queue(move |object| object.complete(result));
            });
        match worker {
            Ok(worker) => self.as_mut().rust_mut().worker = Some(worker),
            Err(_) => self.complete(Err(updates::failure("Could not start update worker"))),
        }
    }
    fn complete(mut self: Pin<&mut Self>, result: Result<Outcome, Failure>) {
        let installed = matches!(&result, Ok(Outcome::Installed(_)));
        let relaunch = match &result {
            Ok(Outcome::Restarted(plan)) => Some(plan.clone()),
            _ => None,
        };
        let mut binding = self.as_mut().rust_mut();
        let Some(model) = binding.model.as_mut() else {
            return;
        };
        let announce = matches!(&result,Ok(Outcome::Checked(v)) if !v.is_null()) && !model.closed;
        let download = model.finish(result);
        if model.closed {
            return;
        }
        self.as_mut().publish();
        if announce {
            let state: serde_json::Value =
                serde_json::from_str(&self.rust().state_json.to_string()).unwrap();
            self.as_mut()
                .notice(QString::from(state["status"].as_str().unwrap_or("")));
        }
        if let Some(plan) = relaunch {
            crate::update_restart::request_relaunch(plan);
            self.as_mut().restart_requested();
        } else if installed {
            self.dispatch(&QString::from("restart"), false);
        } else if download {
            self.dispatch(&QString::from("download"), false);
        }
    }
    fn shutdown(mut self: Pin<&mut Self>) -> bool {
        // The window must defer close until explicit installation has finished.
        if self
            .rust()
            .model
            .as_ref()
            .is_some_and(|m| matches!(m.active, Some(Task::Install { .. } | Task::Restart(_))))
        {
            return false;
        }
        self.rust().cancel.store(true, Ordering::Release);
        if let Some(worker) = self.as_mut().rust_mut().worker.take() {
            let _ = worker.join();
        }
        if let Some(model) = self.as_mut().rust_mut().model.as_mut() {
            model.close();
        }
        true
    }
    fn worker_running(&self) -> bool {
        self.rust()
            .worker
            .as_ref()
            .is_some_and(|w| !w.is_finished())
    }
}
