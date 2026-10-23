mod account;
mod appearance;
mod appimage;
mod autostart;
mod background;
mod casefold;
mod cli_text;
mod file_controller;
pub mod files;
mod headless;
mod instance;
mod local_trash;
pub mod localization;
pub mod platform;
mod preferences;
pub mod runtime;
mod startup;
mod update_controller;
mod update_installer;
mod updates;
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};
fn main() {
    appimage::run();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = headless::run(&args) {
        std::process::exit(code);
    }
    let component = match args.as_slice() {
        [mode, _catalog] if mode == "--check-window" => "NativeShell.qml",
        [mode, _catalog, _images] if mode == "--check-window" => "NativeShell.qml",
        [mode, _catalog, _sdk] if mode == "--check-window-fixture" => "NativeShell.qml",
        [mode, _catalog, _images, _language, _title] if mode == "--check-window" => {
            "NativeShell.qml"
        }
        [mode, _catalog] if mode == "--check-updates" => "UpdateCheck.qml",
        [mode] if mode == "--check-appearance" => "AppearanceCheck.qml",
        [mode, _catalog, _fixture] if mode == "--check-controller" => "ControllerCheck.qml",
        [mode, _catalog, _local] if mode == "--check-background" => "BackgroundCheck.qml",
        [mode, _catalog, _sdk, _point] if mode == "--check-mount" => "MountCheck.qml",
        _ => {
            if let Err(error) = startup::configure(&args, false) {
                eprintln!("{}", error.message);
                std::process::exit(2);
            }
            "NativeShell.qml"
        }
    };
    if args
        .first()
        .is_some_and(|a| a == "--check-window" || a == "--check-window-fixture")
    {
        if let Err(error) = startup::configure(&args, true) {
            eprintln!("{}", error.message);
            std::process::exit(2);
        }
    }
    cxx_qt::init_qml_module!("NeutronDrive.Native");
    let display = ["DISPLAY", "WAYLAND_DISPLAY"]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()));
    let offscreen = std::env::var("QT_QPA_PLATFORM")
        .is_ok_and(|v| matches!(v.as_str(), "offscreen" | "minimal"));
    if !display && !offscreen {
        eprintln!("{}", cli_text::CliText::system().diagnostic("No graphical session is available. Start NeutronDrive in a Wayland or X11 session."));
        std::process::exit(1);
    }
    let mut application = QGuiApplication::new();
    QGuiApplication::set_desktop_file_name(&QString::from("io.github.SigmaCodez.NeutronDrive"));
    let mut engine = QQmlApplicationEngine::new();
    let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reported = failed.clone();
    engine
        .pin_mut()
        .on_object_creation_failed(move |_, _| {
            reported.store(true, std::sync::atomic::Ordering::Release);
        })
        .release();
    let mut app = application.pin_mut();
    app.as_mut()
        .set_application_name(&QString::from("NeutronDrive"));
    engine.pin_mut().load(&QUrl::from(
        format!("qrc:/qt/qml/NeutronDrive/Native/qml/{component}").as_str(),
    ));
    if failed.load(std::sync::atomic::Ordering::Acquire) {
        drop(engine);
        drop(application);
        std::process::exit(1);
    }
    let result = app.exec();
    drop(engine);
    drop(application);
    std::process::exit(result);
}
