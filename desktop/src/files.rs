//! File-browser state machine. All dialog destinations are captured when opened;
//! an answer never silently applies a mutation to a newer selection or folder.
use crate::{localization::Localization, platform::qt, runtime::Failure};
use cxx_qt_lib::QUrl;
use serde_json::{json, Value};
use std::collections::{BTreeSet, VecDeque};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum Done {
    TakeoutScan,
    TakeoutImport,
    FileManager,
    Setup,
    Loaded,
    Refresh,
    None,
    Share(Value),
    ShareReload(Value),
    Versions(String),
    VersionRestored,
    Photos,
    PhotoLibrary(Value),
    Version,
    LoggedOut,
    Background,
    Mount,
    TrashList,
    TrashRestored,
    Autostart(bool),
    CloseStatus,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    AccountStorage,
    FileManager,
    Drive,
    Background,
    Mount,
    Trash,
    Autostart,
}
#[derive(Clone, Debug)]
pub struct Work {
    pub id: u64,
    pub target: Target,
    pub title: String,
    pub operation: Value,
    pub done: Done,
}
#[derive(Clone, Debug)]
enum Answer {
    LinkSettings {
        node: Value,
        value: Value,
        settings: Value,
    },
    TakeoutFolder,
    TakeoutImport(Value),
    Invite {
        node: Value,
        value: Value,
        email: String,
        role: String,
    },
    PhotoLibrary(Value),
    DownloadPhoto {
        uid: String,
    },
    RestoreVersion {
        path: String,
        revision: String,
        expected: String,
    },
    Close,
    Upload {
        parent: String,
    },
    Download {
        paths: Vec<String>,
    },
    Mkdir {
        parent: String,
    },
    Rename {
        path: String,
    },
    Move {
        paths: Vec<String>,
    },
    Trash {
        paths: Vec<String>,
    },
    Share {
        node: Value,
        value: Value,
    },
    Revoke {
        node: Value,
        value: Value,
    },
    Logout,
    BackgroundFolder,
    ComputerFolder,
    BackgroundRemoval(Value),
    MountFolder,
    ActivateBackground {
        local: String,
        remote: String,
        computer: bool,
    },
}
#[derive(Clone, Debug)]
pub enum Effect {
    Work(Work),
    Dialog(Value),
    Clipboard(String),
    OpenUrl(String),
    PollService(Target),
    Cancel,
    Close,
    Hide,
}
// Session-only metadata: bounded, never written to disk or reused after logout.
struct CachedFolder {
    path: String,
    nodes: Vec<Value>,
    saved: Instant,
    bytes: usize,
}
pub struct Files {
    pub locale: Localization,
    pub state: Value,
    pub entries: Value,
    pub files_revision: u64,
    pub photos_revision: u64,
    nodes: Vec<Value>,
    visible: Vec<Value>,
    photo_index: Vec<String>,
    photo_checked: Option<Instant>,
    selection: BTreeSet<usize>,
    selection_anchor: Option<usize>,
    crumbs: Vec<(String, String)>,
    pending: Option<(u64, Answer)>,
    serial: u64,
    active: Option<Work>,
    folder_cache: VecDeque<CachedFolder>,
    next_work: u64,
    login_url: Option<String>,
    background_local: Option<String>,
    mount_local: Option<String>,
    snap: bool,
    flatpak: bool,
    closing: bool,
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn folded(value: &str) -> String {
    crate::casefold::fold(value)
}
impl Files {
    pub fn new(locale: Localization, executable: &str, demo: bool) -> Self {
        let mut fields = json!({});
        for (key, value) in [
            ("search", ""),
            ("background_local_label", "Kein lokaler Ordner gewählt"),
            ("background_remote", "/my-files"),
            ("background_add_button", ""),
            (
                "background_status",
                "Noch keine Hintergrund-Aufträge geladen.",
            ),
            ("files_title", "Meine Dateien"),
            ("empty_label", "Noch keine Dateien geladen."),
            ("count_label", ""),
            ("notice_label", ""),
            ("login_button", ""),
            ("status_label", "Bereit"),
            ("spinner", ""),
            ("cancel_button", ""),
            ("file_list", ""),
            ("upload_button", ""),
            ("folder_upload_button", ""),
            ("mkdir_button", ""),
            ("refresh_button", ""),
        ] {
            fields[key] = json!({"value":locale.translate(value),"enabled":true,"visible":true});
        }
        for (key, value) in [
            ("mount_label", "Kein Einhängeordner gewählt"),
            ("mount_remote", "/my-files"),
            ("mount_button", ""),
            ("mount_status", "Noch kein Online-Laufwerk verbunden."),
            ("cache_path", ""),
            ("local_trash_status", "Noch keine Einträge geladen."),
            (
                "local_trash_empty",
                "Keine registrierten Papierkorbeinträge.",
            ),
        ] {
            fields[key] = json!({"value":locale.translate(value),"enabled":true,"visible":true});
        }
        for key in ["notice_label", "login_button", "spinner", "cancel_button"] {
            fields[key]["visible"] = false.into();
        }
        fields["backend_label"] = json!({"value":if demo {locale.translate("Demo — keine Cloud-Verbindung")} else {executable.into()},"enabled":true,"visible":true});
        for key in ["mount_writable", "mount_offline"] {
            fields[key] = json!({"value":false,"enabled":true,"visible":true});
        }
        let title = locale.translate("Meine Dateien");
        let mut this = Self {
            locale,
            state: json!({"fields":fields,"actions":{},"focusIndex":0,
            "page":"files","path":"/my-files","busy":false,"demo":demo,"version":env!("CARGO_PKG_VERSION"),
            "noticeError":false,"crumbs":[title],"selection":[],"jobs":[],"trash":[],"activity":[]}),
            entries: json!([]),
            files_revision: 0,
            nodes: vec![],
            visible: vec![],
            photo_index: vec![],
            photo_checked: None,
            photos_revision: 1,
            selection: BTreeSet::new(),
            selection_anchor: None,
            crumbs: vec![(title, "/my-files".into())],
            pending: None,
            serial: 0,
            active: None,
            folder_cache: VecDeque::new(),
            next_work: 0,
            login_url: None,
            background_local: None,
            mount_local: None,
            flatpak: std::env::var_os("FLATPAK_ID").is_some_and(|v| !v.is_empty()),
            closing: false,
            snap: std::env::var_os("SNAP").is_some_and(|v| !v.is_empty()),
        };
        this.state["fields"]["background_add_button"]["enabled"] = (!demo).into();
        this.state["fields"]["mount_button"]["enabled"] =
            (!demo && !this.snap && !this.flatpak).into();
        this.state["photos"] = json!([]);
        this.state["photoOffset"] = json!(0);
        this.state["photosMore"] = json!(false);
        this.state["authenticated"] = json!(false);
        this.state["account"] = Value::Null;
        this.state["storage"] = Value::Null;
        this.state["sandboxed"] = json!(this.snap || this.flatpak);
        this.state["setupNeeded"] = json!(false);
        this.state["setupFolder"] = json!("");
        this.state["photoTotal"] = Value::Null;
        this.state["photoAlbum"] = json!("");
        this.state["photoAlbums"] = json!([]);
        this.state["photoSelection"] = json!([]);
        this.state["history"] = json!({"visible":false,"revisions":[]});
        this.update_actions();
        this
    }
    fn tr(&self, source: &str) -> String {
        self.locale.translate(source)
    }
    fn path(&self) -> &str {
        text(&self.state, "path")
    }
    pub fn busy(&self) -> bool {
        self.active.is_some()
    }
    pub fn message(&mut self, message: &str, error: bool) {
        self.state["noticeError"] = error.into();
        self.field("notice_label", json!(self.locale.diagnostic(message)));
        self.state["fields"]["notice_label"]["visible"] = (!message.is_empty()).into();
    }
    fn field(&mut self, key: &str, value: Value) {
        self.state["fields"][key]["value"] = value;
    }
    fn selected(&self) -> Vec<Value> {
        self.selection
            .iter()
            .filter_map(|i| self.visible.get(*i).cloned())
            .collect()
    }
    fn paths(&self) -> Vec<String> {
        self.selected()
            .iter()
            .map(|n| text(n, "path").to_string())
            .collect()
    }
    fn update_actions(&mut self) {
        let selected = self.selected();
        let safe =
            !selected.is_empty() && selected.iter().all(|n| n["safe"] == true) && !self.busy();
        let trash = self.path() == "/trash";
        for name in [
            "Download",
            "Umbenennen",
            "Verschieben",
            "Freigabe",
            "Papierkorb",
            "Wiederherstellen",
        ] {
            let allowed = safe
                && (!["Umbenennen", "Verschieben", "Freigabe"].contains(&name)
                    || selected.len() == 1)
                && !(trash && ["Download", "Freigabe"].contains(&name));
            self.state["actions"][name] = json!({"value":"","enabled":allowed,"visible":(name == "Wiederherstellen") == trash});
        }
        for name in ["Keep on device", "Free up space"] {
            self.state["actions"][name] = json!({"enabled":safe && !trash && selected.len()==1 && !self.snap && !self.flatpak,"visible":!trash && !self.snap && !self.flatpak});
        }
        self.state["actions"]["Versions"] = json!({"enabled":safe && !trash && selected.len()==1 && selected[0]["type"]=="file", "visible":!trash});
        let writable = self.path().starts_with("/my-files") || self.path().matches('/').count() > 1;
        for key in ["upload_button", "folder_upload_button", "mkdir_button"] {
            self.state["fields"][key]["enabled"] = (writable && !trash && !self.busy()).into();
        }
        self.state["fields"]["refresh_button"]["enabled"] = (!self.busy()).into();
        self.state["selection"] = json!(self.selection);
    }
    fn render(&mut self) {
        let query = folded(text(&self.state["fields"]["search"], "value"));
        let visible: Vec<Value> = self
            .nodes
            .iter()
            .filter(|n| query.is_empty() || folded(text(n, "name")).contains(&query))
            .cloned()
            .collect();
        // Search terms and hidden entries may change without changing any
        // visible row. Preserve delegates and index selection in that case.
        if self.files_revision == 0 || self.visible != visible {
            self.visible = visible;
            self.selection.clear();
            self.selection_anchor = None;
            self.state["focusIndex"] = 0.into();
            self.entries = json!(self.visible.iter().map(|n| {
            let folder = n["type"] == "folder";
            let name = text(n,"name");
            json!({"name":if n["safe"] != true && ["[Name nicht entschlüsselbar]","[Ungültiger Name]"].contains(&name) {self.locale.diagnostic(name)} else {name.into()},
                "kind":self.tr(if folder {"Ordner"} else {"Datei"}),"isFolder":folder,"safe":n["safe"] == true,
                "size":if folder {"—".into()} else {self.locale.size_text(n["size"].as_u64())},
                "modified":self.locale.date_text(text(n,"modified"))})
        }).collect::<Vec<_>>());
            self.files_revision += 1;
        }
        self.field(
            "empty_label",
            json!(self.tr(if query.is_empty() {
                "Dieser Ordner ist leer."
            } else {
                "Keine passenden Dateien in diesem Ordner."
            })),
        );
        self.state["fields"]["empty_label"]["visible"] = self.visible.is_empty().into();
        self.field(
            "count_label",
            json!(self
                .tr("{p0} von {p1} Einträgen · Doppelklick öffnet Ordner")
                .replace("{p0}", &self.visible.len().to_string())
                .replace("{p1}", &self.nodes.len().to_string())),
        );
        self.update_actions();
    }
    fn select(&mut self, index: usize, extend: bool) {
        if (self.busy() && !self.active_read()) || index >= self.visible.len() {
            return;
        }
        self.state["focusIndex"] = index.into();
        self.selection_anchor = Some(index);
        if !extend {
            self.selection.clear();
            self.selection.insert(index);
        } else if !self.selection.remove(&index) {
            self.selection.insert(index);
        }
        self.update_actions();
    }
    fn select_range(&mut self, index: usize) {
        if (self.busy() && !self.active_read()) || index >= self.visible.len() {
            return;
        }
        let anchor = self
            .selection_anchor
            .unwrap_or_else(|| self.state["focusIndex"].as_u64().unwrap_or(0) as usize)
            .min(self.visible.len() - 1);
        self.selection_anchor = Some(anchor);
        // Rebuild from the fixed anchor so reversing direction shrinks the
        // range instead of toggling holes into it.
        self.selection = (anchor.min(index)..=anchor.max(index)).collect();
        self.state["focusIndex"] = index.into();
        self.update_actions();
    }
    fn submit(&mut self, title: &str, operation: Value, done: Done) -> Vec<Effect> {
        if self.busy() {
            self.message(
                &self.tr("Es läuft bereits ein Vorgang. Bitte warten oder abbrechen."),
                false,
            );
            return vec![];
        }
        // A retry starts a new attempt. Keep informational transfer summaries,
        // but do not leave an old error banner over a successful new result.
        // The failed attempt remains recorded in activity history.
        if self.state["noticeError"] == true {
            self.message("", false);
        }
        // Mutations (including login/logout) invalidate session metadata before
        // starting: partial success or failure must not revive stale entries.
        if !matches!(
            done,
            Done::Loaded | Done::Photos | Done::TrashList | Done::Version | Done::CloseStatus
        ) && !matches!(&done, Done::PhotoLibrary(request) if matches!(text(request, "action"), "index" | "details" | "albums"))
        {
            self.folder_cache.clear();
        }
        self.state["progress"] = Value::Null;
        self.next_work += 1;
        let title = self.tr(title);
        let work = Work {
            id: self.next_work,
            target: Target::Drive,
            title: title.clone(),
            operation,
            done,
        };
        self.active = Some(work.clone());
        self.state["busy"] = true.into();
        self.state["reading"] = json!(self.active_read());
        for key in ["spinner", "cancel_button"] {
            self.state["fields"][key]["visible"] = true.into();
        }
        self.state["fields"]["file_list"]["enabled"] = false.into();
        self.field("status_label", json!(format!("{title} …")));
        self.update_actions();
        let status = self.tr("Läuft …");
        let time = qt::time_text().to_string();
        let records = self.state["activity"].as_array_mut().unwrap();
        records.insert(0, json!({"time":time,"title":title,"status":status}));
        records.truncate(100);
        vec![Effect::Work(work)]
    }
    fn submit_background(&mut self, title: &str, operation: Value) -> Vec<Effect> {
        if self.state["demo"] == true {
            self.message(
                &self.tr("Der Demo-Modus startet keine Hintergrunddienste."),
                false,
            );
            return vec![];
        }
        let mut effects = self.submit(title, operation, Done::Background);
        if let Some(Effect::Work(work)) = effects.first_mut() {
            work.target = Target::Background;
            self.active = Some(work.clone());
        }
        effects
    }
    pub fn service_generation(&self) -> u64 {
        self.next_work
    }
    pub fn background_polled(&mut self, generation: u64, value: &Value) {
        if self.next_work == generation {
            self.render_background(value);
        }
    }
    fn render_background(&mut self, value: &Value) {
        self.state["deletions"] = value["mount"]["deletions"].clone();
        self.state["downloads"] = value["mount"]["downloads"].clone();
        self.state["mountOperation"] = value["mount"]["operation"].clone();
        self.state["backgroundRunning"] = json!(value["running"] == true);
        let error = value["error"]
            .as_str()
            .filter(|v| !v.is_empty())
            .or_else(|| value["event_source"]["error"].as_str())
            .unwrap_or("");
        let mut status = self.tr(if value["running"] == true {
            "Dienst läuft"
        } else {
            "Dienst ist gestoppt"
        });
        if !error.is_empty() {
            status.push_str(" · ");
            status.push_str(&self.locale.diagnostic(error));
        }
        self.field("background_status", status.into());
        let jobs = value["jobs"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        self.state["syncSummary"] = json!({"total":jobs.len(),
            "syncing":jobs.iter().filter(|j|j["state"]=="syncing").count(),
            "paused":jobs.iter().filter(|j|j["paused"]==true || j["state"]=="paused").count(),
            "attention":jobs.iter().filter(|j|matches!(j["state"].as_str(),Some("conflict"|"conflicts"|"blocked"|"error"|"offline"))).count()});
        self.state["jobs"]=json!(value["jobs"].as_array().into_iter().flatten().map(|job|{
            let state=text(job,"state");
            let label=match state {"idle"=>"Aktuell","syncing"=>"Wird synchronisiert","paused"=>"Pausiert","conflicts"|"conflict"=>"Konflikt","blocked"=>"Blockiert","offline"=>"Offline","error"=>"Failed","stopped"=>"Dienst ist gestoppt",""|"waiting"=>"Waiting",other=>other};
            let conflicts=job["conflicts"].as_array().into_iter().flatten().take(5).map(|v|self.locale.diagnostic(v.as_str().unwrap_or(""))).collect::<Vec<_>>().join("; ");
            json!({"id":job["id"],"name":job["name"].as_str().filter(|v|!v.is_empty()).unwrap_or(text(job,"local_root")),
                "state":state,"progress":job["progress"],"completed":job["completed"],"lastSync":job["last_sync"],"retryAt":job["retry_at"],"conflicts":job["conflicts"],"skipped":job["skipped"],"computer":text(job,"remote_root").starts_with("/devices/"),"detail":format!("{} ↔ {}",text(job,"local_root"),text(job,"remote_root")),"paused":job["paused"]==true,
                "status":format!("{} · {} {}",self.tr(label),self.locale.diagnostic(text(job,"error")),conflicts)})
        }).collect::<Vec<_>>());
    }
    fn activate_background(&mut self, computer: bool) -> Vec<Effect> {
        if self.state["demo"] == true {
            self.message(
                &self.tr("Im Demo-Modus werden keine Hintergrund-Aufträge angelegt."),
                false,
            );
            return vec![];
        }
        let Some(local) = self.background_local.clone() else {
            self.message(
                &self.tr("Bitte zuerst den lokalen Sync-Ordner auswählen."),
                true,
            );
            return vec![];
        };
        let remote = text(&self.state["fields"]["background_remote"], "value").to_string();
        let (title, description) = if computer {
            ("Computersicherung einrichten","Der gewählte Ordner wird unter Computer in Proton Drive synchronisiert. Änderungen gelten in beide Richtungen; bestätigte Löschungen werden in den Papierkorb weitergegeben. Dies ist kein Systemabbild. Bestehende Cloud-Ordner werden nicht automatisch zusammengeführt.")
        } else {
            ("Automatischen Sync aktivieren?","Neue und geänderte Dateien werden künftig automatisch übertragen. Nach einem bestätigten gemeinsamen Stand werden Löschungen auf der Gegenseite in den Papierkorb übernommen. Bei gleichzeitigen Änderungen bleiben Dateien als Konflikt erhalten. Flatpak fragt zusätzlich nach Hintergrundbetrieb und Autostart.")
        };
        self.dialog(
            "confirm",
            title,
            description,
            json!({}),
            Answer::ActivateBackground {
                local,
                remote,
                computer,
            },
        )
    }
    fn submit_target(
        &mut self,
        title: &str,
        operation: Value,
        done: Done,
        target: Target,
    ) -> Vec<Effect> {
        let mut effects = self.submit(title, operation, done);
        if let Some(Effect::Work(work)) = effects.first_mut() {
            work.target = target;
            self.active = Some(work.clone());
        }
        effects
    }
    fn mount_action(&mut self, operation: Value) -> Vec<Effect> {
        if self.state["demo"] == true {
            self.message(
                &self.tr("Im Demo-Modus wird kein Laufwerk verbunden oder verändert."),
                false,
            );
            return vec![];
        }
        if self.snap || self.flatpak {
            self.message(&self.tr("Online-Platzhalter sind in dieser Sandbox nicht verfügbar. Normale Ordnersynchronisation ist in den Einstellungen verfügbar."),true);
            return vec![];
        }
        self.submit_target("Online-Laufwerk", operation, Done::Mount, Target::Mount)
    }
    fn render_mount(&mut self, value: &Value) {
        let mut status = self.tr(if value["mounted"] == true {
            "Verbunden"
        } else {
            "Nicht verbunden"
        });
        let path = text(value, "mountpoint");
        if !path.is_empty() {
            status.push_str(" · ");
            status.push_str(path);
        }
        let recovery = value
            .get("pending_recovery")
            .or_else(|| value.get("recovery"))
            .and_then(Value::as_array);
        if let Some(recovery) = recovery.filter(|v| !v.is_empty()) {
            status.push_str(
                &self.tr("\nNicht hochgeladene Änderungen im Wiederherstellungsspeicher: "),
            );
            status.push_str(&json!(recovery).to_string());
        }
        let error = text(value, "error");
        if !error.is_empty() {
            status.push('\n');
            status.push_str(&self.locale.diagnostic(error));
        }
        self.field("mount_status", status.into());
        if self.mount_local.is_none() {
            if let Some(path) = value["configuration"]["mountpoint"]
                .as_str()
                .filter(|p| !p.is_empty())
            {
                self.mount_local = Some(path.into());
                self.field("mount_label", json!(path));
                self.field(
                    "mount_remote",
                    json!(value["configuration"]["remote_root"]
                        .as_str()
                        .unwrap_or("/my-files")),
                );
            }
        }
    }
    fn render_trash(&mut self, value: &Value) {
        let records = value.as_array().cloned().unwrap_or_default();
        let mut status = self
            .tr("{p0} registrierte Einträge")
            .replace("{p0}", &records.len().to_string());
        if self.state["demo"] == true {
            status.push_str(&self.tr(" · Demo ohne Dateizugriff"));
        }
        self.field("local_trash_status", status.into());
        self.state["fields"]["local_trash_empty"]["visible"] = records.is_empty().into();
        self.state["trash"]=json!(records.iter().map(|value|{
            let record=&value["record"];let status=text(record,"status");
            let label=match status{"trashed"=>"In trash","prepared"=>"Preparing","restoring"=>"Restoring","restored"=>"Restored","recovery"=>"Recovery required","missing"=>"Missing",other=>other};
            json!({"state":value["state"],"id":record["id"],"name":text(record,"original_path"),
                "detail":format!("{} · {}",self.tr(label),text(record,"trash_path")),"error":self.locale.diagnostic(text(record,"error")),
                "restorePath":text(record,"restore_path"),"enabled":self.state["demo"]!=true && ["trashed","prepared","restoring","recovery"].contains(&status)})
        }).collect::<Vec<_>>());
    }
    fn refresh_trash(&mut self) -> Vec<Effect> {
        if self.state["demo"] == true {
            self.render_trash(&json!([]));
            return vec![];
        }
        self.submit_target(
            "Lokalen Papierkorb laden",
            json!({"action":"list"}),
            Done::TrashList,
            Target::Trash,
        )
    }
    fn render_account_storage(&mut self, value: &Value) {
        if let (Some(total), Some(used)) = (value["total"].as_u64(), value["used"].as_u64()) {
            let clean = |key: &str| {
                value["account"][key]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 320 && !s.chars().any(char::is_control))
                    .unwrap_or("")
                    .to_string()
            };
            self.state["account"] = json!({"name":clean("name"),"email":clean("email")});
            self.state["storage"] = json!({"total":total,"used":used,"free":total.saturating_sub(used),"percent":if total>0 {100.0*used as f64/total as f64}else{0.0},"usedText":self.locale.size_text(Some(used)),"freeText":self.locale.size_text(Some(total.saturating_sub(used)))});
        }
    }
    pub fn service_polled(&mut self, target: Target, generation: u64, value: &Value) {
        if self.next_work != generation {
            return;
        }
        match target {
            Target::AccountStorage if self.state["authenticated"] == true => {
                self.render_account_storage(value)
            }
            Target::Background => self.background_polled(generation, value),
            Target::Mount if self.state["page"] == "ondemand" => self.render_mount(value),
            _ => {}
        }
    }
    pub fn refresh(&mut self) -> Vec<Effect> {
        if self.busy() {
            return vec![];
        }
        self.submit(
            "Dateiliste laden",
            json!({"action":"list","path":self.path()}),
            Done::Loaded,
        )
    }
    pub fn progress(&mut self, id: u64, event: Value) {
        if self.active.as_ref().is_some_and(|w| w.id == id) {
            self.state["progress"] = event.clone();
            if let Some(record) = self.state["activity"]
                .as_array_mut()
                .and_then(|a| a.first_mut())
            {
                record["progress"] = event;
            }
        }
    }
    pub fn login_ready(&mut self, id: u64, url: String) {
        if self
            .active
            .as_ref()
            .is_none_or(|w| w.id != id || w.operation["action"] != "login")
        {
            return;
        }
        self.login_url = Some(url);
        self.state["fields"]["login_button"]["visible"] = true.into();
        self.message(&self.tr("Bitte die Anmeldung im geöffneten Proton-Browserfenster abschließen. Falls kein Fenster erscheint, den Anmeldeknopf verwenden."),false);
    }
    pub fn complete(&mut self, id: u64, result: Result<Value, Failure>) -> Vec<Effect> {
        if self.active.as_ref().is_none_or(|w| w.id != id) {
            return vec![];
        }
        let work = self.active.take().unwrap();
        // Validate list envelopes before recording success or replacing the
        // current view. Malformed data is not an empty folder/trash listing.
        let result = match result {
            Ok(value)
                if matches!(&work.done, Done::Loaded | Done::TrashList) && !value.is_array() =>
            {
                Err(Failure {
                    message: if work.done == Done::Loaded {
                        "Invalid native file list"
                    } else {
                        "Invalid trash list"
                    }
                    .into(),
                    cancelled: false,
                })
            }
            other => other,
        };
        self.login_url = None;
        self.state["busy"] = false.into();
        self.state["reading"] = json!(false);
        for key in ["spinner", "cancel_button", "login_button"] {
            self.state["fields"][key]["visible"] = false.into();
        }
        self.state["fields"]["file_list"]["enabled"] = true.into();
        self.update_actions();
        if self.closing {
            return vec![Effect::Close];
        }
        if work.done == Done::CloseStatus {
            return if result.is_ok_and(|value| value["running"] == true) {
                vec![Effect::Hide]
            } else {
                self.closing = true;
                vec![Effect::Close]
            };
        }
        match result {
            Err(error) => {
                let message = self.locale.diagnostic(&error.message);
                self.state["activity"][0]["status"] =
                    format!("{}{message}", self.tr("Fehlgeschlagen · ")).into();
                self.field(
                    "status_label",
                    json!(format!(
                        "{}{}",
                        work.title,
                        self.tr(" · nicht abgeschlossen")
                    )),
                );
                self.message(&message, true);
                vec![]
            }
            Ok(value) => {
                self.state["activity"][0]["status"] = self.tr("Abgeschlossen").into();
                self.field(
                    "status_label",
                    json!(format!("{}{}", work.title, self.tr(" · abgeschlossen"))),
                );
                if value.get("transferredItems").is_some() {
                    let summary = self
                        .tr("{p0} übertragen · {p1} übersprungen · {p2}")
                        .replace("{p0}", &value["transferredItems"].to_string())
                        .replace(
                            "{p1}",
                            &value["skippedItems"].as_u64().unwrap_or(0).to_string(),
                        )
                        .replace(
                            "{p2}",
                            &self
                                .locale
                                .size_text(Some(value["transferredBytes"].as_u64().unwrap_or(0))),
                        );
                    self.state["activity"][0]["status"] = summary.clone().into();
                    self.message(&summary, false);
                }
                match work.done {
                    Done::Setup => {
                        let downloaded = (self.snap || self.flatpak)
                            && value["mode"] == "download"
                            && value["running"] == true;
                        if value["mounted"] != true && !downloaded {
                            self.message("Setup did not start the sync service", true);
                            return vec![];
                        }
                        if !downloaded {
                            self.render_mount(&value);
                        }
                        self.mount_local = self.state["setupFolder"].as_str().map(str::to_string);
                        self.field("mount_label", self.state["setupFolder"].clone());
                        self.field("mount_remote", json!("/my-files"));
                        self.field("mount_writable", json!(true));
                        self.field("mount_offline", json!(false));
                        let mut store = crate::preferences::Store::interface();
                        if !store.save("setup", "complete", "") {
                            self.message("Could not save setup completion", true);
                            return vec![];
                        }
                        self.state["setupNeeded"] = json!(false);
                        self.message(
                            if downloaded { "Sync is running. Files are downloaded to your sync folder." } else { "Sync is running. Files download when opened or kept on this device." },
                            false,
                        );
                        vec![]
                    }
                    Done::Mount => {
                        self.render_mount(&value);
                        self.message(&self.tr("Laufwerksaktion abgeschlossen."), false);
                        vec![]
                    }
                    Done::TrashList => {
                        self.render_trash(&value);
                        vec![]
                    }
                    Done::TrashRestored => {
                        self.message(&self.tr("Datei wiederhergestellt; bestehende Dateien wurden nicht überschrieben."),false);
                        self.refresh_trash()
                    }
                    Done::Background => {
                        self.render_background(&value);
                        if work.operation["action"] == "remove_computer" {
                            return self.refresh();
                        }
                        vec![]
                    }
                    Done::Versions(path) => {
                        if value["revisions"].is_array() && value["active"].is_string() {
                            self.state["history"] = json!({"visible":true,"path":path,"active":value["active"],"revisions":value["revisions"]});
                        } else {
                            self.message("Invalid version history response", true);
                        }
                        vec![]
                    }
                    Done::VersionRestored => {
                        if value["accepted"] != true {
                            self.message("Invalid version restoration response", true);
                            return vec![];
                        }
                        self.state["history"]["visible"] = json!(false);
                        self.message(
                            "Version restoration requested. Refresh to check the server result.",
                            false,
                        );
                        self.refresh()
                    }
                    Done::TakeoutScan => {
                        let count = value["items"].as_array().map_or(0, Vec::len);
                        if count == 0 {
                            self.message("No supported photos or videos found.", true);
                            return vec![];
                        }
                        let description=format!("{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n\n{}",self.tr("Media files"),count,self.tr("Videos"),value["videos"],self.tr("Duplicate files"),value["duplicates"],self.tr("Ignored files"),value["ignored"],self.tr("Metadata warnings"),value["warnings"],self.tr("Import into Proton Photos? Originals stay unchanged. Albums and comments are not imported. You can cancel and retry; completed uploads are kept."));
                        self.dialog(
                            "confirm",
                            "Google Takeout import",
                            &description,
                            json!({}),
                            Answer::TakeoutImport(value),
                        )
                    }
                    Done::TakeoutImport => {
                        let report = format!(
                            "{}: {} · {}: {} · {}: {} · {}: {}",
                            self.tr("Imported"),
                            value["uploaded"],
                            self.tr("Skipped"),
                            value["skipped"],
                            self.tr("Failed"),
                            value["failed"],
                            self.tr("Remaining"),
                            value["remaining"]
                        );
                        self.message(
                            &format!("{report}\n{}", value["error"].as_str().unwrap_or("")),
                            value["failed"].as_u64().unwrap_or(0) > 0,
                        );
                        vec![]
                    }
                    Done::PhotoLibrary(request) => self.photo_result(&request, value),
                    Done::Photos => {
                        if value["photos"].is_array()
                            && value["offset"].is_u64()
                            && value["more"].is_boolean()
                        {
                            let photos = value["photos"].as_array().unwrap();
                            if photos.len() > 48 {
                                self.message("Invalid photo page size", true);
                                return vec![];
                            }
                            self.state["photos"] = json!(photos
                                .iter()
                                .cloned()
                                .map(|mut photo| {
                                    let thumbnail = photo["thumbnail"].as_str().unwrap_or("");
                                    if !thumbnail.starts_with("data:image/jpeg;base64,")
                                        || thumbnail.len() > 710000
                                    {
                                        photo["thumbnail"] = json!("");
                                    }
                                    photo
                                })
                                .collect::<Vec<_>>());
                            self.photos_revision += 1;
                            self.state["photoOffset"] = value["offset"].clone();
                            self.state["photosMore"] = value["more"].clone();
                        } else {
                            self.message("Invalid photo page response", true);
                        }
                        vec![]
                    }
                    Done::Loaded => {
                        if let Some(nodes) = value.as_array() {
                            self.state["authenticated"] = json!(true);
                            let mut sorted = nodes.clone();
                            sorted.sort_by_cached_key(|n| {
                                (n["type"] != "folder", folded(text(n, "name")))
                            });
                            let changed = self.files_revision == 0 || self.nodes != sorted;
                            self.nodes = sorted;
                            self.cache_folder();
                            // Unchanged metadata must not recreate delegates or
                            // discard the user's keyboard focus and selection.
                            if changed {
                                self.render();
                            }
                        } else {
                            self.message("Invalid native file list", true);
                        }
                        vec![]
                    }
                    Done::Refresh => self.refresh(),
                    Done::FileManager => {
                        if value["share"].is_object() {
                            let node = value["share"].clone();
                            self.submit(
                                "Freigabestatus laden",
                                json!({"action":"sharing","path":node["path"]}),
                                Done::Share(node),
                            )
                        } else {
                            vec![]
                        }
                    }
                    Done::Share(node) => self.share_dialog(node, value),
                    Done::ShareReload(node) => self.submit(
                        "Freigabestatus laden",
                        json!({"action":"sharing","path":node["path"]}),
                        Done::Share(node),
                    ),
                    Done::Version => {
                        self.field("backend_label", json!(value.as_str().unwrap_or("")));
                        vec![]
                    }
                    Done::LoggedOut => {
                        self.state["authenticated"] = json!(false);
                        self.state["storage"] = Value::Null;
                        self.state["account"] = Value::Null;
                        self.state["photos"] = json!([]);
                        self.state["photoAlbums"] = json!([]);
                        self.state["photoTotal"] = Value::Null;
                        self.state["photoAlbum"] = json!("");
                        self.state["photoOffset"] = json!(0);
                        self.state["photosMore"] = json!(false);
                        self.state["photoSelection"] = json!([]);
                        self.state["history"] = json!({"visible":false,"revisions":[]});
                        self.state["page"] = json!("files");
                        self.state["path"] = json!("/my-files");
                        self.state["settingsSync"] = json!(false);
                        let title = self.tr("Meine Dateien");
                        self.crumbs = vec![(title.clone(), "/my-files".into())];
                        self.state["crumbs"] = json!([title]);
                        self.field("files_title", json!(title));
                        self.field("search", json!(""));
                        self.photo_index.clear();
                        self.photo_checked = None;
                        self.photos_revision += 1;
                        self.folder_cache.clear();
                        self.nodes.clear();
                        self.render();
                        self.message(&self.tr("Von Proton abgemeldet."), false);
                        vec![]
                    }
                    Done::Autostart(enabled) => {
                        self.message(
                            &self.tr(if enabled {
                                "Autostart aktiviert."
                            } else {
                                "Autostart deaktiviert."
                            }),
                            false,
                        );
                        vec![]
                    }
                    Done::CloseStatus => unreachable!("handled before result rendering"),
                    Done::None => vec![],
                }
            }
        }
    }
    fn dialog(
        &mut self,
        kind: &str,
        title: &str,
        description: &str,
        options: Value,
        answer: Answer,
    ) -> Vec<Effect> {
        self.serial += 1;
        self.pending = Some((self.serial, answer));
        let mut value = json!({"kind":kind,"title":self.tr(title),"description":self.tr(description),"token":self.serial});
        for (k, v) in options.as_object().into_iter().flatten() {
            value[k] = v.clone();
        }
        vec![Effect::Dialog(value)]
    }
    fn share_dialog(&mut self, node: Value, value: Value) -> Vec<Effect> {
        let url = text(&value["urlAccess"], "url").to_string();
        let title = format!("{}{}", self.tr("Freigabe · "), text(&node, "name"));
        self.dialog("share",&title,"Manage access by link or email. Empty password and expiration fields remove these restrictions.",json!({"url":url,"node":node,"linkSettings":{"uid":text(&value["urlAccess"],"uid"),"role":value["urlAccess"]["role"].as_str().unwrap_or("viewer"),"password":text(&value["urlAccess"],"customPassword"),"expiration":text(&value["urlAccess"],"expirationTime"),"editorsCanShare":value["editorsCanShare"].as_bool().unwrap_or(false)}}),Answer::Share{node,value})
    }
    fn local_paths(value: &Value) -> Vec<String> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
            .filter_map(|url| QUrl::from(url).to_local_file())
            .map(|path| path.to_string())
            .filter(|p| !p.is_empty())
            .collect()
    }
    fn answer(&mut self, token: u64, accepted: bool, value: &Value) -> Vec<Effect> {
        if self
            .pending
            .as_ref()
            .is_none_or(|(current, _)| *current != token)
        {
            return vec![];
        }
        if accepted
            && self.active_read()
            && !matches!(self.pending.as_ref(), Some((_, Answer::Close)))
        {
            // A dialog captures its target before navigation. Let its confirmed
            // action supersede a read instead of consuming the answer while
            // submit() is still busy and silently losing the requested action.
            let mut effects = self.abandon_read();
            effects.extend(self.answer(token, accepted, value));
            return effects;
        }
        let (_, answer) = self.pending.take().unwrap();
        if !accepted {
            return match answer {
                Answer::Invite { node, value, .. }
                | Answer::Revoke { node, value }
                | Answer::LinkSettings { node, value, .. } => self.share_dialog(node, value),
                _ => vec![],
            };
        }
        let string = value.as_str().unwrap_or("");
        let response = value.clone();
        match answer {
            Answer::Invite { node, email, role, .. } => self.submit("Share by email",json!({"action":"invite","path":node["path"],"email":email,"role":role}),Done::ShareReload(node)),
            Answer::LinkSettings { node, settings, .. } => self.submit("Save link settings", json!({"action":"set_link","path":node["path"],"settings":settings}), Done::ShareReload(node)),
            Answer::TakeoutFolder => {
                let Some(root)=Self::local_paths(value).into_iter().next() else { return vec![]; };
                self.submit("Google Takeout import",json!({"action":"takeout_scan","root":root}),Done::TakeoutScan)
            }
            Answer::TakeoutImport(plan) => self.submit("Google Takeout import",json!({"action":"takeout_import","plan":plan}),Done::TakeoutImport),
            Answer::Close => {self.closing=true;self.field("status_label",json!(self.tr("Vorgang wird beendet …")));if self.busy(){vec![Effect::Cancel]}else{vec![Effect::Close]}},
            Answer::MountFolder=>{if let Some(path)=Self::local_paths(value).into_iter().next(){self.field("mount_label",json!(path));self.mount_local=Some(path);}vec![]},
            Answer::ComputerFolder => {if let Some(path)=Self::local_paths(value).into_iter().next(){self.background_local=Some(path);return self.activate_background(true);}vec![]},
            Answer::BackgroundRemoval(operation) => self.submit_background("Remove backup",operation),
            Answer::BackgroundFolder => {if let Some(path)=Self::local_paths(value).into_iter().next(){self.field("background_local_label",json!(path));self.background_local=Some(path);}vec![]},
            Answer::ActivateBackground{local,remote,computer}=>self.submit_background(if computer {"Computersicherung einrichten"} else {"Hintergrund-Sync aktivieren"},json!({"action":if computer {"computer"} else {"add"},"local":local,"remote":remote})),
            Answer::PhotoLibrary(request) => {
                    let mut request = request;
                    if matches!(request["action"].as_str(), Some("create" | "rename")) {
                        request["name"] = json!(value);
                    }
                    self.photo_submit(request)
                }
                Answer::DownloadPhoto { uid } => {
                let Some(destination)=Self::local_paths(value).into_iter().next() else { return vec![]; };
                self.submit("Download",json!({"action":"download_photo","uid":uid,"destination":destination}),Done::None)
            }
            Answer::RestoreVersion { path, revision, expected } => self.submit("Version history",json!({"action":"restore_version","path":path,"revision":revision,"expected":expected}),Done::VersionRestored),
            Answer::Logout => self.submit("Abmelden", json!({"action":"logout"}), Done::LoggedOut),
            Answer::Upload { parent } => {
                let paths = Self::local_paths(value);
                if paths.is_empty() {
                    return vec![];
                }
                self.submit("Upload",json!({"action":"upload","paths":paths,"parent":parent,"file_strategy":"rename","folder_strategy":"merge"}),Done::Refresh)
            }
            Answer::Download { paths } => {
                let destinations = Self::local_paths(value);
                let Some(destination) = destinations.first() else {
                    return vec![];
                };
                self.submit("Download",json!({"action":"download","paths":paths,"destination":destination,"file_strategy":"rename","folder_strategy":"merge"}),Done::None)
            }
            Answer::Mkdir { parent } => self.submit(
                "Ordner erstellen",
                json!({"action":"mkdir","parent":parent,"name":string}),
                Done::Refresh,
            ),
            Answer::Rename { path } => self.submit(
                "Umbenennen",
                json!({"action":"rename","path":path,"name":string}),
                Done::Refresh,
            ),
            Answer::Move { paths } => self.submit(
                "Verschieben",
                json!({"action":"move","paths":paths,"destination":string}),
                Done::Refresh,
            ),
            Answer::Trash { paths } => self.submit(
                "Papierkorb",
                json!({"action":"trash","paths":paths}),
                Done::Refresh,
            ),
            Answer::Revoke { node, .. } => self.submit(
                "Freigabelink entfernen",
                json!({"action":"remove_link","path":node["path"]}),
                Done::ShareReload(node),
            ),
            Answer::Share { node, value } => {
                if response["action"] == "settings" {
                    let mut settings = response.clone();
                    settings.as_object_mut().unwrap().remove("action");
                    settings["nodeUid"] = node["uid"].clone();
                    settings["expected"] = json!({"uid":text(&value["urlAccess"],"uid"),"role":value["urlAccess"]["role"].as_str().unwrap_or("viewer"),"password":text(&value["urlAccess"],"customPassword"),"expiration":text(&value["urlAccess"],"expirationTime"),"editorsCanShare":value["editorsCanShare"].as_bool().unwrap_or(false)});
                    return self.dialog("confirm","Save link settings?","Anyone with the link receives the selected access. Empty password and expiration fields remove these restrictions.",json!({}),Answer::LinkSettings {node,value,settings});
                }
                if response["action"] == "invite" {
                    let email=text(&response,"email").trim().to_string();
                    let role=text(&response,"role").to_string();
                    if email.is_empty() || !["viewer","editor"].contains(&role.as_str()) { self.message("Enter an email address and choose a role",true); return vec![]; }
                    return self.dialog("confirm","Send sharing invitation?",&format!("{}: {} ({})",self.tr("Recipient"),email,role),json!({}),Answer::Invite {node,value,email,role});
                }
                let url = text(&value["urlAccess"], "url");
                match string {
                    "copy" if !url.is_empty() => {
                        let mut effects = vec![Effect::Clipboard(url.into())];
                        effects.extend(self.share_dialog(node, value));
                        effects
                    }
                    "revoke" if !url.is_empty() => self.dialog(
                        "confirm",
                        "Freigabelink deaktivieren?",
                        "Der bisherige Link funktioniert anschließend nicht mehr.",
                        json!({}),
                        Answer::Revoke { node, value },
                    ),
                    "create" if url.is_empty() => self.submit(
                        "Freigabelink erstellen",
                        json!({"action":"create_link","path":node["path"]}),
                        Done::Share(node),
                    ),
                    _ => vec![],
                }
            }
        }
    }
    fn cache_folder(&mut self) {
        let path = self.path().to_owned();
        self.folder_cache.retain(|entry| entry.path != path);
        let bytes = serde_json::to_vec(&self.nodes).map_or(usize::MAX, |v| v.len());
        if bytes > 8 * 1024 * 1024 {
            return;
        }
        while self.folder_cache.len() >= 32
            || self
                .folder_cache
                .iter()
                .map(|entry| entry.bytes)
                .sum::<usize>()
                + bytes
                > 8 * 1024 * 1024
        {
            self.folder_cache.pop_front();
        }
        self.folder_cache.push_back(CachedFolder {
            path,
            nodes: self.nodes.clone(),
            saved: Instant::now(),
            bytes,
        });
    }
    fn restore_folder(&mut self) -> bool {
        let path = self.path().to_owned();
        let cached = self
            .folder_cache
            .iter()
            .position(|entry| entry.path == path)
            .and_then(|index| self.folder_cache.remove(index));
        self.nodes = cached
            .as_ref()
            .map_or_else(Vec::new, |entry| entry.nodes.clone());
        let fresh = cached
            .as_ref()
            .is_some_and(|entry| entry.saved.elapsed() < Duration::from_secs(30));
        if let Some(entry) = cached {
            self.folder_cache.push_back(entry);
        }
        self.render();
        fresh
    }
    fn enter_photos(&mut self) -> Vec<Effect> {
        // Returning to the gallery preserves the album, loaded pages and previews.
        // Explicit Refresh still rebuilds the inventory from the cloud.
        if self.state["photoTotal"].is_u64() {
            if self
                .photo_checked
                .is_some_and(|saved| saved.elapsed() >= Duration::from_secs(30))
            {
                return self.photo_action(
                    &json!({"action":"index", "album":self.state["photoAlbum"], "preserve":true}),
                );
            }
            if self.state["photos"].as_array().is_some_and(Vec::is_empty)
                && !self.photo_index.is_empty()
            {
                return self.photo_action(&json!({"action":"more"}));
            }
            return vec![];
        }
        self.photo_action(&json!({"action":"index", "album":self.state["photoAlbum"]}))
    }
    fn abandon_read(&mut self) -> Vec<Effect> {
        if !self.active_read() {
            return vec![];
        }
        self.active = None; // A late response no longer belongs to this view.
        self.state["busy"] = json!(false);
        self.state["reading"] = json!(false);
        self.state["activity"][0]["status"] = json!(self.tr("Abgebrochen"));
        for key in ["spinner", "cancel_button", "login_button"] {
            self.state["fields"][key]["visible"] = json!(false);
        }
        self.state["fields"]["file_list"]["enabled"] = json!(true);
        self.field("status_label", json!(self.tr("Bereit")));
        self.update_actions();
        vec![Effect::Cancel]
    }
    fn active_read(&self) -> bool {
        self.active.as_ref().is_some_and(|work| {
            matches!(work.done, Done::Loaded | Done::Photos | Done::TrashList)
                || matches!(&work.done, Done::PhotoLibrary(request) if matches!(text(request, "action"), "index" | "details" | "albums"))
        })
    }
    fn open_section(&mut self, title: &str, destination: &str) -> Vec<Effect> {
        let file_section = [
            "/my-files",
            "/shared-with-me",
            "/shared-by-me",
            "/devices",
            "/trash",
        ]
        .contains(&destination);
        if !file_section
            && ![
                "photos",
                "local-trash",
                "activity",
                "settings",
                "computer-backup",
                "ondemand",
            ]
            .contains(&destination)
        {
            return vec![];
        }
        if self.busy() && !self.active_read() {
            // Uploads, deletes, login and other transactions keep their
            // existing completion/confirmation semantics.
            self.message(
                &self.tr("Es läuft bereits ein Vorgang. Bitte warten oder abbrechen."),
                false,
            );
            return vec![];
        }
        let mut effects = self.abandon_read();
        self.message("", false);
        self.state["history"]["visible"] = json!(false);
        self.state["page"] = json!(if file_section { "files" } else { destination });
        self.state["settingsSync"] = json!(false);
        if file_section {
            self.crumbs = vec![(title.into(), destination.into())];
            self.state["path"] = json!(destination);
            self.state["crumbs"] = json!([title]);
            self.field("files_title", json!(title));
            self.field("search", json!(""));
            let fresh = self.restore_folder();
            if !fresh {
                effects.extend(self.refresh());
            }
        } else if destination == "photos" {
            effects.extend(self.enter_photos());
        } else {
            effects.extend(self.dispatch("show_page", &json!([destination])));
        }
        effects
    }

