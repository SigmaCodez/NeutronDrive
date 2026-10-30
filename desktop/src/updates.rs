//! Update state and owned download lifetime. UI policy remains independent of Qt workers.
use crate::{
    runtime::{self, Failure},
    update_restart::Plan,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub fn failure(message: &str) -> Failure {
    Failure {
        message: message.into(),
        cancelled: false,
    }
}
pub fn call(operation: Value, callback: &mut impl FnMut(Value) -> i32) -> Result<Value, Failure> {
    unwrap(runtime::call(
        json!({"op":"update_runtime","operation":operation}),
        callback,
    )?)
}
fn unwrap(result: Value) -> Result<Value, Failure> {
    if result.get("error_kind").is_some() {
        return Err(Failure {
            message: result["message"].as_str().unwrap_or("Update failed").into(),
            cancelled: result["error_kind"] == "cancelled",
        });
    }
    result
        .get("value")
        .cloned()
        .ok_or_else(|| failure("Invalid update response"))
}
/// A successful download transfers cleanup ownership, including queued results discarded on close.
#[derive(Debug)]
pub struct Download {
    pub path: PathBuf,
}
impl Drop for Download {
    fn drop(&mut self) {
        // Never recurse: remove only the verified package returned by our downloader.
        let _ = std::fs::remove_file(&self.path);
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Task {
    Check { preview: bool, format: String },
    Download(Value),
    Install { release: Value, path: PathBuf },
    Restart(Plan),
}
#[derive(Debug)]
pub enum Outcome {
    Checked(Value),
    Downloaded(Download),
    Installed(Plan),
    Restarted(Plan),
}
pub struct Model {
    pub automatic: bool,
    pub auto_download: bool,
    pub previews: bool,
    pub last_attempt: f64,
    pub checked_this_session: bool,
    pub notified_tag: String,
    pub format: String,
    pub release: Value,
    pub download: Option<Download>,
    pub active: Option<Task>,
    pub restart: Option<Plan>,
    pub status: String,
    pub demo: bool,
    pub closed: bool,
}
pub fn preview_default(version: &str) -> bool {
    let reply = neutrondrive_core::policy_request(
        json!({"version":1,"op":"update_version","value":version})
            .to_string()
            .as_bytes(),
    );
    reply["ok"] == true && reply["result"][3].as_u64().is_some_and(|stage| stage < 3)
}
impl Model {
    pub fn new(format: String, demo: bool) -> Self {
        Self {
            automatic: true,
            auto_download: false,
            previews: preview_default(env!("CARGO_PKG_VERSION")),
            last_attempt: 0.0,
            checked_this_session: false,
            notified_tag: String::new(),
            format,
            release: Value::Null,
            download: None,
            active: None,
            restart: None,
            status: "Noch nicht nach Updates gesucht.".into(),
            demo,
            closed: false,
        }
    }
    pub fn downloadable(&self) -> bool {
        self.release["asset_url"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
    }
    pub fn due(&self, now: f64) -> bool {
        let elapsed = now - self.last_attempt;
        !self.closed
            && !self.demo
            && self.automatic
            && self.active.is_none()
            && self.restart.is_none()
            && self.download.is_none()
            && (!self.checked_this_session || !(0.0..7200.0).contains(&elapsed))
    }
    pub fn should_announce(&mut self, result: &Result<Outcome, Failure>) -> bool {
        let Ok(Outcome::Checked(release)) = result else {
            return false;
        };
        let Some(tag) = release["tag"].as_str().filter(|tag| !tag.is_empty()) else {
            return false;
        };
        if self.closed || self.notified_tag == tag {
            return false;
        }
        self.notified_tag = tag.to_owned();
        true
    }
    pub fn option(&mut self, name: &str, value: bool) {
        match name {
            "automatic" => self.automatic = value,
            "download" => self.auto_download = value,
            "previews" => {
                self.previews = value;
                self.download = None;
                self.release = Value::Null;
                self.status = "Noch nicht nach Updates gesucht.".into();
            }
            _ => (),
        }
    }
    pub fn begin(&mut self, action: &str, owner_busy: bool, now: f64) -> Option<Task> {
        if self.closed || self.demo || self.active.is_some() {
            return None;
        }
        if self.restart.is_some() && action != "restart" {
            return None;
        }
        let (task, status) = match action {
            "restart" if self.restart.is_some() && !owner_busy => (
                Task::Restart(self.restart.as_ref().unwrap().clone()),
                "Update installiert. App und Dienste werden neu gestartet …",
            ),
            "check" => {
                self.download = None;
                self.release = Value::Null;
                self.last_attempt = now;
                self.checked_this_session = true;
                (
                    Task::Check {
                        preview: self.previews,
                        format: self.format.clone(),
                    },
                    "Updates werden gesucht …",
                )
            }
            "download" if self.downloadable() && self.download.is_none() => (
                Task::Download(self.release.clone()),
                "Update wird heruntergeladen und geprüft …",
            ),
            "install"
                if self.download.is_some()
                    && matches!(self.format.as_str(), "deb" | "rpm-fedora" | "rpm-opensuse") =>
            {
                if owner_busy {
                    self.status =
                        "Bitte den laufenden Vorgang abschließen und erneut versuchen.".into();
                    return None;
                }
                (
                    Task::Install {
                        release: self.release.clone(),
                        path: self.download.as_ref().unwrap().path.clone(),
                    },
                    "Update wird installiert. Bitte die Systemabfrage bestätigen.",
                )
            }
            _ => return None,
        };
        self.status = status.into();
        self.active = Some(task.clone());
        Some(task)
    }
    pub fn finish(&mut self, result: Result<Outcome, Failure>) -> bool {
        let active = self.active.take();
        if self.closed {
            return false;
        }
        match (active,result) {
            (Some(Task::Check{..}),Ok(Outcome::Checked(release))) => {
                self.status=if release.is_null(){"Keine neuere Version verfügbar.".into()}else{"Update verfügbar: {p0}".into()};
                self.release=release;
                return self.auto_download && self.downloadable();
            },
            (Some(Task::Download(_)),Ok(Outcome::Downloaded(download))) => {self.download=Some(download);self.status="Update geprüft und zur Installation bereit.".into();},
            (Some(Task::Install{..}),Ok(Outcome::Installed(plan))) => {self.download=None;self.release=Value::Null;self.restart=Some(plan);self.status="Update installiert. App und Dienste werden neu gestartet …".into();},
            (Some(Task::Restart(_)),Ok(Outcome::Restarted(_))) => {self.restart=None;self.status="Update installiert. App und Dienste werden neu gestartet …".into();},
            (_,Err(error)) => self.status=error.message,
            _ => self.status="Update fehlgeschlagen. Bitte Verbindung und Berechtigungen prüfen und erneut versuchen.".into(),
        }
        false
    }
    pub fn close(&mut self) {
        self.closed = true;
        self.download = None;
    }
}
pub fn execute(
    task: &Task,
    cache: &Path,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Outcome, Failure> {
    match task {
        Task::Restart(plan) => {
            crate::update_restart::restore_with_binary(plan)?;
            Ok(Outcome::Restarted(plan.clone()))
        }
        Task::Check{preview,format} => call(json!({"action":"check","current":env!("CARGO_PKG_VERSION"),"preview":preview,"format":format}),callback).map(Outcome::Checked),
        Task::Download(release) => {
            let path=call(json!({"action":"download","release":release,"destination":cache}),callback)?;
            let path=PathBuf::from(path.as_str().ok_or_else(|| failure("Invalid download path"))?);
            Ok(Outcome::Downloaded(Download{path}))
        },
        Task::Install{release,path} => {
            call(json!({"action":"verify","release":release,"path":path}),callback)?;
            crate::update_restart::install(path, callback).map(Outcome::Installed)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn release() -> Value {
        json!({"tag":"v9.0.0","asset_url":"https://example.test/package"})
    }
    #[test]
    fn rpm_updates_enter_the_shared_install_and_restart_flow() {
        for format in ["rpm-fedora", "rpm-opensuse"] {
            let mut m = Model::new(format.into(), false);
            m.release = release();
            m.download = Some(Download {
                path: "/nonexistent/neutrondrive.rpm".into(),
            });
            assert!(matches!(
                m.begin("install", false, 0.0),
                Some(Task::Install { .. })
            ));
        }
    }
    #[test]
    fn preview_default_tracks_release_channel() {
        assert!(preview_default("0.6.0-alpha.6"));
        assert!(preview_default("0.6.0a6"));
        assert!(!preview_default("1.0.0"));
    }
    #[test]
    fn cadence_clock_change_and_demo() {
        let mut m = Model::new("deb".into(), false);
        m.last_attempt = 100000.0;
        assert!(m.due(100001.0)); // A previous process cannot suppress startup checks.
        m.checked_this_session = true;
        assert!(!m.due(100001.0));
        assert!(!m.due(107199.0));
        assert!(m.due(107200.0));
        assert!(m.due(99999.0));
        m.automatic = false;
        assert!(!m.due(200000.0));
        m.automatic = true;
        m.demo = true;
        assert!(!m.due(200000.0));
        assert!(m.begin("check", false, 0.0).is_none());
    }
    #[test]
    fn notices_only_for_new_release_identity() {
        let mut m = Model::new("deb".into(), false);
        assert!(!m.should_announce(&Err(failure("offline"))));
        assert!(!m.should_announce(&Ok(Outcome::Checked(Value::Null))));
        assert!(m.should_announce(&Ok(Outcome::Checked(release()))));
        assert!(!m.should_announce(&Ok(Outcome::Checked(release()))));
        assert!(m.should_announce(&Ok(Outcome::Checked(json!({"tag":"v9.0.1"})))));
        m.closed = true;
        assert!(!m.should_announce(&Ok(Outcome::Checked(json!({"tag":"v10.0.0"})))));
    }
    #[test]
    fn automatic_download_never_automatically_installs() {
        let mut m = Model::new("deb".into(), false);
        m.auto_download = true;
        assert!(m.begin("check", false, 100000.0).is_some());
        assert!(m.begin("check", false, 100000.0).is_none());
        assert!(m.finish(Ok(Outcome::Checked(release()))));
        assert!(matches!(
            m.begin("download", false, 0.0),
            Some(Task::Download(_))
        ));
        assert!(!m.finish(Err(failure("network"))));
        assert!(m.download.is_none());
        m.option("previews", false);
        assert!(m.release.is_null());
    }
    #[test]
    fn invalid_nested_results_do_not_become_success() {
        assert!(unwrap(json!({"ok":true})).is_err());
        assert!(
            unwrap(json!({"error_kind":"cancelled","message":"cancelled"}))
                .unwrap_err()
                .cancelled
        );
        assert_eq!(unwrap(json!({"value":null})).unwrap(), Value::Null);
    }
    #[test]
    fn installation_is_explicit_and_busy_owner_keeps_download() {
        let dir =
            std::env::temp_dir().join(format!("nd-update-install-model-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("package.deb");
        std::fs::write(&path, b"fixture").unwrap();
        let mut m = Model::new("deb".into(), false);
        m.release = release();
        m.begin("download", false, 0.0);
        assert!(!m.finish(Ok(Outcome::Downloaded(Download { path: path.clone() }))));
        assert!(m.active.is_none());
        assert!(m.begin("install", true, 0.0).is_none());
        assert!(path.exists());
        assert!(matches!(
            m.begin("install", false, 0.0),
            Some(Task::Install { .. })
        ));
        m.finish(Err(failure("authentication denied")));
        assert!(path.exists());
        assert!(m.download.is_some());
        m.begin("install", false, 0.0);
        m.finish(Ok(Outcome::Installed(Plan {
            app: "/new/app".into(),
            cli: String::new(),
            sync: true,
            drive: true,
        })));
        assert!(!path.exists());
        assert!(m.release.is_null());
        assert!(!dir.exists());
    }
    #[test]
    fn installed_update_retries_service_restart_without_reinstalling() {
        let mut m = Model::new("deb".into(), false);
        m.active = Some(Task::Install {
            release: release(),
            path: "/not-owned".into(),
        });
        let plan = Plan {
            app: "/new/app".into(),
            cli: String::new(),
            sync: true,
            drive: true,
        };
        m.finish(Ok(Outcome::Installed(plan.clone())));
        assert!(!m.due(999999.0));
        assert!(m.begin("check", false, 0.0).is_none());
        assert!(m.begin("install", false, 0.0).is_none());
        assert!(matches!(
            m.begin("restart", false, 0.0),
            Some(Task::Restart(_))
        ));
        m.finish(Err(failure("restore failed")));
        assert_eq!(m.status, "restore failed");
        assert_eq!(m.restart, Some(plan.clone()));
        m.begin("restart", false, 0.0);
        m.finish(Ok(Outcome::Restarted(plan)));
        assert!(m.restart.is_none());
    }
    #[test]
    fn closing_or_discarding_a_late_result_cleans_only_its_owned_download() {
        let dir = std::env::temp_dir().join(format!("nd-update-ownership-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("package.deb");
        std::fs::write(&path, b"synthetic").unwrap();
        let mut m = Model::new("deb".into(), false);
        m.release = release();
        m.begin("download", false, 0.0);
        m.close();
        assert!(!m.finish(Ok(Outcome::Downloaded(Download { path: path.clone() }))));
        assert!(!path.exists());
        assert!(!dir.exists());
    }
}
