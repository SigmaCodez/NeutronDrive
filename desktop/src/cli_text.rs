//! CLI localization uses the same offline catalogs without a GUI or preferences write.
use crate::{localization::Localization, startup};

pub struct CliText(Option<Localization>);
impl CliText {
    pub fn system() -> Self {
        Self(
            std::env::current_exe()
                .ok()
                .and_then(|exe| startup::catalog_at(&exe))
                .and_then(|directory| Localization::load(&directory, "system").ok()),
        )
    }
    fn tr(&self, source: &str, fallback: &str) -> String {
        self.0
            .as_ref()
            .map(|locale| locale.translate(source))
            .unwrap_or_else(|| fallback.into())
    }
    pub fn diagnostic(&self, text: &str) -> String {
        self.0
            .as_ref()
            .map(|locale| locale.diagnostic(text))
            .unwrap_or_else(|| text.into())
    }
    pub fn help(&self) -> String {
        let mut lines = vec![
            self.tr(
                "NeutronDrive — unabhängiger Proton Drive Client für Linux",
                "NeutronDrive — independent Proton Drive client for Linux",
            ),
            String::new(),
            "Usage: neutrondrive [OPTIONS]".into(),
            String::new(),
        ];
        for (flag, source, fallback) in [
            (
                "--demo",
                "Oberfläche mit klar markierten lokalen Beispieldaten öffnen",
                "Open the interface with clearly marked local sample data",
            ),
            (
                "--diagnose",
                "Backend-Verfügbarkeit ohne Anmeldung prüfen",
                "Check backend availability without signing in",
            ),
            (
                "--cli PATH",
                "Pfad zum kompatiblen NeutronDrive-SDK-Backend",
                "Path to the compatible NeutronDrive SDK backend",
            ),
            (
                "--daemon",
                "Gespeicherte Sync-Aufträge im Hintergrunddienst ausführen",
                "Run saved sync jobs in the background service",
            ),
            (
                "--background",
                "Sync und gespeicherte Online-Laufwerke bei Sitzungsstart aktivieren",
                "Start sync and saved online drives at session startup",
            ),
            (
                "--desktop-service",
                "Nativer D-Bus-Helfer für Online-Laufwerke",
                "Native D-Bus helper for online drives",
            ),
            (
                "--sync-status",
                "Status des Hintergrund-Sync als JSON",
                "Background sync status as JSON",
            ),
            (
                "--stop-background",
                "Hintergrund-Sync geordnet stoppen",
                "Stop background sync cleanly",
            ),
            (
                "--mount DIR",
                "Online-Laufwerk über den nativen Helfer einhängen",
                "Mount the online drive using the native helper",
            ),
            (
                "--mount-foreground DIR",
                "Online-Laufwerk im Vordergrund ausführen",
                "Run the online drive in the foreground",
            ),
            (
                "--unmount",
                "Online-Laufwerk geordnet aushängen",
                "Unmount the online drive cleanly",
            ),
            (
                "--pin PATH",
                "Online-Datei für Offline-Zugriff vorhalten",
                "Keep an online file available offline",
            ),
            (
                "--evict PATH",
                "Lokalen Cache einer sauberen, geschlossenen Datei freigeben",
                "Release the cache of a clean, closed file",
            ),
            (
                "--service-status",
                "Status des Online-Laufwerks als JSON",
                "Online drive status as JSON",
            ),
            (
                "--state-dir DIR",
                "Verzeichnis für Hintergrund-Aufträge (keine Änderung des Kontospeichers)",
                "Directory for background jobs (does not change account storage)",
            ),
            (
                "--remote PATH",
                "Cloud-Ordner des Online-Laufwerks",
                "Cloud folder for the online drive",
            ),
            (
                "--read-write",
                "Schreibzugriff im Online-Laufwerk aktivieren",
                "Enable write access to the online drive",
            ),
            (
                "--offline",
                "Online-Laufwerk nur aus vorhandenem Cache öffnen",
                "Open the online drive from the existing cache only",
            ),
        ] {
            lines.push(format!("  {flag:<25} {}", self.tr(source, fallback)));
        }
        lines.push("  --version                 Show version".into());
        lines.push("  --help, -h                Show this help".into());
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn every_catalog_localizes_help_without_changing_option_names() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/neutrondrive/locales");
        let supported = Localization::load(&directory, "en").unwrap().supported;
        for language in supported {
            let locale = Localization::load(&directory, &language).unwrap();
            let expected = locale.translate("Status des Hintergrund-Sync als JSON");
            let text = CliText(Some(locale)).help();
            assert!(text.contains(&expected), "{language}");
            for flag in [
                "--background",
                "--mount-foreground DIR",
                "--state-dir DIR",
                "--offline",
                "--cli PATH",
            ] {
                assert!(text.contains(flag), "{language}: {flag}");
            }
        }
    }
    #[test]
    fn missing_catalogs_have_english_help_and_preserve_diagnostic_details() {
        let text = CliText(None);
        assert!(text.help().contains("independent Proton Drive client"));
        assert!(text.help().contains("Stop background sync cleanly"));
        assert_eq!(
            text.diagnostic("specific SDK error: /test/path"),
            "specific SDK error: /test/path"
        );
    }
}