    fn navigate(&mut self) -> Vec<Effect> {
        let mut effects = self.abandon_read();
        let (title, path) = self.crumbs.last().unwrap().clone();
        self.state["path"] = path.into();
        self.state["page"] = "files".into();
        self.state["crumbs"] = json!(self
            .crumbs
            .iter()
            .map(|(title, _)| title)
            .collect::<Vec<_>>());
        self.field("files_title", title.into());
        self.field("search", "".into());
        if !self.restore_folder() {
            effects.extend(self.refresh());
        }
        effects
    }
    pub fn dispatch(&mut self, method: &str, args: &Value) -> Vec<Effect> {
        if self.closing {
            return vec![];
        }
        match method {
            "open_section" => {
                return self.open_section(
                    args[0].as_str().unwrap_or(""),
                    args[1].as_str().unwrap_or(""),
                )
            }
            "request_close" => {
                if args[0] == true {
                    self.message(
                        &self.tr("Update wird installiert. Bitte die Systemabfrage bestätigen."),
                        false,
                    );
                    return vec![];
                }
                // Hiding a window does not cancel work. Only explicit exit (or
                // absence of a tray host) needs the cancellation confirmation.
                if args[1] == true {
                    return vec![Effect::Hide];
                }
                if self.busy() {
                    return self.dialog("confirm","Laufenden Vorgang abbrechen?","Bereits übertragene Dateien bleiben erhalten. Die App wartet auf das Ende des Backends.",json!({}),Answer::Close);
                }
                if self.flatpak && self.state["demo"] != true {
                    return self.submit_target(
                        "Hintergrund-Sync",
                        json!({"action":"status"}),
                        Done::CloseStatus,
                        Target::Background,
                    );
                }
                self.closing = true;
                return vec![Effect::Close];
            }
            "restore_entry" if !self.busy() => {
                if let Some(entry) = args[0]
                    .as_u64()
                    .and_then(|i| {
                        self.state["trash"]
                            .as_array()
                            .and_then(|a| a.get(i as usize))
                    })
                    .cloned()
                    .filter(|v| v["enabled"] == true)
                {
                    return self.submit_target(
                        "Datei wiederherstellen",
                        json!({"action":"restore","state":entry["state"],"id":entry["id"]}),
                        Done::TrashRestored,
                        Target::Trash,
                    );
                }
            }
            "edit"
                if !self.busy()
                    && ["mount_remote", "cache_path"].contains(&args[0].as_str().unwrap_or("")) =>
            {
                self.field(
                    args[0].as_str().unwrap(),
                    json!(args[1].as_str().unwrap_or("")),
                )
            }
            "edit"
                if !self.busy()
                    && ["mount_writable", "mount_offline"]
                        .contains(&args[0].as_str().unwrap_or("")) =>
            {
                self.field(args[0].as_str().unwrap(), json!(args[1] == true))
            }
            "show_page"
                if [
                    "files",
                    "photos",
                    "background",
                    "computer-backup",
                    "ondemand",
                    "local-trash",
                    "activity",
                    "settings",
                ]
                .contains(&args[0].as_str().unwrap_or("")) =>
            {
                self.state["page"] = if args[0] == "background" {
                    json!("settings")
                } else {
                    args[0].clone()
                };
                self.state["settingsSync"] = json!(args[0] == "background");
                let mut effects = self.abandon_read();
                self.message("", false);
                if args[0] == "local-trash" {
                    effects.extend(self.refresh_trash());
                } else if args[0] == "photos" {
                    effects.extend(self.enter_photos());
                } else if args[0] == "files" {
                    if !self.restore_folder() {
                        effects.extend(self.refresh());
                    }
                } else if [
                    "background",
                    "settings",
                    "computer-backup",
                    "ondemand",
                    "activity",
                ]
                .contains(&args[0].as_str().unwrap_or(""))
                {
                    effects.extend(self.dispatch("poll_services", &json!([])));
                }
                return effects;
            }

            "poll_services" if !self.busy() && self.state["demo"] != true => {
                return vec![Effect::PollService(if self.state["page"] == "ondemand" {
                    Target::Mount
                } else {
                    Target::Background
                })]
            }
            "account_storage" if !self.busy() && self.state["authenticated"] == true => {
                return vec![Effect::PollService(Target::AccountStorage)];
            }
            "share_open" if !self.busy() && self.pending.is_none() && args[0]["safe"] == true => {
                let node = args[0].clone();
                return self.submit(
                    "Freigabestatus laden",
                    json!({"action":"sharing","path":node["path"]}),
                    Done::Share(node),
                );
            }
            "file_manager" if !self.busy() && self.pending.is_none() => {
                if let Err(error) = crate::file_manager::validate(&args[0]) {
                    self.message(&error.message, true);
                    return vec![];
                }
                return self.submit_target(
                    "NeutronDrive",
                    args[0].clone(),
                    Done::FileManager,
                    Target::FileManager,
                );
            }
            "setup_folder_url" if !self.busy() => {
                if let Some(path) = Self::local_paths(&json!([args[0]])).into_iter().next() {
                    self.state["setupFolder"] = json!(path);
                }
            }
            "setup_folder" if !self.busy() => {
                self.state["setupFolder"] = args[0].clone();
            }
            "setup_finish" if !self.busy() => {
                if self.state["demo"] == true {
                    self.message("Setup is unavailable in demo mode.", true);
                    return vec![];
                }
                return self.submit_target(
                    "Set up sync",
                    json!({"action":"setup","mountpoint":self.state["setupFolder"]}),
                    Done::Setup,
                    Target::Mount,
                );
            }
            "setup_open" if !self.busy() => {
                self.state["setupNeeded"] = json!(true);
            }
            "setup_later" => {
                self.state["setupNeeded"] = json!(false);
            }
            "photo_library" if !self.busy() => {
                return self.photo_action(&args[0]);
            }
            "photos_page" if !self.busy() => {
                let Some(offset) = args[0].as_u64() else {
                    return vec![];
                };
                return self.submit(
                    "Photos",
                    json!({"action":"photos","offset":offset}),
                    Done::Photos,
                );
            }
            "photo_download" if !self.busy() => {
                let photo = args[0]
                    .as_u64()
                    .and_then(|index| self.state["photos"].get(index as usize))
                    .cloned();
                if let Some(photo) = photo.filter(|p| p["safe"] == true) {
                    return self.dialog(
                        "folder",
                        "Download-Zielordner wählen",
                        "",
                        json!({"multiple":false}),
                        Answer::DownloadPhoto {
                            uid: text(&photo, "uid").into(),
                        },
                    );
                }
            }
            "history_close" => {
                self.state["history"]["visible"] = json!(false);
            }
            "version_restore" if !self.busy() => {
                let history = self.state["history"].clone();
                if let Some(entry) = args[0]
                    .as_u64()
                    .and_then(|index| history["revisions"].get(index as usize))
                    .filter(|entry| entry["state"] == "superseded")
                {
                    return self.dialog("confirm","Restore this version?","The selected historical version will become the current file. Concurrent cloud edits require reloading the history.",json!({}),Answer::RestoreVersion{path:text(&history,"path").into(),revision:text(entry,"uid").into(),expected:text(&history,"active").into()});
                }
            }
            "remove_computer" => {
                if let Some(node) = self
                    .selected()
                    .first()
                    .cloned()
                    .filter(|n| self.path() == "/devices" && n["safe"] == true)
                {
                    return self.dialog("confirm","Remove this computer?","Stop its backup jobs and remove this computer and its cloud backup from Proton Drive. This may permanently delete the cloud copy. Local original files remain on this computer.",json!({}),Answer::BackgroundRemoval(json!({"action":"remove_computer","path":node["path"],"uid":node["raw"]["uid"],"root_uid":node["raw"]["rootFolderUid"]})));
                }
            }
            "job_action" if args[0] == "remove_backup" || args[0] == "remove" => {
                if let Some(job) = self.state["jobs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|j| j["id"] == args[1])
                    .cloned()
                {
                    return self.dialog("confirm",if args[0]=="remove_backup" {"Remove backup?"}else{"Stop syncing this folder?"},if args[0]=="remove_backup" {"Stop syncing and move this folder's cloud copy to Proton Drive trash. Local original files are kept."}else{"Stop syncing this folder. Local files and the cloud copy are kept."},json!({}),Answer::BackgroundRemoval(json!({"action":args[0],"id":job["id"]})));
                }
            }
            "job_action"
                if ["pause", "resume", "sync", "remove"]
                    .contains(&args[0].as_str().unwrap_or("")) =>
            {
                return self
                    .submit_background("Hintergrund-Sync", json!({"action":args[0],"id":args[1]}))
            }
            "edit" if args[0] == "background_remote" && !self.busy() => {
                self.field("background_remote", json!(args[1].as_str().unwrap_or("")))
            }
            "edit" if args[0] == "search" && (!self.busy() || self.active_read()) => {
                let next = args[1].as_str().unwrap_or("");
                let changed =
                    folded(text(&self.state["fields"]["search"], "value")) != folded(next);
                self.field("search", json!(next));
                // Equivalent Unicode queries have the same results. Preserve
                // delegates, keyboard focus and selection instead of rebuilding.
                if changed {
                    self.render();
                }
            }
            "select_row" => {
                if let Some(index) = args[0].as_u64() {
                    self.select(index as usize, args[1] == true);
                }
            }
            "select_range" => {
                if let Some(index) = args[0].as_u64() {
                    self.select_range(index as usize);
                }
            }
            "clear_selection" if !self.busy() || self.active_read() => {
                self.selection.clear();
                self.selection_anchor = None;
                self.update_actions();
            }
            "move_selection" if !self.visible.is_empty() => {
                let index = (self.state["focusIndex"]
                    .as_i64()
                    .unwrap_or(0)
                    .saturating_add(args[0].as_i64().unwrap_or(0)))
                .clamp(0, self.visible.len() as i64 - 1);
                if args[1] == true {
                    self.select_range(index as usize);
                } else {
                    self.select(index as usize, false);
                }
            }
            "select_all" if !self.busy() || self.active_read() => {
                self.selection = (0..self.visible.len()).collect();
                self.selection_anchor = None;
                self.update_actions();
            }
            "navigate" if !self.busy() || self.active_read() => {
                self.crumbs = vec![(
                    args[0].as_str().unwrap_or("").into(),
                    args[1].as_str().unwrap_or("").into(),
                )];
                return self.navigate();
            }
            "crumb" if !self.busy() || self.active_read() => {
                if let Some(index) = args[0].as_u64() {
                    if (index as usize) < self.crumbs.len() {
                        self.crumbs.truncate(index as usize + 1);
                        return self.navigate();
                    }
                }
            }
            "row_activated" if (!self.busy() || self.active_read()) && self.path() != "/trash" => {
                if let Some(node) = args[0]
                    .as_u64()
                    .and_then(|i| self.visible.get(i as usize))
                    .cloned()
                    .filter(|n| n["safe"] == true)
                {
                    if node["type"] == "folder" {
                        self.crumbs
                            .push((text(&node, "name").into(), text(&node, "path").into()));
                        return self.navigate();
                    }
                    self.select(args[0].as_u64().unwrap() as usize, false);
                    return self.action("Download");
                }
            }
            "answer_dialog" => {
                return self.answer(args[0].as_u64().unwrap_or(0), args[1] == true, &args[2])
            }
            "action" if args[0] == "cancel" => {
                if self.active_read() {
                    return self.abandon_read();
                }
                return vec![Effect::Cancel];
            }
            "action" => return self.action(args[0].as_str().unwrap_or("")),
            _ => {}
        }
        vec![]
    }
    fn action(&mut self, name: &str) -> Vec<Effect> {
        if name == "openLogin" {
            return self
                .login_url
                .as_ref()
                .map(|url| vec![Effect::OpenUrl(url.clone())])
                .unwrap_or_default();
        }
        if self.busy() {
            return vec![];
        }
        if let Some(action) = self.state["actions"].get(name) {
            if action["enabled"] != true || action["visible"] != true {
                return vec![];
            }
        }
        let nodes = self.selected();
        let paths = self.paths();
        match name {
            "autostart" | "noAutostart" => {
                if self.state["demo"] == true { return vec![]; }
                let enabled = name == "autostart";
                self.submit_target("Autostart", json!({"enabled":enabled}), Done::Autostart(enabled), Target::Autostart)
            },
            "chooseMount"=>self.dialog("folder","Leeren Einhängeordner wählen","",json!({"multiple":false}),Answer::MountFolder),
            "mount"=>{
                let Some(local)=self.mount_local.clone() else{self.message(&self.tr("Bitte zuerst einen leeren Einhängeordner auswählen."),true);return vec![];};
                self.mount_action(json!({"action":"mount","mountpoint":local,"remote_root":self.state["fields"]["mount_remote"]["value"],"writable":self.state["fields"]["mount_writable"]["value"],"offline":self.state["fields"]["mount_offline"]["value"]}))
            },
            "mountStatus"=>self.mount_action(json!({"action":"status"})),
            "unmount"=>self.mount_action(json!({"action":"unmount"})),
            "refreshMount"=>self.mount_action(json!({"action":"refresh"})),
            "helperLogin"=>self.mount_action(json!({"action":"login"})),
            "pin"|"evict"=>self.mount_action(json!({"action":name,"path":self.state["fields"]["cache_path"]["value"]})),
            "refreshTrash"=>self.refresh_trash(),
            "chooseBackground"=>self.dialog("folder","Sync-Ordner wählen","",json!({"multiple":false}),Answer::BackgroundFolder),
            "addJob"=>self.activate_background(false),
            "chooseComputerFolder"=>self.dialog("folder","Choose a folder to back up","",json!({}),Answer::ComputerFolder),
            "addComputerFolder"=>self.activate_background(true),
            "start"|"stop"|"restart"=>self.submit_background("Hintergrund-Sync",json!({"action":name})),
            "login" => {
                self.message(&self.tr("Die Anmeldung erfolgt bei Proton im Browser. Diese App ist unabhängig und wird nicht von Proton unterstützt."),false);
                self.submit("Anmelden",json!({"action":"login"}),Done::Refresh)
            },
            "logout" => self.dialog("confirm","Von Proton abmelden?","Hintergrund-Sync wird gestoppt. Native Online-Laufwerke müssen vorher alle geöffneten Dateien schließen. Dies beendet auch die gemeinsam verwendete native CLI-Sitzung.",json!({}),Answer::Logout),
            "cli" => vec![Effect::OpenUrl("https://proton.me/download/drive/cli/index.html".into())],
            "project" => vec![Effect::OpenUrl("https://github.com/SigmaCodez/NeutronDrive".into())],
            "web" => vec![Effect::OpenUrl("https://drive.proton.me".into())],
            "donate" => vec![Effect::OpenUrl("https://ko-fi.com/D1D11J5U3D".into())],
            "subscription" => vec![Effect::OpenUrl("https://account.proton.me/drive/dashboard".into())],
            "refresh" => self.refresh(),
            "Versions" if nodes.len()==1 => self.submit("Version history",json!({"action":"versions","path":paths[0]}),Done::Versions(paths[0].clone())),
            "version" => self.submit("Backend prüfen", json!({"action":"version"}), Done::Version),
            "upload" | "uploadFolder"
                if self.state["fields"]["upload_button"]["enabled"] == true =>
            {
                self.dialog(
                    if name == "uploadFolder" {
                        "folder"
                    } else {
                        "files"
                    },
                    if name == "uploadFolder" {
                        "Ordner hochladen"
                    } else {
                        "Dateien hochladen"
                    },
                    "",
                    json!({"multiple":name=="upload"}),
                    Answer::Upload {
                        parent: self.path().into(),
                    },
                )
            }
            "mkdir" if self.state["fields"]["mkdir_button"]["enabled"] == true => self.dialog(
                "prompt",
                "Neuer Ordner",
                "Name des Ordners",
                json!({"initial":""}),
                Answer::Mkdir {
                    parent: self.path().into(),
                },
            ),
            "Download" => self.dialog(
                "folder",
                "Download-Zielordner wählen",
                "",
                json!({"multiple":false}),
                Answer::Download { paths },
            ),
            "Umbenennen" if nodes.len() == 1 => self.dialog(
                "prompt",
                "Umbenennen",
                "Neuer Name",
                json!({"initial":nodes[0]["name"]}),
                Answer::Rename {
                    path: paths[0].clone(),
                },
            ),
            "Verschieben" => self.dialog(
                "prompt",
                "Verschieben",
                "Cloud-Zielordner (z. B. /my-files/Dokumente)",
                json!({"initial":"/my-files"}),
                Answer::Move { paths },
            ),
            "Papierkorb" => {
                let description = self
                    .tr("{p0} ausgewählte Einträge werden in den Proton-Papierkorb verschoben.")
                    .replace("{p0}", &paths.len().to_string());
                self.dialog(
                    "confirm",
                    "In den Papierkorb verschieben?",
                    &description,
                    json!({"destructive":true}),
                    Answer::Trash { paths },
                )
            }
            "Wiederherstellen" => self.submit(
                "Wiederherstellen",
                json!({"action":"restore","paths":paths}),
                Done::Refresh,
            ),
            "Keep on device" | "Free up space" if nodes.len()==1 => self.mount_action(json!({"action":if name=="Keep on device" {"pin_remote"}else{"evict_remote"},"path":paths[0]})),
            "Freigabe" if nodes.len() == 1 => self.submit(
                "Freigabestatus laden",
                json!({"action":"sharing","path":paths[0]}),
                Done::Share(nodes[0].clone()),
            ),
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_progress_cannot_change_the_current_operation() {
        let mut m = loaded();
        let next = work(m.dispatch("action", &json!(["version"])));
        m.progress(next.id + 1, json!({"total":10,"completed":9}));
        assert!(m.state["progress"].is_null());
        m.progress(
            next.id,
            json!({"total":10,"completed":3,"path":"example.txt"}),
        );
        assert_eq!(m.state["progress"]["completed"], 3);
        m.complete(next.id, Ok(json!("1.0.7")));
        m.progress(next.id, json!({"total":10,"completed":9}));
        assert_eq!(m.state["activity"][0]["progress"]["completed"], 3);
    }
    #[test]
    fn computer_removal_requires_confirmation_and_captures_identity() {
        let mut m = loaded();
        m.state["path"] = json!("/devices");
        m.nodes = vec![
            json!({"safe":true,"uid":"device","raw":{"uid":"device","rootFolderUid":"root"},"name":"PC","path":"/devices/PC","type":"folder"}),
        ];
        m.visible = m.nodes.clone();
        m.selection.insert(0);
        m.update_actions();
        let effects = m.dispatch("remove_computer", &json!([]));
        assert!(matches!(effects.first(), Some(Effect::Dialog(_))));
    }
    #[test]
    fn close_waits_for_confirmed_cancellation_and_rejects_stale_answers() {
        let mut m = model();
        let job = work(m.dispatch("action", &json!(["refresh"])));
        let effects = m.dispatch("request_close", &json!([false]));
        let Effect::Dialog(dialog) = &effects[0] else {
            panic!("expected confirmation")
        };
        let token = dialog["token"].as_u64().unwrap();
        assert!(m
            .dispatch("answer_dialog", &json!([token + 1, true, ""]))
            .is_empty());
        assert!(matches!(
            m.dispatch("answer_dialog", &json!([token, true, ""]))[0],
            Effect::Cancel
        ));
        assert!(m.dispatch("action", &json!(["refresh"])).is_empty());
        assert!(matches!(
            m.complete(
                job.id,
                Err(Failure {
                    message: "cancelled".into(),
                    cancelled: true
                })
            )[0],
            Effect::Close
        ));
    }
    #[test]
    fn sandbox_setup_uses_download_sync_and_mount_actions_do_not_start_workers() {
        for flatpak in [false, true] {
            let mut m = model();
            m.snap = !flatpak;
            m.flatpak = flatpak;
            m.state["demo"] = json!(false);
            m.state["setupFolder"] = json!("/home/test/ProtonDrive");
            assert!(m.mount_action(json!({"action":"mount"})).is_empty());
            assert!(m.mount_action(json!({"action":"pin"})).is_empty());
            let job = work(m.dispatch("setup_finish", &json!([])));
            assert_eq!(job.operation["action"], "setup");
            // Failed startup must leave setup incomplete.
            m.state["setupNeeded"] = json!(true);
            m.complete(job.id, Ok(json!({"mode":"download","running":false})));
            assert_eq!(m.state["setupNeeded"], true);
        }
    }
    #[test]
    fn close_preserves_active_install_and_flatpak_background_service() {
        let mut m = model();
        assert!(m.dispatch("request_close", &json!([true])).is_empty());
        assert!(!m.closing);
        m.flatpak = true;
        let status = work(m.dispatch("request_close", &json!([false])));
        assert_eq!(status.target, Target::Background);
        assert!(matches!(
            m.complete(status.id, Ok(json!({"running": true})))[0],
            Effect::Hide
        ));
        assert!(!m.closing);
        let status = work(m.dispatch("request_close", &json!([false])));
        assert!(matches!(
            m.complete(status.id, Ok(json!({"running": false})))[0],
            Effect::Close
        ));
        assert!(m.closing);
    }
    #[test]
    fn takeout_requires_review_before_upload_and_donation_is_external() {
        let mut m = model();
        m.state["authenticated"] = json!(true);
        m.state["demo"] = json!(false);
        let d = dialog(m.dispatch("photo_library", &json!([{"action":"takeout"}])));
        assert_eq!(d["kind"], "folder");
        let scan = work(m.dispatch(
            "answer_dialog",
            &json!([d["token"], true, ["file:///tmp/Takeout"]]),
        ));
        assert_eq!(scan.operation["action"], "takeout_scan");
        let preview=dialog(m.complete(scan.id,Ok(json!({"root":"/tmp/Takeout","items":[{}],"videos":0,"duplicates":0,"ignored":0,"warnings":0}))));
        assert_eq!(preview["kind"], "confirm");
        assert!(m
            .dispatch("answer_dialog", &json!([preview["token"], false, ""]))
            .is_empty());
        let effects = m.dispatch("action", &json!(["donate"]));
        assert!(matches!(&effects[0],Effect::OpenUrl(url) if url=="https://ko-fi.com/D1D11J5U3D"));
    }
    fn model() -> Files {
        let locale = Localization::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/neutrondrive/locales"),
            "en",
        )
        .unwrap();
        Files::new(locale, "/nonexistent/sdk", false)
    }
    fn work(effects: Vec<Effect>) -> Work {
        match effects
            .into_iter()
            .find(|effect| matches!(effect, Effect::Work(_)))
            .expect("work effect")
        {
            Effect::Work(work) => work,
            _ => panic!("expected work"),
        }
    }
    fn dialog(effects: Vec<Effect>) -> Value {
        match effects.into_iter().next().expect("dialog effect") {
            Effect::Dialog(value) => value,
            _ => panic!("expected dialog"),
        }
    }
    fn nodes() -> Value {
        json!([
            {"name":"Straße.txt","type":"file","path":"/my-files/Straße.txt","safe":true,"size":1536},
            {"name":"z directory","type":"folder","path":"/my-files/z directory","safe":true},
            {"name":"[Ungültiger Name]","type":"file","path":"/my-files/unsafe","safe":false},
            {"name":"alpha.txt","type":"file","path":"/my-files/alpha.txt","safe":true}
        ])
    }
    fn loaded() -> Files {
        let mut m = model();
        let w = work(m.refresh());
        m.complete(w.id, Ok(nodes()));
        m
    }
    #[test]
    fn link_settings_capture_original_share_and_require_confirmation() {
        let mut m = loaded();
        for node in &mut m.nodes {
            node["uid"] = json!(text(node, "path"));
        }
        m.render();
        m.select(0, false);
        let original = m.paths()[0].clone();
        let request = work(m.action("Freigabe"));
        let initial = json!({"urlAccess":{"uid":"link","url":"https://drive.proton.me/urls/test","role":"viewer","customPassword":"original","expirationTime":"2099-01-01T00:00:00.000Z"},"editorsCanShare":false});
        let share = dialog(m.complete(request.id, Ok(initial)));
        assert_eq!(share["linkSettings"]["password"], "original");
        let confirmation = dialog(m.answer(share["token"].as_u64().unwrap(), true, &json!({"action":"settings","role":"editor","password":"new secret","expiration":"","editorsCanShare":true})));
        assert!(!confirmation.to_string().contains("new secret"));
        m.select(2, false);
        let request = work(m.answer(confirmation["token"].as_u64().unwrap(), true, &Value::Null));
        assert_eq!(request.operation["path"], original);
        assert_eq!(
            request.operation["settings"]["expected"]["password"],
            "original"
        );
        assert_eq!(request.operation["settings"]["password"], "new secret");
        assert_eq!(request.operation["settings"]["nodeUid"], original);
        assert_eq!(request.operation["action"], "set_link");
        let reload = work(m.complete(request.id, Ok(json!({}))));
        assert_eq!(reload.operation["action"], "sharing");
    }
    #[test]
    fn sharing_invitation_uses_captured_node_and_requires_confirmation() {
        let mut m = loaded();
        m.select(0, false);
        let original = m.paths()[0].clone();
        let request = work(m.action("Freigabe"));
        let share = dialog(m.complete(request.id, Ok(json!({}))));
        let confirm = dialog(m.answer(
            share["token"].as_u64().unwrap(),
            true,
            &json!({"action":"invite","email":"test@example.invalid","role":"viewer"}),
        ));
        m.select(2, false);
        let request = work(m.answer(confirm["token"].as_u64().unwrap(), true, &json!("")));
        assert_eq!(
            request.operation,
            json!({"action":"invite","path":original,"email":"test@example.invalid","role":"viewer"})
        );
        let reload = work(m.complete(request.id, Ok(json!({"accepted":true}))));
        assert_eq!(
            reload.operation,
            json!({"action":"sharing","path":original})
        );
        let restored = dialog(m.complete(reload.id, Ok(json!({}))));
        assert_eq!(restored["node"]["path"], original);
        let confirm = dialog(m.answer(
            restored["token"].as_u64().unwrap(),
            true,
            &json!({"action":"invite","email":"second@example.invalid","role":"viewer"}),
        ));
        let cancelled = dialog(m.answer(confirm["token"].as_u64().unwrap(), false, &Value::Null));
        assert_eq!(cancelled["kind"], "share");
        assert_eq!(cancelled["node"]["path"], original);
    }
    #[test]
    fn sorting_search_and_selection_preserve_independent_file_model() {
        let mut m = loaded();
        assert_eq!(m.entries[0]["name"], "z directory");
        let revision = m.files_revision;
        m.dispatch("select_row", &json!([0, false]));
        assert_eq!(
            m.files_revision, revision,
            "selection must not recreate delegates"
        );
        m.dispatch("select_row", &json!([1, true]));
        assert_eq!(
            m.state["actions"]["Download"]["enabled"], false,
            "unsafe selection"
        );
        m.dispatch("edit", &json!(["search", "STRASSE"]));
        assert_eq!(
            m.entries.as_array().unwrap().len(),
            1,
            "full Unicode case folding"
        );
        assert_eq!(m.entries[0]["name"], "Straße.txt");
        assert_eq!(m.state["selection"], json!([]));
        m.dispatch("move_selection", &json!([i64::MAX, false]));
        assert_eq!(m.state["selection"], json!([0]));
        assert_eq!(m.state["actions"]["Freigabe"]["enabled"], true);
    }
    #[test]
    fn range_selection_expands_shrinks_and_crosses_fixed_anchor() {
        let mut m = loaded();
        let revision = m.files_revision;
        m.dispatch("select_row", &json!([1, false]));
        m.dispatch("select_range", &json!([3]));
        assert_eq!(m.state["selection"], json!([1, 2, 3]));
        m.dispatch("move_selection", &json!([-1, true]));
        assert_eq!(m.state["selection"], json!([1, 2]));
        m.dispatch("select_range", &json!([0]));
        assert_eq!(m.state["selection"], json!([0, 1]));
        assert_eq!(m.files_revision, revision);
        m.dispatch("select_row", &json!([3, true]));
        assert_eq!(m.state["selection"], json!([0, 1, 3]));
        m.dispatch("select_range", &json!([2]));
        assert_eq!(m.state["selection"], json!([2, 3]));
    }
    #[test]
    fn clearing_selection_keeps_focus_and_disables_file_actions() {
        let mut m = loaded();
        m.dispatch("select_row", &json!([2, false]));
        m.dispatch("clear_selection", &json!([]));
        assert_eq!(m.state["selection"], json!([]));
        assert_eq!(m.state["focusIndex"], 2);
        assert_eq!(m.state["actions"]["Download"]["enabled"], false);
        m.dispatch("move_selection", &json!([-1, true]));
        assert_eq!(m.state["selection"], json!([1, 2]));
    }
    #[test]
    fn filtering_resets_range_anchor_and_invalid_ranges_do_nothing() {
        let mut m = loaded();
        m.dispatch("select_row", &json!([3, false]));
        m.dispatch("edit", &json!(["search", "STRASSE"]));
        m.dispatch("select_range", &json!([0]));
        assert_eq!(m.state["selection"], json!([0]));
        m.dispatch("select_range", &json!([-1]));
        m.dispatch("select_range", &json!([100]));
        assert_eq!(m.state["selection"], json!([0]));
        m.dispatch("edit", &json!(["search", "no-match"]));
        m.dispatch("select_range", &json!([0]));
        assert_eq!(m.state["selection"], json!([]));
    }
    #[test]
    fn large_folder_refined_search_preserves_rows_and_selection() {
        let mut m = model();
        m.nodes = (0..10000).map(|i| json!({"name":format!("Document {i:05}.txt"),"path":format!("/my-files/{i}"),"type":"file","safe":true})).collect();
        m.render();
        m.dispatch("edit", &json!(["search", "doc"]));
        m.select(5000, false);
        let revision = m.files_revision;
        for query in ["docu", "document", "DOCUMENT "] {
            m.dispatch("edit", &json!(["search", query]));
            assert_eq!(m.files_revision, revision);
            assert_eq!(m.state["selection"], json!([5000]));
            assert_eq!(m.entries.as_array().unwrap().len(), 10000);
        }
        m.dispatch("edit", &json!(["search", "Document 05000"]));
        assert_eq!(m.entries.as_array().unwrap().len(), 1);
        assert_eq!(m.state["selection"], json!([]));
    }

    #[test]
    fn different_filters_with_identical_matches_keep_model_and_selection() {
        let mut m = loaded();
        m.dispatch("edit", &json!(["search", "str"]));
        m.select(0, false);
        let revision = m.files_revision;
        m.dispatch("edit", &json!(["search", "strasse"]));
        assert_eq!(m.files_revision, revision);
        assert_eq!(m.state["selection"], json!([0]));
        assert_eq!(m.entries[0]["name"], "Straße.txt");
        let w = work(m.refresh());
        let mut changed = nodes();
        changed
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"other","path":"/my-files/other","type":"file","safe":true}));
        m.complete(w.id, Ok(changed));
        assert_eq!(m.files_revision, revision);
        assert_eq!(m.state["selection"], json!([0]));
        assert!(text(&m.state["fields"]["count_label"], "value").contains("5"));
    }

    #[test]
    fn local_filter_works_during_cloud_read_without_cancelling_it() {
        let mut m = loaded();
        let w = work(m.refresh());
        assert!(m.dispatch("edit", &json!(["search", "STRASSE"])).is_empty());
        assert_eq!(m.entries.as_array().unwrap().len(), 1);
        assert_eq!(m.active.as_ref().unwrap().id, w.id);
        let revision = m.files_revision;
        m.complete(w.id, Ok(nodes()));
        assert_eq!(m.files_revision, revision);
        assert_eq!(m.entries.as_array().unwrap().len(), 1);
        let w = work(m.submit("Upload", json!({"action":"upload"}), Done::Refresh));
        m.dispatch("edit", &json!(["search", ""]));
        assert_eq!(m.state["fields"]["search"]["value"], "STRASSE");
        assert_eq!(m.active.as_ref().unwrap().id, w.id);
    }

    #[test]
    fn equivalent_unicode_search_preserves_selection_and_model() {
        let mut m = loaded();
        m.dispatch("edit", &json!(["search", "Straße"]));
        m.dispatch("select_row", &json!([0, false]));
        let revision = m.files_revision;
        let entries = m.entries.clone();
        m.dispatch("edit", &json!(["search", "STRASSE"]));
        assert_eq!(m.files_revision, revision);
        assert_eq!(m.entries, entries);
        assert_eq!(m.state["selection"], json!([0]));
        assert_eq!(m.state["fields"]["search"]["value"], "STRASSE");
        m.dispatch("edit", &json!(["search", "no matching file"]));
        assert!(m.entries.as_array().unwrap().is_empty());
        assert_eq!(m.state["selection"], json!([]));
        m.dispatch("edit", &json!(["search", ""]));
        assert_eq!(m.entries.as_array().unwrap().len(), m.nodes.len());
    }
    #[test]
    fn navigation_does_not_wait_for_cancel_and_discards_late_results() {
        let mut m = loaded();
        let old = work(m.refresh());
        let photo = work(m.dispatch("open_section", &json!(["Photos", "photos"])));
        assert_eq!(m.state["page"], "photos");
        let shared = work(m.dispatch("open_section", &json!(["Shared", "/shared-with-me"])));
        assert_eq!(shared.operation["path"], "/shared-with-me");
        assert!(m.complete(old.id, Ok(nodes())).is_empty());
        assert!(m
            .complete(photo.id, Ok(json!({"items":[],"total":0})))
            .is_empty());
        assert!(m.nodes.is_empty());
        assert_eq!(m.active.as_ref().unwrap().id, shared.id);
        m.dispatch("show_page", &json!(["settings"]));
        assert!(!m.busy());
        assert_eq!(m.state["page"], "settings");
        m.complete(shared.id, Ok(nodes()));
        assert!(m.nodes.is_empty());
        assert_eq!(m.state["page"], "settings");
    }

    #[test]
    fn cancel_read_immediately_releases_ui_and_ignores_late_failure() {
        let mut m = loaded();
        let old = work(m.refresh());
        m.dispatch("action", &json!(["cancel"]));
        assert!(!m.busy());
        let next = work(m.dispatch("open_section", &json!(["Computer", "/devices"])));
        m.complete(
            old.id,
            Err(Failure {
                message: "late error".into(),
                cancelled: true,
            }),
        );
        assert_ne!(m.state["noticeError"], true);
        assert_eq!(m.active.as_ref().unwrap().id, next.id);
    }

    #[test]
    fn cached_folder_is_immediate_and_expired_folder_stays_visible_during_refresh() {
        let mut m = loaded();
        let expected = m.entries.clone();
        m.dispatch("open_section", &json!(["Activity", "activity"]));
        let effects = m.dispatch("open_section", &json!(["Files", "/my-files"]));
        assert!(!effects.iter().any(|e| matches!(e, Effect::Work(_))));
        assert_eq!(m.entries, expected);
        assert!(!m.busy());
        m.folder_cache.back_mut().unwrap().saved = Instant::now() - Duration::from_secs(31);
        m.dispatch("open_section", &json!(["Activity", "activity"]));
        let refresh = work(m.dispatch("open_section", &json!(["Files", "/my-files"])));
        assert_eq!(m.entries, expected);
        assert_eq!(refresh.operation["path"], "/my-files");
        m.complete(refresh.id, Ok(json!([])));
        assert!(m.entries.as_array().unwrap().is_empty());
    }

    #[test]
    fn cached_folder_can_be_shown_before_another_read_finishes() {
        let mut m = loaded();
        let expected = m.entries.clone();
        let photo = work(m.dispatch("open_section", &json!(["Photos", "photos"])));
        m.dispatch("open_section", &json!(["Files", "/my-files"]));
        assert!(!m.busy());
        assert_eq!(m.entries, expected);
        m.complete(photo.id, Ok(json!({"items":[], "total":0})));
        assert_eq!(m.state["page"], "files");
        assert_eq!(m.entries, expected);
    }

    #[test]
    fn cached_subfolder_can_open_while_parent_refresh_is_stalled() {
        let mut m = loaded();
        let folder = m
            .visible
            .iter()
            .position(|node| node["type"] == "folder" && node["safe"] == true)
            .unwrap();
        let path = m.visible[folder]["path"].clone();
        let old = work(m.refresh());
        assert_eq!(m.state["reading"], true);
        let next = work(m.dispatch("row_activated", &json!([folder])));
        assert_eq!(next.operation["path"], path);
        m.complete(old.id, Ok(json!([])));
        assert_eq!(m.active.as_ref().unwrap().id, next.id);
    }

    #[test]
    fn gallery_return_preserves_inventory_previews_and_album() {
        let mut m = loaded();
        m.state["photoTotal"] = json!(1);
        m.state["photoAlbum"] = json!("album-id");
        m.state["photos"] = json!([{"uid":"photo", "thumbnail":"cached", "previewLoaded":true}]);
        m.photo_index = vec!["photo".into()];
        let revision = m.photos_revision;
        let effects = m.dispatch("open_section", &json!(["Photos", "photos"]));
        assert!(effects.is_empty());
        assert_eq!(m.photos_revision, revision);
        assert_eq!(m.state["photoAlbum"], "album-id");
        assert_eq!(m.state["photos"][0]["thumbnail"], "cached");
        let refresh = work(m.dispatch(
            "photo_library",
            &json!([{"action":"index", "album":"album-id"}]),
        ));
        assert_eq!(refresh.operation["request"]["action"], "index");
    }

    #[test]
    fn logout_resets_navigation_and_photo_pagination() {
        let mut m = loaded();
        m.state["page"] = json!("photos");
        m.state["path"] = json!("/my-files/private");
        m.crumbs = vec![("private".into(), "/my-files/private".into())];
        m.state["photoAlbum"] = json!("old-album");
        m.state["photoOffset"] = json!(96);
        m.state["photosMore"] = json!(true);
        m.state["photoSelection"] = json!([1]);
        m.state["history"] = json!({"visible":true,"path":"/my-files/private","revisions":[{}]});
        let w = work(m.submit("Logout", json!({"action":"logout"}), Done::LoggedOut));
        m.complete(w.id, Ok(json!({})));
        assert_eq!(m.state["page"], "files");
        assert_eq!(m.state["path"], "/my-files");
        assert_eq!(m.crumbs.len(), 1);
        assert_eq!(m.crumbs[0].1, "/my-files");
        assert_eq!(m.state["photoAlbum"], "");
        assert_eq!(m.state["photoOffset"], 0);
        assert_eq!(m.state["photosMore"], false);
        assert_eq!(m.state["photoSelection"], json!([]));
        assert_eq!(m.state["history"], json!({"visible":false,"revisions":[]}));
    }

    #[test]
    fn mutations_and_logout_invalidate_session_cache() {
        let mut m = loaded();
        assert!(!m.folder_cache.is_empty());
        let w = work(m.submit("Upload", json!({"action":"upload"}), Done::Refresh));
        assert!(m.folder_cache.is_empty());
        m.complete(
            w.id,
            Err(Failure {
                message: "failed upload".into(),
                cancelled: false,
            }),
        );
        m.cache_folder();
        let w = work(m.submit("Logout", json!({"action":"logout"}), Done::LoggedOut));
        m.complete(w.id, Ok(json!({})));
        assert!(m.folder_cache.is_empty());
        assert!(m.nodes.is_empty());
    }

    #[test]
    fn folder_cache_is_bounded_and_keeps_recent_folders() {
        let mut m = loaded();
        for i in 0..40 {
            m.state["path"] = json!(format!("/my-files/{i}"));
            m.cache_folder();
        }
        assert_eq!(m.folder_cache.len(), 32);
        assert_eq!(m.folder_cache.front().unwrap().path, "/my-files/8");
        m.state["path"] = json!("/my-files/8");
        assert!(m.restore_folder());
        assert_eq!(m.folder_cache.back().unwrap().path, "/my-files/8");
        m.nodes = vec![json!({"name":"x".repeat(8 * 1024 * 1024)})];
        m.state["path"] = json!("/my-files/oversized");
        m.cache_folder();
        assert!(!m
            .folder_cache
            .iter()
            .any(|entry| entry.path.ends_with("oversized")));
    }

    #[test]
    fn sidebar_navigation_does_not_cancel_a_mutation() {
        let mut m = loaded();
        let transfer = work(m.submit("Upload", json!({"action":"upload"}), Done::Refresh));
        assert!(m
            .dispatch("open_section", &json!(["Photos", "photos"]))
            .is_empty());
        assert_eq!(m.active.as_ref().unwrap().id, transfer.id);
        assert_eq!(m.state["page"], "files");
        assert!(!m.state["fields"]["notice_label"]["value"]
            .as_str()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn retry_clears_old_error_without_erasing_success_information() {
        let mut m = loaded();
        let w = work(m.refresh());
        m.complete(
            w.id,
            Err(Failure {
                message: "Temporary read failure".into(),
                cancelled: false,
            }),
        );
        assert_eq!(m.state["noticeError"], true);
        let retry = work(m.refresh());
        assert_eq!(m.state["noticeError"], false);
        assert_eq!(m.state["fields"]["notice_label"]["visible"], false);
        assert!(text(&m.state["activity"][1], "status").contains("Temporary read failure"));
        m.complete(retry.id, Ok(nodes()));
        assert_eq!(m.state["noticeError"], false);
        m.message("Transfer summary", false);
        let _ = work(m.refresh());
        assert_eq!(
            m.state["fields"]["notice_label"]["value"],
            "Transfer summary"
        );
    }

    #[test]
    fn malformed_lists_preserve_existing_data_and_mark_operation_failed() {
        for done in [Done::Loaded, Done::TrashList] {
            let mut m = loaded();
            m.state["trash"] = json!([{"id":"keep","enabled":true}]);
            let entries = m.entries.clone();
            let trash = m.state["trash"].clone();
            let w = work(m.submit("List", json!({"action":"list"}), done));
            m.complete(w.id, Ok(json!({"unexpected":"response"})));
            assert_eq!(m.entries, entries);
            assert_eq!(m.state["trash"], trash);
            assert_eq!(m.state["noticeError"], true);
            assert_ne!(m.state["activity"][0]["status"], m.tr("Abgeschlossen"));
            assert!(!m.busy());
        }
    }

    #[test]
    fn unchanged_refresh_preserves_selection_focus_and_model_revision() {
        let mut m = loaded();
        m.dispatch("select_row", &json!([2, false]));
        let entries = m.entries.clone();
        let revision = m.files_revision;
        let selected = m.state["selection"].clone();
        let focused = m.state["focusIndex"].clone();
        let w = work(m.refresh());
        m.complete(w.id, Ok(nodes()));
        assert_eq!(m.entries, entries);
        assert_eq!(m.files_revision, revision);
        assert_eq!(m.state["selection"], selected);
        assert_eq!(m.state["focusIndex"], focused);
        assert!(!m.busy());
    }

    #[test]
    fn changed_refresh_still_invalidates_index_selection_and_empty_first_load_renders() {
        let mut m = loaded();
        m.dispatch("select_row", &json!([2, false]));
        let revision = m.files_revision;
        let w = work(m.refresh());
        m.complete(w.id, Ok(json!([])));
        assert!(m.files_revision > revision);
        assert_eq!(m.state["selection"], json!([]));
        let mut m = model();
        let w = work(m.refresh());
        m.complete(w.id, Ok(json!([])));
        assert!(m.files_revision > 0);
        assert!(!text(&m.state["fields"]["count_label"], "value").is_empty());
    }

    #[test]
    fn accepted_dialog_interrupts_read_and_keeps_original_destination() {
        let mut m = loaded();
        let d = dialog(m.dispatch("action", &json!(["mkdir"])));
        let old = work(m.dispatch("navigate", &json!(["Other", "/my-files/Other"])));
        let effects = m.dispatch("answer_dialog", &json!([d["token"], true, "New folder"]));
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, Effect::Cancel)));
        let next = work(effects);
        assert_eq!(
            next.operation,
            json!({"action":"mkdir","parent":"/my-files","name":"New folder"})
        );
        assert!(m.complete(old.id, Ok(json!([]))).is_empty());
        assert_eq!(m.active.as_ref().unwrap().id, next.id);
        assert!(m
            .dispatch("answer_dialog", &json!([d["token"], true, "duplicate"]))
            .is_empty());
    }

    #[test]
    fn rejected_or_stale_dialog_does_not_cancel_a_current_read() {
        for stale in [true, false] {
            let mut m = loaded();
            let d = dialog(m.dispatch("action", &json!(["mkdir"])));
            let old = work(m.refresh());
            let token = d["token"].as_u64().unwrap() + u64::from(stale);
            assert!(m
                .dispatch("answer_dialog", &json!([token, stale, "unused"]))
                .is_empty());
            assert_eq!(m.active.as_ref().unwrap().id, old.id);
        }
    }

    #[test]
    fn dialog_token_and_captured_parent_survive_navigation() {
        let mut m = loaded();
        let d = dialog(m.dispatch("action", &json!(["mkdir"])));
        let w = work(m.dispatch("navigate", &json!(["Other", "/my-files/Other"])));
        m.complete(w.id, Ok(json!([])));
        assert!(m
            .dispatch(
                "answer_dialog",
                &json!([d["token"].as_u64().unwrap() + 1, true, "wrong"])
            )
            .is_empty());
        let w = work(m.dispatch("answer_dialog", &json!([d["token"], true, "new folder"])));
        assert_eq!(
            w.operation,
            json!({"action":"mkdir","parent":"/my-files","name":"new folder"})
        );
        assert!(m
            .dispatch("answer_dialog", &json!([d["token"], true, "duplicate"]))
            .is_empty());
        assert!(m.dispatch("navigate", &json!(["bad", "/trash"])).is_empty());
        assert_eq!(m.state["path"], "/my-files/Other");
    }
    #[test]
    fn cancelled_dialog_and_remote_urls_never_create_transfers() {
        let mut m = loaded();
        let d = dialog(m.dispatch("action", &json!(["upload"])));
        assert!(m
            .dispatch(
                "answer_dialog",
                &json!([d["token"], false, ["file:///tmp/a"]])
            )
            .is_empty());
        let d = dialog(m.dispatch("action", &json!(["upload"])));
        assert!(m
            .dispatch(
                "answer_dialog",
                &json!([d["token"], true, ["https://example.org/a"]])
            )
            .is_empty());
        let d = dialog(m.dispatch("action", &json!(["upload"])));
        let w = work(m.dispatch(
            "answer_dialog",
            &json!([
                d["token"],
                true,
                ["https://example.org/a", "file:///tmp/a%20b"]
            ]),
        ));
        assert_eq!(w.operation["paths"], json!(["/tmp/a b"]));
        assert_eq!(w.operation["file_strategy"], "rename");
    }
    #[test]
    fn clearing_notice_hides_the_banner() {
        let mut m = loaded();
        m.message("An operation failed", true);
        assert_eq!(m.state["fields"]["notice_label"]["visible"], true);
        m.message("", false);
        assert_eq!(m.state["fields"]["notice_label"]["visible"], false);
        assert_eq!(m.state["noticeError"], false);
    }
    #[test]
    fn stale_result_does_not_release_current_busy_operation() {
        let mut m = loaded();
        let w = work(m.refresh());
        m.complete(w.id + 1, Ok(json!([])));
        assert!(m.busy());
        assert_eq!(m.state["fields"]["file_list"]["enabled"], false);
        let old = m.entries.clone();
        m.complete(
            w.id,
            Err(Failure {
                message: "Cancelled".into(),
                cancelled: true,
            }),
        );
        assert!(!m.busy());
        assert_eq!(m.entries, old);
        assert_eq!(m.state["noticeError"], true);
        assert!(text(&m.state["activity"][0], "status").contains("Cancelled"));
        assert_eq!(m.state["fields"]["spinner"]["visible"], false);
    }
    #[test]
    fn trash_only_allows_restore_and_preserves_selected_targets() {
        let mut m = loaded();
        let w = work(m.dispatch("navigate", &json!(["Trash", "/trash"])));
        m.complete(
            w.id,
            Ok(json!([{"name":"a","type":"file","path":"/trash/a","safe":true}])),
        );
        m.dispatch("select_all", &json!([]));
        for action in [
            "Download",
            "Freigabe",
            "Papierkorb",
            "Umbenennen",
            "Verschieben",
            "upload",
            "mkdir",
        ] {
            assert!(
                m.dispatch("action", &json!([action])).is_empty(),
                "{action}"
            );
        }
        let w = work(m.dispatch("action", &json!(["Wiederherstellen"])));
        assert_eq!(
            w.operation,
            json!({"action":"restore","paths":["/trash/a"]})
        );
    }
    #[test]
    fn sharing_revoke_requires_separate_confirmation() {
        let mut m = loaded();
        m.dispatch("select_row", &json!([3, false]));
        let w = work(m.dispatch("action", &json!(["Freigabe"])));
        let d = dialog(m.complete(
            w.id,
            Ok(json!({"urlAccess":{"url":"https://drive.proton.me/urls/example"}})),
        ));
        let copied = m.dispatch("answer_dialog", &json!([d["token"], true, "copy"]));
        assert!(
            matches!(&copied[0],Effect::Clipboard(url) if url=="https://drive.proton.me/urls/example")
        );
        let d = match &copied[1] {
            Effect::Dialog(d) => d.clone(),
            _ => panic!(),
        };
        let confirm = dialog(m.dispatch("answer_dialog", &json!([d["token"], true, "revoke"])));
        assert_eq!(confirm["kind"], "confirm");
        let w = work(m.dispatch("answer_dialog", &json!([confirm["token"], true, ""])));
        assert_eq!(w.operation["action"], "remove_link");
        assert_eq!(w.operation["path"], "/my-files/Straße.txt");
    }
    #[test]
    fn storage_poll_does_not_block_navigation_or_restore_stale_identity() {
        let mut m = model();
        m.state["authenticated"] = json!(true);
        let generation = m.service_generation();
        assert!(matches!(
            m.dispatch("account_storage", &json!([])).as_slice(),
            [Effect::PollService(Target::AccountStorage)]
        ));
        assert!(!m.busy());
        let navigation = work(m.dispatch("navigate", &json!(["/my-files"])));
        assert_eq!(navigation.operation["action"], "list");
        let response =
            json!({"total":100,"used":25,"account":{"name":"stale","email":"stale@example.org"}});
        m.service_polled(Target::AccountStorage, generation, &response);
        assert!(m.state["account"].is_null());
        m.state["authenticated"] = json!(false);
        m.service_polled(Target::AccountStorage, m.service_generation(), &response);
        assert!(m.state["account"].is_null());
    }
    #[test]
    fn account_identity_is_bounded_and_logout_clears_personal_views() {
        let mut m = model();
        m.state["authenticated"] = json!(true);
        let effects = m.dispatch("account_storage", &json!([]));
        assert!(matches!(
            effects.as_slice(),
            [Effect::PollService(Target::AccountStorage)]
        ));
        assert!(!m.busy());
        assert_eq!(m.state["fields"]["spinner"]["visible"], false);
        m.service_polled(
            Target::AccountStorage,
            m.service_generation(),
            &json!({"total":100,"used":25,"account":{"name":"Alex","email":"bad\nvalue"}}),
        );
        assert_eq!(m.state["account"], json!({"name":"Alex","email":""}));
        assert_eq!(m.state["storage"]["percent"], 25.0);
        let effects = m.dispatch("action", &json!(["subscription"]));
        assert!(
            matches!(&effects[0], Effect::OpenUrl(url) if url=="https://account.proton.me/drive/dashboard")
        );
        m.state["photos"] = json!([{"uid":"private"}]);
        let d = dialog(m.dispatch("action", &json!(["logout"])));
        let request = work(m.dispatch("answer_dialog", &json!([d["token"], true, ""])));
        m.complete(request.id, Ok(Value::Null));
        assert!(m.state["account"].is_null());
        assert_eq!(m.state["photos"], json!([]));
    }

    #[test]
    fn login_events_are_operation_scoped_and_logout_requires_confirmation() {
        let mut m = loaded();
        let w = work(m.dispatch("action", &json!(["login"])));
        m.login_ready(w.id + 1, "https://account.proton.me/wrong".into());
        assert_eq!(m.state["fields"]["login_button"]["visible"], false);
        let url = "https://account.proton.me/auth-desktop?synthetic=1";
        m.login_ready(w.id, url.into());
        assert_eq!(m.state["fields"]["login_button"]["visible"], true);
        assert!(!m.state.to_string().contains("synthetic=1"));
        assert!(
            matches!(&m.dispatch("action",&json!(["openLogin"]))[0],Effect::OpenUrl(v) if v==url)
        );
        let refresh = work(m.complete(w.id, Ok(json!({}))));
        assert_eq!(refresh.operation["action"], "list");
        assert_eq!(m.state["fields"]["login_button"]["visible"], false);
        m.login_ready(w.id, url.into());
        assert!(m.dispatch("action", &json!(["openLogin"])).is_empty());
        m.complete(refresh.id, Ok(nodes()));
        let d = dialog(m.dispatch("action", &json!(["logout"])));
        assert_eq!(d["kind"], "confirm");
        assert!(!m.busy());
        assert!(m
            .dispatch("answer_dialog", &json!([d["token"], false, ""]))
            .is_empty());
        let d = dialog(m.dispatch("action", &json!(["logout"])));
        let logout = work(m.dispatch("answer_dialog", &json!([d["token"], true, ""])));
        assert_eq!(logout.done, Done::LoggedOut);
        m.complete(logout.id, Ok(Value::Null));
        assert_eq!(m.entries, json!([]));
    }
    #[test]
    fn background_setup_captures_folder_and_remote_at_confirmation() {
        let mut m = model();
        assert!(m.dispatch("action", &json!(["addJob"])).is_empty());
        assert_eq!(m.state["noticeError"], true);
        let chooser = dialog(m.dispatch("action", &json!(["chooseBackground"])));
        m.dispatch(
            "answer_dialog",
            &json!([chooser["token"], true, ["file:///tmp/first%20folder"]]),
        );
        m.dispatch("edit", &json!(["background_remote", "/my-files/first"]));
        let confirm = dialog(m.dispatch("action", &json!(["addJob"])));
        assert_eq!(confirm["kind"], "confirm");
        assert!(!m.busy());
        m.dispatch("edit", &json!(["background_remote", "/my-files/second"]));
        let work = work(m.dispatch("answer_dialog", &json!([confirm["token"], true, ""])));
        assert_eq!(work.target, Target::Background);
        assert_eq!(
            work.operation,
            json!({"action":"add","local":"/tmp/first folder","remote":"/my-files/first"})
        );
        m.complete(work.id, Ok(json!({"running":true,"jobs":[]})));
        let computer = dialog(m.dispatch("action", &json!(["addComputerFolder"])));
        assert_eq!(computer["kind"], "confirm");
        assert!(m
            .dispatch("answer_dialog", &json!([computer["token"], false, ""]))
            .is_empty());
    }
    #[test]
    fn stale_background_status_cannot_overwrite_a_new_action() {
        let mut m = model();
        assert!(matches!(
            m.dispatch("show_page", &json!(["background"]))[0],
            Effect::PollService(Target::Background)
        ));
        let generation = m.service_generation();
        let w = work(m.dispatch("action", &json!(["start"])));
        m.complete(w.id,Ok(json!({"running":true,"jobs":[{"id":"a","name":"Folder","local_root":"/tmp/local","remote_root":"/my-files","state":"conflict","conflicts":["1","2","3","4","5","6"]}]})));
        let current = m.state["fields"]["background_status"].clone();
        m.background_polled(generation, &json!({"running":false,"jobs":[]}));
        assert_eq!(m.state["fields"]["background_status"], current);
        assert_eq!(m.state["jobs"][0]["detail"], "/tmp/local ↔ /my-files");
        assert!(!m.state["jobs"][0]["status"].as_str().unwrap().contains('6'));
        let generation = m.service_generation();
        m.dispatch("show_page", &json!(["settings"]));
        m.background_polled(generation, &json!({"running":false}));
        // Tray state remains live even while another page is displayed.
        assert_eq!(m.state["backgroundRunning"], false);
        assert_ne!(m.state["fields"]["background_status"], current);
    }
    #[test]
    fn demo_never_creates_background_work() {
        let mut m = model();
        m.state["demo"] = true.into();
        for action in ["addJob", "addComputerFolder", "start", "stop"] {
            assert!(m.dispatch("action", &json!([action])).is_empty());
        }
        assert!(m
            .dispatch("job_action", &json!(["remove", "id"]))
            .is_empty());
        assert!(m.dispatch("show_page", &json!(["background"])).is_empty());
        assert!(!m.busy());
    }
    #[test]
    fn mount_options_and_recovery_status_are_preserved() {
        let mut m = model();
        assert!(matches!(
            m.dispatch("show_page", &json!(["ondemand"]))[0],
            Effect::PollService(Target::Mount)
        ));
        let chooser = dialog(m.dispatch("action", &json!(["chooseMount"])));
        m.dispatch(
            "answer_dialog",
            &json!([chooser["token"], true, ["file:///tmp/my%20mount"]]),
        );
        m.dispatch("edit", &json!(["mount_remote", "/my-files/photos"]));
        m.dispatch("edit", &json!(["mount_offline", true]));
        m.dispatch("edit", &json!(["mount_writable", true]));
        let w = work(m.dispatch("action", &json!(["mount"])));
        assert_eq!(w.target, Target::Mount);
        assert_eq!(
            w.operation,
            json!({"action":"mount","mountpoint":"/tmp/my mount","remote_root":"/my-files/photos","writable":true,"offline":true})
        );
        m.complete(w.id,Ok(json!({"mounted":true,"mountpoint":"/tmp/my mount","pending_recovery":["retained-file"],"configuration":{"mountpoint":"/other"}})));
        assert_eq!(m.state["fields"]["mount_label"]["value"], "/tmp/my mount");
        assert!(m.state["fields"]["mount_status"]["value"]
            .as_str()
            .unwrap()
            .contains("retained-file"));
        m.dispatch("edit", &json!(["cache_path", "folder/file"]));
        let w = work(m.dispatch("action", &json!(["evict"])));
        assert_eq!(w.operation, json!({"action":"evict","path":"folder/file"}));
    }
    #[test]
    fn snap_and_demo_mount_guards_do_not_submit_workers() {
        let mut m = model();
        m.mount_local = Some("/tmp/mount".into());
        m.snap = true;
        assert!(m.dispatch("action", &json!(["mount"])).is_empty());
        assert_eq!(m.state["noticeError"], true);
        m.snap = false;
        m.state["demo"] = true.into();
        for name in [
            "mount",
            "unmount",
            "refreshMount",
            "helperLogin",
            "pin",
            "evict",
        ] {
            assert!(m.dispatch("action", &json!([name])).is_empty());
        }
        assert!(m.dispatch("show_page", &json!(["ondemand"])).is_empty());
    }
    #[test]
    fn trash_restoration_requires_recoverable_receipt_and_refreshes_only_after_success() {
        let mut m = model();
        let listing = work(m.dispatch("show_page", &json!(["local-trash"])));
        assert_eq!(listing.target, Target::Trash);
        let records = json!([{"state":"/private/local-trash","record":{"id":"first","status":"recovery","original_path":"/local/first","trash_path":"/trash/first","restore_path":"/local/recovered","error":"retained"}},
            {"state":"/private/local-trash","record":{"id":"second","status":"restored","original_path":"/local/second"}}]);
        m.complete(listing.id, Ok(records));
        assert_eq!(m.state["trash"][0]["restorePath"], "/local/recovered");
        assert!(m.dispatch("restore_entry", &json!([1])).is_empty());
        let restore = work(m.dispatch("restore_entry", &json!([0])));
        assert_eq!(
            restore.operation,
            json!({"action":"restore","state":"/private/local-trash","id":"first"})
        );
        assert!(m
            .complete(
                restore.id,
                Err(Failure {
                    message: "still in recovery".into(),
                    cancelled: true
                })
            )
            .is_empty());
        assert_eq!(m.state["noticeError"], true);
        assert_eq!(m.state["trash"].as_array().unwrap().len(), 2);
        let restore = work(m.dispatch("restore_entry", &json!([0])));
        let refresh = work(m.complete(restore.id, Ok(json!({"status":"restored"}))));
        assert_eq!(refresh.target, Target::Trash);
        assert_eq!(refresh.operation, json!({"action":"list"}));
    }
    #[test]
    fn tray_close_hides_only_when_available_and_does_not_end_session() {
        let mut m = model();
        let effects = m.dispatch("request_close", &json!([false, true]));
        assert!(matches!(effects.as_slice(), [Effect::Hide]));
        assert!(!m.closing);
        assert!(matches!(
            m.dispatch("request_close", &json!([false, false]))
                .as_slice(),
            [Effect::Close]
        ));
    }

    #[test]
    fn tray_close_preserves_active_work() {
        let mut m = model();
        let job = work(m.dispatch("action", &json!(["refresh"])));
        assert!(m.busy());
        assert!(matches!(
            m.dispatch("request_close", &json!([false, true]))
                .as_slice(),
            [Effect::Hide]
        ));
        assert!(m.busy());
        assert!(!m.closing);
        m.complete(job.id, Ok(json!([])));
        assert!(!m.busy());
        assert!(!m.closing);
    }

    #[test]
    fn photo_download_captures_identity_and_rejects_remote_destination() {
        let mut m = model();
        m.state["photos"] = json!([{"uid":"first","safe":true}]);
        let d = dialog(m.dispatch("photo_download", &json!([0])));
        m.state["photos"] = json!([{"uid":"second","safe":true}]);
        let w = work(m.dispatch(
            "answer_dialog",
            &json!([d["token"], true, ["file:///tmp/photo%20destination"]]),
        ));
        assert_eq!(
            w.operation,
            json!({"action":"download_photo","uid":"first","destination":"/tmp/photo destination"})
        );
        m.complete(w.id, Ok(json!({})));
        let d = dialog(m.dispatch("photo_download", &json!([0])));
        assert!(m
            .dispatch(
                "answer_dialog",
                &json!([d["token"], true, ["https://example.org/path"]])
            )
            .is_empty());
    }
    #[test]
    fn restore_dialog_captures_file_revision_and_baseline() {
        let mut m = model();
        m.state["history"] = json!({"visible":true,"path":"/my-files/one","active":"current","revisions":[{"uid":"old","state":"superseded"},{"uid":"current","state":"active"}]});
        assert!(m.dispatch("version_restore", &json!([1])).is_empty());
        let d = dialog(m.dispatch("version_restore", &json!([0])));
        m.state["history"] = json!({"path":"/my-files/two","active":"other"});
        let w = work(m.dispatch("answer_dialog", &json!([d["token"], true, ""])));
        assert_eq!(
            w.operation,
            json!({"action":"restore_version","path":"/my-files/one","revision":"old","expected":"current"})
        );
    }
    #[test]
    fn background_summary_tracks_attention_without_switching_page() {
        let mut m = model();
        m.background_polled(m.next_work,&json!({"running":true,"jobs":[{"state":"syncing"},{"state":"paused","paused":true},{"state":"conflict"},{"state":"offline"}]}));
        assert_eq!(
            m.state["syncSummary"],
            json!({"total":4,"syncing":1,"paused":1,"attention":2})
        );
        assert_eq!(m.state["backgroundRunning"], true);
    }
}

#[path = "photo_library.rs"]
mod photo_library;
