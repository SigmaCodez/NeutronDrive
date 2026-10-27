//! Small bindings to Qt APIs missing from cxx-qt-lib. No application logic in C++.
#[cxx::bridge]
pub mod qt {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("neutrondrive-qt/include/platform.h");
        type QGuiApplication = cxx_qt_lib::QGuiApplication;
        type QSettings;
        type QLockFile;
        type QTranslator;
    }
    #[namespace = "ndqt"]
    unsafe extern "C++" {
        #[rust_name = "system_tray_available"]
        fn systemTrayAvailable() -> bool;
        #[rust_name = "widgets_application"]
        fn widgetsApplication(arguments: &QStringList) -> UniquePtr<QGuiApplication>;
        #[rust_name = "new_translator"]
        fn newTranslator() -> UniquePtr<QTranslator>;
        #[rust_name = "load_translation"]
        fn loadTranslation(
            translator: Pin<&mut QTranslator>,
            language: &QString,
            domain: &QString,
        ) -> bool;
        #[rust_name = "install_translation"]
        fn installTranslation(translator: Pin<&mut QTranslator>) -> bool;
        #[rust_name = "remove_translation"]
        fn removeTranslation(translator: Pin<&mut QTranslator>);
        #[rust_name = "instance_lock"]
        fn instanceLock(path: &QString) -> UniquePtr<QLockFile>;
        #[rust_name = "claim_lock"]
        fn claimLock(lock: Pin<&mut QLockFile>) -> bool;
        #[rust_name = "open_url"]
        fn openUrl(url: &QString) -> bool;
        #[rust_name = "interface_settings"]
        fn interfaceSettings() -> UniquePtr<QSettings>;
        #[rust_name = "file_settings"]
        fn fileSettings(path: &QString) -> UniquePtr<QSettings>;
        fn setting(settings: &QSettings, key: &QString, fallback: &QString) -> QString;
        #[rust_name = "set_setting"]
        fn setSetting(settings: Pin<&mut QSettings>, key: &QString, value: &QString);
        #[rust_name = "sync_settings"]
        fn syncSettings(settings: Pin<&mut QSettings>) -> bool;
        #[rust_name = "system_languages"]
        fn systemLanguages() -> QStringList;
        #[rust_name = "native_language_name"]
        fn nativeLanguageName(locale: &QString) -> QString;
        #[rust_name = "number_text"]
        fn numberText(locale: &QString, value: f64, precision: i32) -> QString;
        #[rust_name = "date_text"]
        fn dateText(locale: &QString, value: &QString) -> QString;
        #[rust_name = "time_text"]
        fn timeText() -> QString;
        #[rust_name = "cache_location"]
        fn cacheLocation() -> QString;
        #[rust_name = "set_clipboard"]
        fn setClipboard(text: &QString);
        #[rust_name = "clipboard_text"]
        fn clipboardText() -> QString;
        #[rust_name = "snapshot_window"]
        fn snapshotWindow(name: &QString, path: &QString) -> bool;
        #[rust_name = "window_key"]
        fn windowKey(name: &QString, key: i32, control: bool, shift: bool) -> bool;
        #[rust_name = "window_click"]
        fn windowClick(name: &QString, x: f64, y: f64, double_click: bool) -> bool;
    }
}
