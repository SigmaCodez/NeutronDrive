//! Native Qt appearance state and scoped, owned portal subscription.
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use std::{
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
        #[qproperty(QString, mode, READ, NOTIFY)]
        #[qproperty(bool, dark, READ, NOTIFY)]
        type NativeAppearance = super::Appearance;
        #[qinvokable]
        #[cxx_name = "setMode"]
        fn set_mode(self: Pin<&mut NativeAppearance>, mode: &QString);
        #[qinvokable]
        #[cxx_name = "setQtFallback"]
        fn set_qt_fallback(self: Pin<&mut NativeAppearance>, dark: bool);
        #[qinvokable]
        fn shutdown(self: Pin<&mut NativeAppearance>);
        #[qinvokable]
        #[cxx_name = "watcherRunning"]
        fn watcher_running(self: &NativeAppearance) -> bool;
    }
    impl cxx_qt::Initialize for NativeAppearance {}
    impl cxx_qt::Threading for NativeAppearance {}
}
pub struct Appearance {
    mode: QString,
    dark: bool,
    fallback: bool,
    portal: i32,
    closed: bool,
    cancel: Arc<AtomicBool>,
    watcher: Option<JoinHandle<()>>,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            mode: QString::from("system"),
            dark: false,
            fallback: false,
            portal: 0,
            closed: false,
            cancel: Arc::new(AtomicBool::new(false)),
            watcher: None,
        }
    }
}
fn dark_for(mode: &str, portal: i32, fallback: bool) -> bool {
    match mode {
        "dark" => true,
        "light" => false,
        _ => match portal {
            1 => true,
            2 => false,
            _ => fallback,
        },
    }
}
impl Appearance {
    fn stop(&mut self) {
        self.closed = true;
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.watcher.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Appearance {
    fn drop(&mut self) {
        self.stop();
    }
}
impl qobject::NativeAppearance {
    fn refresh(mut self: Pin<&mut Self>) {
        let state = self.rust();
        let dark = dark_for(&state.mode.to_string(), state.portal, state.fallback);
        if state.dark != dark {
            self.as_mut().rust_mut().dark = dark;
            self.as_mut().dark_changed();
        }
    }
    fn set_mode(mut self: Pin<&mut Self>, mode: &QString) {
        if self.rust().closed {
            return;
        }
        let value = mode.to_string();
        let value = if ["system", "light", "dark"].contains(&value.as_str()) {
            value.as_str()
        } else {
            "system"
        };
        let value = QString::from(value);
        if self.rust().mode != value {
            self.as_mut().rust_mut().mode = value;
            self.as_mut().mode_changed();
            self.refresh();
        }
    }
    fn set_qt_fallback(mut self: Pin<&mut Self>, dark: bool) {
        if !self.rust().closed {
            self.as_mut().rust_mut().fallback = dark;
            self.refresh();
        }
    }
    fn portal_changed(mut self: Pin<&mut Self>, scheme: i32) {
        if !self.rust().closed {
            self.as_mut().rust_mut().portal = scheme;
            self.refresh();
        }
    }
    fn shutdown(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().stop();
    }
    fn watcher_running(&self) -> bool {
        self.rust()
            .watcher
            .as_ref()
            .is_some_and(|w| !w.is_finished())
    }
}
impl cxx_qt::Initialize for qobject::NativeAppearance {
    fn initialize(mut self: Pin<&mut Self>) {
        let qt = self.qt_thread();
        let cancel = self.rust().cancel.clone();
        let worker = std::thread::Builder::new()
            .name("qt-appearance".into())
            .spawn(move || {
                let request = br#"{"version":1,"op":"portal","operation":{"action":"appearance"}}"#;
                let _ = neutrondrive_core::runtime_request(request, &mut |event| {
                    if cancel.load(Ordering::Acquire) {
                        return 1;
                    }
                    if event["kind"] == "appearance_changed" {
                        let scheme = event["scheme"].as_i64().unwrap_or(0) as i32;
                        if qt
                            .queue(move |object| object.portal_changed(scheme))
                            .is_err()
                        {
                            return 1;
                        }
                    }
                    0
                });
            });
        // Portal absence or inability to start a thread leaves Qt's fallback usable.
        if let Ok(worker) = worker {
            self.as_mut().rust_mut().watcher = Some(worker);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_mode_overrides_portal_and_unknown_scheme_uses_qt() {
        for portal in [0, 1, 2, 999] {
            for fallback in [true, false] {
                assert!(dark_for("dark", portal, fallback));
                assert!(!dark_for("light", portal, fallback));
                assert_eq!(
                    dark_for("system", portal, fallback),
                    match portal {
                        1 => true,
                        2 => false,
                        _ => fallback,
                    }
                );
            }
        }
    }
}
