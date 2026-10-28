//! Catalog loading and formatting for Qt. Catalog validation remains in the core.
use crate::{
    platform::qt,
    runtime::{self, Failure},
};
use cxx_qt_lib::QString;
use serde_json::{json, Map, Value};
use std::path::Path;

pub struct Localization {
    pub language: String,
    pub supported: Vec<String>,
    english: Map<String, Value>,
    translated: Map<String, Value>,
}
fn native(operation: Value) -> Result<Value, Failure> {
    runtime::value(json!({"op":"localization", "operation":operation}))
}
impl Localization {
    pub fn load(directory: &Path, preference: &str) -> Result<Self, Failure> {
        let ui_languages: Vec<String> = qt::system_languages()
            .iter()
            .map(|v| v.to_string())
            .collect();
        let environment: Map<String, Value> = ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
            .iter()
            .filter_map(|key| std::env::var(key).ok().map(|v| (key.to_string(), v.into())))
            .collect();
        Self::load_with_environment(directory, preference, environment, ui_languages)
    }
    fn load_with_environment(
        directory: &Path,
        preference: &str,
        environment: Map<String, Value>,
        ui_languages: Vec<String>,
    ) -> Result<Self, Failure> {
        let supported: Vec<String> = serde_json::from_value(native(json!({"action":"supported"}))?)
            .map_err(|_| Failure {
                message: "Invalid locale list".into(),
                cancelled: false,
            })?;
        let language = if supported.iter().any(|v| v == preference) {
            preference.to_string()
        } else {
            native(
                json!({"action":"select", "environment":environment,"ui_languages":ui_languages}),
            )?
            .as_str()
            .unwrap_or("en")
            .to_string()
        };
        let catalog = |code: &str| -> Result<Map<String, Value>, Failure> {
            native(json!({"action":"catalog", "directory":directory, "code":code}))?
                .as_object()
                .cloned()
                .ok_or(Failure {
                    message: "Invalid language catalog".into(),
                    cancelled: false,
                })
        };
        Ok(Self {
            english: catalog("en")?,
            translated: catalog(&language)?,
            supported,
            language,
        })
    }
    pub fn translate(&self, source: &str) -> String {
        native(json!({"action":"translate", "source":source,
            "english":{source:self.english.get(source)}, "translated":{source:self.translated.get(source)}}))
            .ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| source.to_string())
    }
    pub fn diagnostic(&self, text: &str) -> String {
        native(json!({"action":"diagnostic","text":text,"english":self.english,"translated":self.translated}))
            .ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| text.to_string())
    }
    pub fn options(&self) -> Value {
        let mut values = vec![json!({"code":"system","name":self.translate("Systemeinstellung")})];
        values.extend(self.supported.iter().map(|code| {
            json!({"code":code,
            "name":format!("{} ({code})",qt::native_language_name(&QString::from(code.as_str())))})
        }));
        json!(values)
    }
    pub fn size_text(&self, size: Option<u64>) -> String {
        let Some(size) = size else {
            return "—".into();
        };
        let mut value = size as f64;
        for unit in ["B", "KB", "MB", "GB", "TB"] {
            if value < 1024.0 || unit == "TB" {
                return format!(
                    "{} {unit}",
                    qt::number_text(
                        &QString::from(self.language.as_str()),
                        value,
                        if unit == "B" { 0 } else { 1 }
                    )
                );
            }
            value /= 1024.0;
        }
        unreachable!()
    }
    pub fn date_text(&self, value: &str) -> String {
        let result = qt::date_text(
            &QString::from(self.language.as_str()),
            &QString::from(value),
        )
        .to_string();
        if result.is_empty() {
            "—".into()
        } else {
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn load(code: &str, env: Value, ui: Vec<String>) -> Localization {
        Localization::load_with_environment(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/neutrondrive/locales"),
            code,
            env.as_object().unwrap().clone(),
            ui,
        )
        .unwrap()
    }
    #[test]
    fn locale_fallback_manual_override_and_catalogs() {
        let english = load("system", json!({"LANG":"ja_JP.UTF-8"}), vec![]);
        assert_eq!(english.language, "en");
        assert_eq!(english.translate("Meine Dateien"), "My files");
        assert_eq!(english.size_text(Some(1536)), "1.5 KB");
        assert_eq!(english.size_text(None), "—");
        assert_eq!(english.date_text("not a date"), "—");
        assert_ne!(english.date_text("2026-10-20T12:00:00Z"), "—");
        assert_eq!(
            load("de", json!({"LANG":"fr_FR.UTF-8"}), vec![]).language,
            "de"
        );
        assert_eq!(
            load("system", json!({}), vec!["pt-BR".into()]).language,
            "pt-BR"
        );
        assert_eq!(english.options().as_array().unwrap().len(), 26);
        for code in &english.supported {
            let locale = load(code, json!({}), vec![]);
            assert!(!locale.translate("Meine Dateien").is_empty());
            assert!(locale
                .translated
                .get("Abgebrochen")
                .is_some_and(|v| v.as_str().is_some_and(|s| !s.is_empty())));
            if code == "en" {
                assert_eq!(locale.translate("Abgebrochen"), "Cancelled");
            }
            assert_eq!(
                locale.diagnostic("unchanged arbitrary filename.txt"),
                "unchanged arbitrary filename.txt"
            );
        }
    }
}
