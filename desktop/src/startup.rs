//! Native GUI launch configuration and lifetime of the per-user activation endpoint.
use crate::{
    headless,
    instance::Instance,
    runtime::{self, Failure},
};
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    pin::Pin,
    sync::OnceLock,
};
static CONFIG: OnceLock<Value> = OnceLock::new();
fn failure(message: &str) -> Failure {
    Failure {
        message: message.into(),
        cancelled: false,
    }
}
fn options(args: &[String]) -> Result<(bool, bool, Option<String>), Failure> {
    let mut demo = false;
    let mut tray = false;
    let mut cli = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--demo" => demo = true,
            "--tray" => tray = true,
            "--cli" => {
                let value = args
                    .next()
                    .filter(|v| !v.is_empty() && !v.starts_with("--"))
                    .ok_or_else(|| failure("Missing backend executable"))?;
                cli = Some(value.clone());
            }
            _ => return Err(failure("Unknown graphical application option")),
        }
    }
    Ok((demo, tray, cli))
}
pub(crate) fn catalog_at(executable: &Path) -> Option<PathBuf> {
    let parent = executable.parent()?;
    [
        parent.join("locales"),
        parent.join("../locales"),
        parent.join("neutrondrive/locales"),
        parent.join("../neutrondrive/locales"),
        parent.join("../share/neutrondrive/locales"),
        parent.join("../lib/neutrondrive/locales"),
        parent.join("../lib/neutrondrive/neutrondrive/locales"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/neutrondrive/locales"),
    ]
    .into_iter()
    .find(|p| p.join("en.json").is_file())
    .and_then(|p| p.canonicalize().ok())
}
pub fn configure(args: &[String], check: bool) -> Result<(), Failure> {
    let fixture = check && args.first().is_some_and(|a| a == "--check-window-fixture");
    let file_action = if args.first().is_some_and(|s| s == "--file-action") {
        Some(crate::file_manager::request(args)?)
    } else {
        None
    };
    let (demo, tray, cli) = if file_action.is_some() {
        (false, false, None)
    } else if check {
        (!fixture, false, None)
    } else {
        options(args)?
    };
    let app = std::env::current_exe().map_err(|_| failure("Application path unavailable"))?;
    let catalog = if check {
        PathBuf::from(
            args.get(1)
                .ok_or_else(|| failure("Missing catalog directory"))?,
        )
    } else {
        catalog_at(&app).ok_or_else(|| failure("Application language catalogs not found"))?
    };
    let sdk = if fixture {
        args.get(2)
            .cloned()
            .ok_or_else(|| failure("Missing fixture executable"))?
    } else if check {
        String::new()
    } else {
        headless::executable(cli.as_deref())
    };
    let payload = catalog
        .parent()
        .ok_or_else(|| failure("Invalid catalog location"))?;
    let available = Path::new(&sdk).is_file()
        && std::ffi::CString::new(sdk.as_bytes())
            .is_ok_and(|path| unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 });
    CONFIG
        .set(json!({"fileAction":file_action,"tray":tray,"demo":demo,"catalog":catalog,"executable":sdk,"available":available,"payload":payload,"flatpak":std::env::var_os("FLATPAK_ID").is_some()}))
        .map_err(|_| failure("Application already configured"))
}
#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, config_json, READ, CONSTANT, cxx_name = "configJson")]
        #[qproperty(QString, error, READ, NOTIFY)]
        type NativeStartup = super::Startup;
        #[qinvokable]
        #[cxx_name = "trayAvailable"]
        fn tray_available(self: &NativeStartup) -> bool;
        #[qinvokable]
        fn claim(self: Pin<&mut NativeStartup>) -> i32;
        #[qinvokable]
        #[cxx_name = "activationPending"]
        fn activation_pending(self: &NativeStartup) -> bool;
        #[qinvokable]
        #[cxx_name = "nextFileAction"]
        fn next_file_action(self: &NativeStartup) -> QString;
        #[qinvokable]
        fn shutdown(self: Pin<&mut NativeStartup>);
        #[qinvokable]
        #[cxx_name = "installTranslations"]
        fn install_translations(self: Pin<&mut NativeStartup>, language: &QString);
    }
}
pub struct Startup {
    config_json: QString,
    error: QString,
    instance: Option<Instance>,
    translators: Vec<cxx::UniquePtr<crate::platform::qt::QTranslator>>,
}
impl Default for Startup {
    fn default() -> Self {
        Self {
            config_json: QString::from(CONFIG.get().unwrap_or(&Value::Null).to_string()),
            error: QString::default(),
            instance: None,
            translators: Vec::new(),
        }
    }
}
impl qobject::NativeStartup {
    fn tray_available(&self) -> bool {
        crate::platform::qt::system_tray_available()
    }
    fn install_translations(mut self: Pin<&mut Self>, language: &QString) {
        use crate::platform::qt;
        if !self.rust().translators.is_empty() {
            return;
        }
        for domain in ["qtbase", "qtdeclarative"] {
            let mut translator = qt::new_translator();
            if qt::load_translation(translator.pin_mut(), language, &QString::from(domain))
                && qt::install_translation(translator.pin_mut())
            {
                self.as_mut().rust_mut().translators.push(translator);
            }
        }
    }
    fn claim(mut self: Pin<&mut Self>) -> i32 {
        if self.rust().instance.is_some() {
            return 1;
        }
        let result = (|| {
            let config = CONFIG
                .get()
                .ok_or_else(|| failure("Application not configured"))?;
            let root = runtime::value(json!({"op":"data_dir"}))?;
            let scope = if config["demo"] == true {
                "demo"
            } else if std::env::var_os("FLATPAK_ID").is_some_and(|v| !v.is_empty()) {
                "flatpak"
            } else if std::env::var_os("SNAP").is_some_and(|v| !v.is_empty()) {
                "snap"
            } else {
                "native"
            };
            Instance::claim_mode(
                &Path::new(
                    root.as_str()
                        .ok_or_else(|| failure("Invalid application state directory"))?,
                )
                .join(format!("qt-window-{scope}")),
                config.get("fileAction").filter(|v| !v.is_null()).cloned(),
                config["tray"] != true,
            )
        })();
        match result {
            Ok(Some(instance)) => {
                self.as_mut().rust_mut().instance = Some(instance);
                1
            }
            Ok(None) => 0,
            Err(error) => {
                self.as_mut().rust_mut().error = QString::from(error.message);
                self.error_changed();
                -1
            }
        }
    }
    fn next_file_action(&self) -> QString {
        QString::from(
            self.rust()
                .instance
                .as_ref()
                .and_then(Instance::next_request)
                .map(|v| v.to_string())
                .unwrap_or_default(),
        )
    }
    fn activation_pending(&self) -> bool {
        self.rust().instance.as_ref().is_some_and(Instance::pending)
    }
    fn shutdown(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().instance = None;
    }
}
impl Drop for Startup {
    fn drop(&mut self) {
        for translator in &mut self.translators {
            crate::platform::qt::remove_translation(translator.pin_mut());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graphical_options_preserve_explicit_backend_and_reject_service_flags() {
        assert_eq!(
            options(&["--demo".into(), "--cli".into(), "/sdk with spaces".into()]).unwrap(),
            (true, false, Some("/sdk with spaces".into()))
        );
        assert_eq!(
            options(&["--tray".into(), "--cli".into(), "/sdk".into()]).unwrap(),
            (false, true, Some("/sdk".into()))
        );
        assert!(options(&["--cli".into()]).is_err());
        assert!(options(&["--daemon".into()]).is_err());
    }
    #[test]
    fn source_checkout_catalogs_are_available() {
        assert!(catalog_at(Path::new("/nonexistent/native-app"))
            .unwrap()
            .join("en.json")
            .is_file());
    }
    #[test]
    fn installed_prefix_catalogs_take_precedence_over_source_checkout() {
        let root = std::env::temp_dir().join(format!("nd-launch-resources-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("bin")).unwrap();
        let catalogs = root.join("lib/neutrondrive/locales");
        std::fs::create_dir_all(&catalogs).unwrap();
        std::fs::write(catalogs.join("en.json"), "{}").unwrap();
        assert_eq!(catalog_at(&root.join("bin/neutrondrive")), Some(catalogs));
        std::fs::remove_dir_all(root).unwrap();
    }
}
