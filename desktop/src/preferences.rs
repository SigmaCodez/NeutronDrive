//! QSettings-compatible preferences. Failed writes never change the active UI mode.
use crate::{localization::Localization, platform::qt};
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use std::{path::Path, pin::Pin};
#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, language, READ, NOTIFY)]
        #[qproperty(
            QString,
            language_preference,
            READ,
            NOTIFY,
            cxx_name = "languagePreference"
        )]
        #[qproperty(
            QString,
            language_options,
            READ,
            NOTIFY,
            cxx_name = "languageOptionsJson"
        )]
        #[qproperty(QString, appearance, READ, NOTIFY)]
        #[qproperty(QString, error, READ, NOTIFY)]
        type NativePreferences = super::Preferences;
        #[qinvokable]
        fn initialize(self: Pin<&mut NativePreferences>, catalog_directory: &QString) -> bool;
        #[qinvokable]
        #[cxx_name = "setLanguagePreference"]
        fn set_language_preference(self: Pin<&mut NativePreferences>, value: &QString) -> bool;
        #[qinvokable]
        #[cxx_name = "setAppearancePreference"]
        fn set_appearance_preference(self: Pin<&mut NativePreferences>, value: &QString) -> bool;
        #[qinvokable]
        fn translate(self: &NativePreferences, source: &QString) -> QString;
    }
}
pub(crate) struct Store {
    settings: cxx::UniquePtr<qt::QSettings>,
}
impl Store {
    pub(crate) fn interface() -> Self {
        Self {
            settings: qt::interface_settings(),
        }
    }
    pub(crate) fn read(&self, key: &str, fallback: &str) -> String {
        qt::setting(
            &self.settings,
            &QString::from(key),
            &QString::from(fallback),
        )
        .to_string()
    }
    pub(crate) fn save(&mut self, key: &str, value: &str, fallback: &str) -> bool {
        let previous = self.read(key, fallback);
        qt::set_setting(
            self.settings.pin_mut(),
            &QString::from(key),
            &QString::from(value),
        );
        if qt::sync_settings(self.settings.pin_mut()) {
            return true;
        }
        // Keep the previous value in QSettings' cache, matching the existing UI.
        qt::set_setting(
            self.settings.pin_mut(),
            &QString::from(key),
            &QString::from(previous.as_str()),
        );
        false
    }
}
pub struct Preferences {
    language: QString,
    language_preference: QString,
    language_options: QString,
    appearance: QString,
    error: QString,
    store: Store,
    locale: Option<Localization>,
}
impl Default for Preferences {
    fn default() -> Self {
        let store = Store::interface();
        let mode = store.read("appearance", "system");
        Self {
            language: QString::from("en"),
            language_preference: QString::from("system"),
            language_options: QString::from("[]"),
            appearance: QString::from(if ["system", "light", "dark"].contains(&mode.as_str()) {
                mode.as_str()
            } else {
                "system"
            }),
            error: QString::default(),
            store,
            locale: None,
        }
    }
}
impl qobject::NativePreferences {
    fn initialize(mut self: Pin<&mut Self>, directory: &QString) -> bool {
        // Language is fixed for this window's lifetime. A saved change applies on restart.
        if self.rust().locale.is_some() {
            return true;
        }
        let preference = self.rust().store.read("language", "system");
        let locale = match Localization::load(Path::new(&directory.to_string()), &preference) {
            Ok(locale) => locale,
            Err(error) => {
                self.as_mut().rust_mut().error = QString::from(error.message);
                self.error_changed();
                return false;
            }
        };
        let preference = if locale.supported.contains(&preference) {
            preference.as_str()
        } else {
            "system"
        };
        self.as_mut().rust_mut().language_preference = QString::from(preference);
        self.as_mut().rust_mut().language = QString::from(locale.language.as_str());
        self.as_mut().rust_mut().language_options = QString::from(locale.options().to_string());
        self.as_mut().rust_mut().locale = Some(locale);
        self.as_mut().language_changed();
        self.as_mut().language_preference_changed();
        self.language_options_changed();
        true
    }
    fn save(mut self: Pin<&mut Self>, key: &str, value: &str) -> bool {
        if self.as_mut().rust_mut().store.save(key, value, "system") {
            return true;
        }
        let error = self.translate(&QString::from(
            "Einstellung konnte nicht gespeichert werden.",
        ));
        self.as_mut().rust_mut().error = error;
        self.error_changed();
        false
    }
    fn set_language_preference(mut self: Pin<&mut Self>, value: &QString) -> bool {
        let text = value.to_string();
        if !self
            .rust()
            .locale
            .as_ref()
            .is_some_and(|l| text == "system" || l.supported.contains(&text))
        {
            return false;
        }
        if !self.as_mut().save("language", &text) {
            return false;
        }
        if self.rust().language_preference != *value {
            self.as_mut().rust_mut().language_preference = value.clone();
            self.language_preference_changed();
        }
        true
    }
    fn set_appearance_preference(mut self: Pin<&mut Self>, value: &QString) -> bool {
        if !["system", "light", "dark"].contains(&value.to_string().as_str()) {
            return false;
        }
        if !self.as_mut().save("appearance", &value.to_string()) {
            return false;
        }
        if self.rust().appearance != *value {
            self.as_mut().rust_mut().appearance = value.clone();
            self.appearance_changed();
        }
        true
    }
    fn translate(&self, source: &QString) -> QString {
        self.rust()
            .locale
            .as_ref()
            .map(|l| QString::from(l.translate(&source.to_string())))
            .unwrap_or_else(|| source.clone())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_existing_ini_and_preserves_unrelated_preferences() {
        let dir = std::env::temp_dir().join(format!("nd-qt-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("interface.ini");
        std::fs::write(
            &file,
            "[General]\nlanguage=de\nappearance=dark\n\n[updates]\nautomatic=false\n",
        )
        .unwrap();
        let mut store = Store {
            settings: qt::file_settings(&QString::from(file.to_str().unwrap())),
        };
        assert_eq!(store.read("language", "system"), "de");
        assert_eq!(store.read("updates/automatic", "true"), "false");
        assert!(store.save("appearance", "light", "system"));
        drop(store);
        let store = Store {
            settings: qt::file_settings(&QString::from(file.to_str().unwrap())),
        };
        assert_eq!(store.read("appearance", "system"), "light");
        assert_eq!(store.read("language", "system"), "de");
        assert_eq!(store.read("updates/automatic", "true"), "false");
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn failed_write_restores_cached_preference() {
        let mut store = Store {
            settings: qt::file_settings(&QString::from("/proc/neutrondrive-qt-no-write.ini")),
        };
        assert!(!store.save("appearance", "dark", "system"));
        assert_eq!(store.read("appearance", "system"), "system");
    }
}
