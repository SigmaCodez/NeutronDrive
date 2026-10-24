use cxx_qt_build::{CxxQtBuilder, QmlModule};
fn main() {
    // Autostart and service managers invoke the installed executable directly.
    // Resolve the bundled Qt libraries without inheriting a shell launcher.
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../qt/lib");
    CxxQtBuilder::new_qml_module(QmlModule::new("NeutronDrive.Native").qml_files([
        "qml/AppearanceCheck.qml",
        "qml/ControllerCheck.qml",
        "qml/BackgroundCheck.qml",
        "qml/MountCheck.qml",
        "qml/UpdateCheck.qml",
        "qml/NativeShell.qml",
    ]))
    .qrc("ui.qrc")
    .file("src/startup.rs")
    .file("src/appearance.rs")
    .file("src/platform.rs")
    .file("src/preferences.rs")
    .file("src/file_controller.rs")
    .file("src/update_controller.rs")
    .qt_module("Gui")
    .qt_module("Quick")
    .qt_module("Widgets")
    .build();
}
