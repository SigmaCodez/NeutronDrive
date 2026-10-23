//! Native background service: shared state, scheduling and owned worker threads.
use crate::{
    control_server, daemon_control, daemon_job, event_ancestry, event_runtime, filesystem,
    host::Services,
    job_roots, jobs, metadata_cache, policy,
    process::{self, Failure},
    storage, sync_execute, sync_plan, sync_remote, watcher,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, MutexGuard,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
type Result<T> = std::result::Result<T, Failure>;
fn fail(message: &str) -> Failure {
    Failure {
        message: message.into(),
        cancelled: true,
    }
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn policy_call(mut value: Value) -> Result<Value> {
    value["version"] = json!(1);
    policy::dispatch(&serde_json::to_vec(&value).map_err(|e| e.to_string())?).map_err(Into::into)
}
fn clock() -> f64 {
    let mut t: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut t);
    }
    t.tv_sec as f64 + t.tv_nsec as f64 / 1e9
}
fn wall() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn jitter() -> f64 {
    use std::io::Read;
    let mut bytes = [0u8; 8];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_ok()
    {
        (u64::from_ne_bytes(bytes) >> 11) as f64 / ((1u64 << 53) as f64)
    } else {
        0.5
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub state: PathBuf,
    pub executable: String,
    #[serde(default)]
    pub mount_paths: Vec<String>,
}
struct Job {
    config: Value,
    runtime: Value,
    watcher: Option<u64>,
    token: u64,
}
fn initial() -> Value {
    json!({"state":"pending","pending":true,"due":0.0,"first_dirty":0.0,"paths":[],"reasons":["startup"],"known_uids":[],"failures":0,"last_sync":null,"error":"","conflicts":[],"skipped":[],"completed":0,"rebuild_watch":false,"local_snapshot":{},"snapshot_ready":false})
}
struct State {
    jobs: BTreeMap<String, Job>,
    order: Vec<String>,
    current: Option<String>,
    events: Vec<Value>,
    event_status: Value,
    running: bool,
}
struct Shared {
    config: Config,
    state: Mutex<State>,
    wake: Condvar,
    stopped: AtomicBool,
    cancel: AtomicBool,
    sdk: Mutex<()>,
    cache: u64,
    threads: Mutex<Vec<JoinHandle<()>>>,
}
struct NoHost;
impl Services for NoHost {
    fn poll(&mut self) -> Result<()> {
        Ok(())
    }
    fn request(&mut self, _: &str, _: Value) -> Result<Value> {
        Err("Unexpected daemon service".into())
    }
}
impl Shared {
    fn lock(&self) -> Result<MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| "Daemon state unavailable".into())
    }
    fn jobs(&self) -> Result<Vec<Value>> {
        jobs::transact(
            &self.config.state,
            &self.config.mount_paths,
            jobs::Action::List,
            &mut |_| 0,
        )?
        .as_array()
        .cloned()
        .ok_or_else(|| "Invalid jobs".into())
    }
    fn cache(&self, mut args: Value) -> Result<Value> {
        args["id"] = json!(self.cache);
        metadata_cache::run(
            serde_json::from_value(args).map_err(|e| e.to_string())?,
            &mut NoHost,
        )
    }
    fn status(&self, state: &State) -> Result<Value> {
        let now = clock();
        let timestamp = wall();
        let mut jobs = vec![];
        for job in state.order.iter().filter_map(|id| state.jobs.get(id)) {
            let mut value = job.config.clone();
            for key in [
                "state",
                "pending",
                "last_sync",
                "error",
                "completed",
                "reasons",
            ] {
                value[key] = job.runtime[key].clone();
            }
            for key in ["conflicts", "skipped"] {
                value[key] = json!(job.runtime[key]
                    .as_array()
                    .ok_or("Invalid status list")?
                    .iter()
                    .take(1000)
                    .collect::<Vec<_>>());
            }
            value["retry_at"] = if job.runtime["state"] == "offline" {
                json!(timestamp + (job.runtime["due"].as_f64().unwrap_or(now) - now).max(0.0))
            } else {
                Value::Null
            };
            jobs.push(value);
        }
        Ok(
            json!({"version":1,"running":state.running&&!self.stopped.load(Ordering::Acquire),"pid":std::process::id(),"updated_at":timestamp,"event_source":state.event_status,"rust_core":policy_call(json!({"op":"info"}))?,"unresolved_events":state.events,"jobs":jobs}),
        )
    }
    fn persist(&self, state: &State) -> Result<()> {
        storage::write(&self.config.state.join("status.json"), self.status(state)?)
    }
    fn schedule(
        &self,
        job: &mut Job,
        reason: &str,
        relative: Option<String>,
        immediate: bool,
    ) -> Result<()> {
        let mut state = json!({"paused":job.config["paused"]});
        for key in [
            "state",
            "pending",
            "due",
            "first_dirty",
            "paths",
            "reasons",
            "rebuild_watch",
        ] {
            state[key] = job.runtime[key].clone();
        }
        let update = policy_call(
            json!({"op":"schedule","state":state,"reason":reason,"relative":relative,"immediate":immediate,"now":clock(),"debounce":1.5}),
        )?;
        for (key, value) in update.as_object().ok_or("Invalid schedule")? {
            if key != "paused" {
                job.runtime[key] = value.clone();
            }
        }
        self.wake.notify_all();
        Ok(())
    }
    fn transition(&self, job: &mut Job, outcome: Value) -> Result<()> {
        let runtime = serde_json::from_value(job.runtime.clone()).map_err(|e| e.to_string())?;
        let outcome = serde_json::from_value(outcome).map_err(|e| e.to_string())?;
        let context=serde_json::from_value(json!({"paused":job.config["paused"],"stopping":self.stopped.load(Ordering::Acquire),"root":job.config["local_root"],"now":clock(),"wall":wall(),"debounce":1.5,"retry_base":30.0,"retry_max":900.0,"jitter":0.9 + 0.2 * jitter()})).map_err(|e|e.to_string())?;
        job.runtime = daemon_job::transition(runtime, outcome, context)?;
        Ok(())
    }
    fn stop_watch(job: &mut Job) -> Result<()> {
        job.token = job.token.wrapping_add(1);
        if let Some(id) = job.watcher.take() {
            watcher::stop(id)?;
        }
        Ok(())
    }
    fn watch(self: &Arc<Self>, id: &str, job: &mut Job) -> Result<()> {
        Self::stop_watch(job)?;
        let root = Path::new(text(&job.config, "local_root"));
        let identity = jobs::local_identity(
            root,
            &self.config.state,
            &self.config.mount_paths,
            &jobs::mounts()?,
        )?;
        if Some(identity.0) != job.config["local_device"].as_u64()
            || Some(identity.1) != job.config["local_inode"].as_u64()
        {
            return Err("Der lokale Sync-Ordner wurde ersetzt; Job bewusst neu anlegen.".into());
        }
        use std::os::unix::ffi::OsStrExt;
        let watch = watcher::create(root.as_os_str().as_bytes().to_vec())?;
        job.watcher = Some(watch);
        job.runtime["rebuild_watch"] = json!(false);
        let shared = Arc::clone(self);
        let id = id.to_owned();
        let token = job.token;
        let task = thread::spawn(move || {
            let result = watcher::run(watch, &mut |event| {
                if shared.stopped.load(Ordering::Acquire) {
                    return 1;
                }
                if event["kind"] == "watch" {
                    let relative = event["relative"]
                        .as_array()
                        .and_then(|v| {
                            v.iter()
                                .map(|b| b.as_u64().and_then(|b| u8::try_from(b).ok()))
                                .collect::<Option<Vec<_>>>()
                        })
                        .and_then(|b| String::from_utf8(b).ok());
                    let _ = shared.local_event(&id, token, relative, text(&event, "reason"));
                }
                0
            });
            if result.is_err() && !shared.stopped.load(Ordering::Acquire) {
                let _ = shared.local_event(&id, token, None, "watch-error");
            }
            let _ = watcher::stop(watch);
        });
        let mut tasks = self
            .threads
            .lock()
            .map_err(|_| "Watcher thread registry unavailable")?;
        let mut index = 0;
        while index < tasks.len() {
            if tasks[index].is_finished() {
                let _ = tasks.swap_remove(index).join();
            } else {
                index += 1;
            }
        }
        tasks.push(task);
        Ok(())
    }
    fn local_event(
        &self,
        id: &str,
        token: u64,
        relative: Option<String>,
        reason: &str,
    ) -> Result<()> {
        let mut state = self.lock()?;
        if self.stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Some(job) = state.jobs.get_mut(id) {
            if token != job.token {
                return Ok(());
            }
            if let Some(path) = &relative {
                if job.runtime["state"] != "syncing"
                    && job.runtime["snapshot_ready"] == true
                    && filesystem::matches_snapshot(
                        Path::new(text(&job.config, "local_root")),
                        path,
                        job.runtime["local_snapshot"].clone(),
                    )
                {
                    return Ok(());
                }
            }
            self.schedule(job, reason, relative, false)?;
        }
        Ok(())
    }
    fn command(
        self: &Arc<Self>,
        action: daemon_control::Action,
        id: Option<String>,
    ) -> Result<Value> {
        let mut state = self.lock()?;
        daemon_control::command(
            action,
            id,
            &mut StateHost {
                shared: self,
                state: &mut state,
            },
        )
    }
    fn event_jobs(state: &State) -> Vec<Value> {
        state.jobs.iter().map(|(id,j)|json!({"id":id,"remote_uid":j.config["remote_uid"],"account_root_uid":j.config["account_root_uid"],"known_uids":j.runtime["known_uids"],"paused":j.config["paused"]})).collect()
    }
    fn remote_event(&self, event: Value) -> Result<()> {
        let mut state = self.lock()?;
        if self.stopped.load(Ordering::Acquire) {
            return Err(fail("Dienst wird beendet; Event-Cursor nicht bestätigen."));
        }
        let identity = json!({"node_uid":event["node_uid"],"parent_uid":event["parent_uid"],"root_uid":event["root_uid"]});
        let decision = policy_call(
            json!({"op":"route_event","event":{"kind":event["kind"],"reset":event["reset"].as_bool().unwrap_or(false),"identity":identity},"jobs":Self::event_jobs(&state),"pending":state.events}),
        )?;
        if decision["offline"] == true {
            state.event_status = json!({"state":"offline","error":"Remote-Eventverbindung unterbrochen; Wiederverbindung läuft."});
        } else {
            state.event_status = json!({"state":"connected","error":""});
            let mut roots = vec![];
            for id in decision["affected"]
                .as_array()
                .ok_or("Invalid event routing")?
            {
                if let Some(job) = state.jobs.get_mut(id.as_str().ok_or("Invalid event job")?) {
                    roots.push(job.config["remote_root"].clone());
                    self.schedule(job, text(&decision, "reason"), None, false)?;
                }
            }
            if !roots.is_empty() {
                self.cache(json!({"action":"event","roots":roots,"node_uid":identity["node_uid"],"parent_uid":identity["parent_uid"],"reset":event["reset"].as_bool().unwrap_or(false)}))?;
            }
            if !decision["append"].is_null() {
                state.events.push(decision["append"].clone());
            }
            for id in decision["overflow"]
                .as_array()
                .ok_or("Invalid event overflow")?
            {
                if let Some(job) = state.jobs.get_mut(id.as_str().ok_or("Invalid event job")?) {
                    self.cache(json!({"action":"event","roots":[job.config["remote_root"]],"node_uid":null,"parent_uid":null,"reset":true}))?;
                    self.schedule(job, "remote-overflow", None, false)?;
                }
            }
        }
        self.persist(&state)?;
        self.wake.notify_all();
        Ok(())
    }
}
struct StateHost<'a> {
    shared: &'a Arc<Shared>,
    state: &'a mut State,
}
impl Services for StateHost<'_> {
    fn poll(&mut self) -> Result<()> {
        Ok(())
    }
    fn request(&mut self, method: &str, args: Value) -> Result<Value> {
        if method != "daemon_state" {
            return Err("Unsupported daemon state service".into());
        }
        let id = text(&args, "id");
        match text(&args, "action") {
            "jobs" => return Ok(json!(self.shared.jobs()?)),
            "snapshot" => {
                return Ok(
                    json!({"current":self.state.current,"jobs":self.state.jobs.iter().map(|(id,j)|(id.clone(),json!({"job":j.config,"watcher":j.watcher.is_some()}))).collect::<BTreeMap<_,_>>()}),
                )
            }
            "status" => return self.shared.status(self.state),
            "persist" => self.shared.persist(self.state)?,
            "notify" => self.shared.wake.notify_all(),
            "stop" => self.shared.stopped.store(true, Ordering::Release),
            "cancel" => self.shared.cancel.store(true, Ordering::Release),
            "remove" => {
                self.state.jobs.remove(id);
                self.state.order.retain(|key| key != id);
            }
            "update_job" => {
                jobs::transact(
                    &self.shared.config.state,
                    &self.shared.config.mount_paths,
                    jobs::Action::Update {
                        id: id.into(),
                        paused: args["paused"].as_bool(),
                        name: None,
                    },
                    &mut |_| 0,
                )?;
            }
            "upsert" => {
                let config = args["job"].clone();
                let id = text(&config, "id").to_owned();
                if !self.state.jobs.contains_key(&id) {
                    self.state.order.push(id.clone());
                }
                self.state
                    .jobs
                    .entry(id)
                    .or_insert_with(|| Job {
                        config: config.clone(),
                        runtime: initial(),
                        watcher: None,
                        token: 0,
                    })
                    .config = config.clone();
            }
            "patch" => {
                let job = self.state.jobs.get_mut(id).ok_or("Unknown runtime job")?;
                for (key, value) in args["fields"].as_object().ok_or("Invalid runtime patch")? {
                    if key != "watch_token" {
                        job.runtime[key] = value.clone();
                    }
                }
            }
            "stop_watcher" => {
                Shared::stop_watch(self.state.jobs.get_mut(id).ok_or("Unknown runtime job")?)?
            }
            "watch" => self.shared.watch(
                id,
                self.state.jobs.get_mut(id).ok_or("Unknown runtime job")?,
            )?,
            "invalidate" => {
                self.shared.cache(json!({"action":"event","roots":[args["root"]],"node_uid":null,"parent_uid":null,"reset":true}))?;
            }
            "schedule" => self.shared.schedule(
                self.state.jobs.get_mut(id).ok_or("Unknown runtime job")?,
                text(&args, "reason"),
                None,
                true,
            )?,
            _ => return Err("Unknown daemon state operation".into()),
        }
        Ok(Value::Null)
    }
}
struct Sdk<'a> {
    shared: &'a Shared,
    event: bool,
}
impl Sdk<'_> {
    fn cancelled(&self) -> bool {
        self.shared.stopped.load(Ordering::Acquire)
            || (!self.event && self.shared.cancel.load(Ordering::Acquire))
    }
    fn lock(&self) -> Result<MutexGuard<'_, ()>> {
        loop {
            self.poll_cancel()?;
            match self.shared.sdk.try_lock() {
                Ok(lock) => return Ok(lock),
                Err(std::sync::TryLockError::WouldBlock) => {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(_) => return Err("SDK serialization unavailable".into()),
            }
        }
    }
    fn poll_cancel(&self) -> Result<()> {
        if self.cancelled() {
            Err(fail("Sync-Job wurde angehalten."))
        } else {
            Ok(())
        }
    }
}
impl Services for Sdk<'_> {
    fn poll(&mut self) -> Result<()> {
        self.poll_cancel()
    }
    fn request(&mut self, method: &str, args: Value) -> Result<Value> {
        let _lock = self.lock()?;
        let mut callback = |_: Value| i32::from(self.cancelled());
        sync_remote::Sdk {
            executable: self.shared.config.executable.clone(),
            callback: &mut callback,
        }
        .request(method, args)
    }
}
struct Cached<'a>(&'a Shared);
impl Services for Cached<'_> {
    fn poll(&mut self) -> Result<()> {
        Sdk {
            shared: self.0,
            event: false,
        }
        .poll()
    }
    fn request(&mut self, method: &str, mut args: Value) -> Result<Value> {
        if matches!(method, "try_info" | "download") {
            return Sdk {
                shared: self.0,
                event: false,
            }
            .request(method, args);
        }
        args["action"] = json!(match method {
            "list_nodes" | "list_nodes_fresh" => "list",
            other => other,
        });
        args["id"] = json!(self.0.cache);
        if matches!(method, "list_nodes" | "list_nodes_fresh") {
            args["fresh"] = json!(method == "list_nodes_fresh");
        }
        metadata_cache::run(
            serde_json::from_value(args).map_err(|e| e.to_string())?,
            &mut Sdk {
                shared: self.0,
                event: false,
            },
        )
    }
}
enum JobError {
    Blocked(String),
    Failed(Failure),
}
impl From<Failure> for JobError {
    fn from(value: Failure) -> Self {
        Self::Failed(value)
    }
}
impl Shared {
    fn execute(&self, id: &str) -> std::result::Result<(Value, Value), JobError> {
        let job = self
            .lock()?
            .jobs
            .get(id)
            .ok_or_else(|| fail("Sync-Job wurde entfernt."))?
            .config
            .clone();
        let local = jobs::local_identity(
            Path::new(text(&job, "local_root")),
            &self.config.state,
            &self.config.mount_paths,
            &jobs::mounts()?,
        )
        .map_err(|e| JobError::Blocked(e.message))?;
        if Some(local.0) != job["local_device"].as_u64()
            || Some(local.1) != job["local_inode"].as_u64()
        {
            return Err(JobError::Blocked(
                "Lokaler Sync-Ordner wurde ersetzt; Job bewusst neu anlegen.".into(),
            ));
        }
        let roots = job_roots::resolve(
            self.jobs()?,
            id,
            &mut Sdk {
                shared: self,
                event: false,
            },
        )?;
        if let Some(error) = roots["blocked"].as_str() {
            return Err(JobError::Blocked(error.into()));
        }
        if roots["cancelled"] == true {
            return Err(fail("Sync-Job wurde pausiert oder entfernt.").into());
        }
        let mut selected = None;
        for job in roots["jobs"]
            .as_array()
            .ok_or_else(|| Failure::from("Invalid root proof"))?
        {
            let pinned = jobs::transact(
                &self.config.state,
                &self.config.mount_paths,
                jobs::Action::Pin {
                    id: text(job, "id").into(),
                    uid: text(job, "remote_uid").into(),
                    account_uid: text(job, "account_root_uid").into(),
                    remote_path: text(job, "remote_root").into(),
                },
                &mut |_| i32::from(self.cancel.load(Ordering::Acquire)),
            )?;
            if text(job, "id") == id {
                selected = Some(pinned.clone());
            }
            if let Some(runtime) = self.lock()?.jobs.get_mut(text(job, "id")) {
                runtime.config = pinned;
            }
        }
        let job = selected.ok_or_else(|| fail("Sync-Job wurde entfernt."))?;
        let sync_state = self.config.state.join("sync");
        let state = sync_state
            .to_str()
            .ok_or_else(|| Failure::from("Invalid sync state path"))?;
        let plan = sync_plan::run(
            state,
            text(&job, "local_root"),
            text(&job, "remote_root"),
            &mut Cached(self),
        )?;
        if plan["remote_root_uid"] != job["remote_uid"] {
            return Err(JobError::Blocked(
                "Remote-Wurzel wurde während der Vorbereitung geändert.".into(),
            ));
        }
        {
            let mut state = self.lock()?;
            let current = self.jobs()?.into_iter().find(|j| j["id"] == id);
            if self.stopped.load(Ordering::Acquire)
                || current.as_ref().is_none_or(|j| j["paused"] == true)
            {
                return Err(fail("Sync-Job wurde angehalten.").into());
            }
            let mut known: BTreeSet<String> = plan["_remote"]
                .as_object()
                .ok_or_else(|| Failure::from("Invalid sync preview"))?
                .values()
                .map(|e| text(e, "uid").into())
                .collect();
            known.insert(text(&job, "remote_uid").into());
            if let Some(runtime) = state.jobs.get_mut(id) {
                runtime.runtime["known_uids"] = json!(known);
            }
        }
        let previous = plan["_local"].clone();
        let data = process::data_dir()?;
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .filter(|p| Path::new(p).is_absolute())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
            });
        let result = sync_execute::run(
            sync_execute::Request {
                state: state.into(),
                plan,
                trash_state: data.join("local-trash").to_string_lossy().into_owned(),
                trash_data: data_home.to_string_lossy().into_owned(),
                trash_desktop: true,
            },
            &mut Cached(self),
        )?;
        Ok((previous, result))
    }
    fn run_job(self: &Arc<Self>, id: &str) -> Result<()> {
        let outcome = self.execute(id);
        let mut state = self.lock()?;
        if let Some(job) = state.jobs.get_mut(id) {
            let action = match outcome {
                Ok((previous, result)) => {
                    json!({"action":"complete","previous":previous,"result":result})
                }
                Err(JobError::Blocked(error)) => json!({"action":"blocked","error":error}),
                Err(JobError::Failed(error)) if error.cancelled => json!({"action":"cancelled"}),
                Err(JobError::Failed(error)) => json!({"action":"retry","error":error.message}),
            };
            self.transition(job, action)?;
            if job.runtime["rebuild_watch"] == true && job.config["paused"] != true {
                if let Err(error) = self.watch(id, job) {
                    self.transition(job, json!({"action":"retry","error":error.message}))?;
                }
            }
            self.transition(job, json!({"action":"finalize"}))?;
        }
        state.current = None;
        self.persist(&state)?;
        self.wake.notify_all();
        Ok(())
    }
    fn resolve_events(&self, batch: Vec<Value>) -> Result<()> {
        for event in batch {
            let jobs = Self::event_jobs(&*self.lock()?);
            let decision = event_ancestry::resolve(
                serde_json::from_value(event.clone()).map_err(|e| e.to_string())?,
                serde_json::from_value(json!(jobs)).map_err(|e| e.to_string())?,
                &mut Sdk {
                    shared: self,
                    event: false,
                },
            )?;
            if decision["cancelled"] == true && self.stopped.load(Ordering::Acquire) {
                return Ok(());
            }
            let mut state = self.lock()?;
            let mut roots = vec![];
            for id in decision["affected"]
                .as_array()
                .ok_or("Invalid event resolution")?
            {
                if let Some(job) = state.jobs.get_mut(id.as_str().ok_or("Invalid job")?) {
                    if job.config["paused"] != true {
                        roots.push(job.config["remote_root"].clone());
                        self.schedule(job, "remote-ancestry", None, false)?;
                    }
                }
            }
            if !roots.is_empty() {
                self.cache(json!({"action":"event","roots":roots,"node_uid":event["node_uid"],"parent_uid":decision["ancestor"],"reset":decision["ancestor"].is_null()}))?;
            }
            if let Some(index) = state.events.iter().position(|e| e == &event) {
                state.events.remove(index);
            }
            self.persist(&state)?;
            self.wake.notify_all();
        }
        Ok(())
    }
    fn wait(&self, duration: Duration) -> Result<()> {
        let deadline = Instant::now() + duration;
        let mut state = self.lock()?;
        while !self.stopped.load(Ordering::Acquire) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            state = self
                .wake
                .wait_timeout(state, left)
                .map_err(|_| "Daemon wake unavailable")?
                .0;
        }
        Ok(())
    }
    fn events(&self) -> Result<()> {
        let mut paths = vec![];
        let mut file = PathBuf::new();
        let mut cursors = json!({});
        let mut known = vec![];
        let mut announced = false;
        let mut ready = false;
        let mut failures = 0;
        while !self.stopped.load(Ordering::Acquire) {
            let attempt = (|| -> Result<()> {
                let mut roots = BTreeSet::from(["/my-files".to_string()]);
                for job in self.jobs()? {
                    if job["paused"] != true {
                        roots.insert(crate::computer::boundary(text(&job, "remote_root"))?);
                    }
                }
                let roots: Vec<_> = roots.into_iter().collect();
                if roots != paths {
                    let new_file =
                        crate::event_source::state_file(&self.config.state.join("events"), &roots);
                    let loaded = event_runtime::load(&new_file, &roots)?;
                    paths = roots;
                    file = new_file;
                    cursors = loaded["cursors"].clone();
                    known = loaded["roots"]
                        .as_array()
                        .ok_or("Invalid cursor roots")?
                        .clone();
                    announced = false;
                }
                let sdk = Sdk {
                    shared: self,
                    event: true,
                };
                let batch = {
                    let _lock = sdk.lock()?;
                    event_runtime::query(
                        self.config.executable.clone(),
                        &paths,
                        cursors.clone(),
                        &mut |_| i32::from(sdk.cancelled()),
                    )?
                };
                let mut delivery_error = None;
                let batch = event_runtime::deliver(
                    event_runtime::Delivery {
                        file: file.to_string_lossy().into_owned(),
                        paths: paths.clone(),
                        known_roots: known.clone(),
                        announced,
                        ready,
                    },
                    batch,
                    &mut |event| {
                        if self.stopped.load(Ordering::Acquire) {
                            return 1;
                        }
                        if event["kind"] == "remote_event" {
                            if let Err(error) = self.remote_event(event["event"].clone()) {
                                delivery_error = Some(error);
                                return 1;
                            }
                        }
                        0
                    },
                );
                if let Some(error) = delivery_error {
                    return Err(error);
                }
                let batch = batch?;
                cursors = batch["cursors"].clone();
                known = batch["roots"]
                    .as_array()
                    .ok_or("Invalid cursor roots")?
                    .clone();
                announced = true;
                ready = true;
                Ok(())
            })();
            let delay = match attempt {
                Ok(()) => {
                    failures = 0;
                    30.0
                }
                Err(error) => {
                    if self.stopped.load(Ordering::Acquire) {
                        break;
                    }
                    if error.cancelled
                        || error.message.contains("andere NeutronDrive-Instanz")
                        || error.message.contains("Backend ist noch beschäftigt")
                    {
                        30.0
                    } else {
                        ready = false;
                        let retry = policy_call(
                            json!({"op":"event_retry","interval":30.0,"failures":failures,"jitter":jitter()}),
                        )?;
                        failures = retry["failures"].as_u64().unwrap_or(failures + 1);
                        let _=self.remote_event(json!({"kind":"error","reset":false,"node_uid":null,"parent_uid":null,"root_uid":null}));
                        retry["delay"].as_f64().unwrap_or(30.0)
                    }
                }
            };
            self.wait(Duration::from_secs_f64(delay))?;
        }
        Ok(())
    }
    fn wait_task(
        &self,
        task: JoinHandle<Result<()>>,
        callback: &mut impl FnMut(Value) -> i32,
    ) -> Result<()> {
        while !task.is_finished() {
            if callback(json!({"kind":"poll"})) != 0 {
                self.stopped.store(true, Ordering::Release);
                self.cancel.store(true, Ordering::Release);
                self.wake.notify_all();
            }
            thread::sleep(Duration::from_millis(25));
        }
        task.join().map_err(|_| "Native daemon worker panicked")?
    }
    fn main_loop(self: &Arc<Self>, callback: &mut impl FnMut(Value) -> i32) -> Result<()> {
        loop {
            if callback(json!({"kind":"poll"})) != 0 {
                self.command(daemon_control::Action::Stop, None)?;
            }
            let mut state = self.lock()?;
            if self.stopped.load(Ordering::Acquire) {
                break;
            }
            if !state.events.is_empty() {
                let batch = state.events.iter().take(64).cloned().collect();
                self.cancel.store(false, Ordering::Release);
                drop(state);
                let worker = Arc::clone(self);
                self.wait_task(
                    thread::spawn(move || worker.resolve_events(batch)),
                    callback,
                )?;
                continue;
            }
            let candidates:Vec<_>=state.order.iter().filter_map(|id| state.jobs.get(id).map(|job| (id,job))).map(|(id,j)|json!({"id":id,"pending":j.runtime["pending"],"paused":j.config["paused"],"state":j.runtime["state"],"due":j.runtime["due"]})).collect();
            let decision = policy_call(json!({"op":"select","jobs":candidates,"now":clock()}))?;
            if decision.is_null() || decision["delay"].as_f64().unwrap_or(0.0) > 0.0 {
                let delay = decision["delay"].as_f64().unwrap_or(0.25).min(0.25);
                drop(
                    self.wake
                        .wait_timeout(state, Duration::from_secs_f64(delay))
                        .map_err(|_| "Daemon wait unavailable")?,
                );
                continue;
            }
            let id = text(&decision, "id").to_owned();
            state.current = Some(id.clone());
            self.transition(
                state.jobs.get_mut(&id).ok_or("Selected job disappeared")?,
                json!({"action":"begin"}),
            )?;
            self.cancel.store(false, Ordering::Release);
            self.persist(&state)?;
            drop(state);
            let worker = Arc::clone(self);
            let task = thread::spawn(move || worker.run_job(&id));
            self.wait_task(task, callback)?;
        }
        Ok(())
    }
}
struct Control(Arc<Shared>);
impl Services for Control {
    fn poll(&mut self) -> Result<()> {
        Ok(())
    }
    fn request(&mut self, method: &str, args: Value) -> Result<Value> {
        if method != "daemon_command" {
            return Err("Invalid daemon service".into());
        }
        let action = serde_json::from_value(args["action"].clone())
            .map_err(|_| "Unbekannter Dienstbefehl.")?;
        self.0
            .command(action, args["job_id"].as_str().map(String::from))
    }
}
pub fn run(config: Config, callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    storage::directory(&config.state, true, true)?;
    let server = control_server::run(
        control_server::Operation::Create {
            state: config.state.clone(),
        },
        &mut NoHost,
    )?
    .as_u64()
    .ok_or("Invalid server handle")?;
    let cache = match metadata_cache::run(metadata_cache::Operation::Create, &mut NoHost) {
        Ok(value) => value.as_u64().ok_or("Invalid metadata cache")?,
        Err(error) => {
            control_server::run(
                control_server::Operation::Release { id: server },
                &mut NoHost,
            )?;
            return Err(error);
        }
    };
    let shared = Arc::new(Shared {
        config,
        state: Mutex::new(State {
            jobs: BTreeMap::new(),
            order: vec![],
            current: None,
            events: vec![],
            event_status: json!({"state":"starting","error":""}),
            running: true,
        }),
        wake: Condvar::new(),
        stopped: AtomicBool::new(false),
        cancel: AtomicBool::new(false),
        sdk: Mutex::new(()),
        cache,
        threads: Mutex::new(vec![]),
    });
    let mut control_thread = None;
    let mut event_thread = None;
    let outcome = (|| -> Result<()> {
        {
            let mut state = shared.lock()?;
            daemon_control::reload(&mut StateHost {
                shared: &shared,
                state: &mut state,
            })?;
            shared.persist(&state)?;
        }
        let control = Arc::clone(&shared);
        control_thread = Some(thread::spawn(move || {
            let result = control_server::run(
                control_server::Operation::Serve { id: server },
                &mut Control(Arc::clone(&control)),
            );
            if result.is_err() {
                control.stopped.store(true, Ordering::Release);
                control.cancel.store(true, Ordering::Release);
                control.wake.notify_all();
            }
        }));
        let events = Arc::clone(&shared);
        event_thread = Some(thread::spawn(move || {
            if events.events().is_err() {
                events.stopped.store(true, Ordering::Release);
                events.cancel.store(true, Ordering::Release);
                events.wake.notify_all();
            }
        }));
        shared.main_loop(callback)
    })();
    shared.stopped.store(true, Ordering::Release);
    shared.cancel.store(true, Ordering::Release);
    shared.wake.notify_all();
    if let Ok(mut state) = shared.lock() {
        for job in state.jobs.values_mut() {
            let _ = Shared::stop_watch(job);
            job.runtime["state"] = json!(if job.config["paused"] == true {
                "paused"
            } else {
                "stopped"
            });
        }
        state.running = false;
        let _ = shared.persist(&state);
    }
    let _ = control_server::run(control_server::Operation::Stop { id: server }, &mut NoHost);
    if let Some(task) = event_thread {
        let _ = task.join();
    }
    if let Some(task) = control_thread {
        let _ = task.join();
    }
    if let Ok(mut tasks) = shared.threads.lock() {
        for task in tasks.drain(..) {
            let _ = task.join();
        }
    }
    control_server::run(
        control_server::Operation::Release { id: server },
        &mut NoHost,
    )?;
    metadata_cache::run(
        metadata_cache::Operation::Release { id: cache },
        &mut NoHost,
    )?;
    outcome?;
    Ok(json!({"stopped":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn job_status_keeps_configuration_order_through_updates_and_removal() {
        let shared = Arc::new(Shared {
            config: Config {
                state: PathBuf::from("/unused"),
                executable: "/unused".into(),
                mount_paths: vec![],
            },
            state: Mutex::new(State {
                jobs: BTreeMap::new(),
                order: vec![],
                current: None,
                events: vec![],
                event_status: json!({}),
                running: true,
            }),
            wake: Condvar::new(),
            stopped: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            sdk: Mutex::new(()),
            cache: 0,
            threads: Mutex::new(vec![]),
        });
        let mut state = shared.lock().unwrap();
        let mut host = StateHost {
            shared: &shared,
            state: &mut state,
        };
        for id in ["z", "a", "z"] {
            host.request(
                "daemon_state",
                json!({"action":"upsert","job":{"id":id,"paused":false}}),
            )
            .unwrap();
        }
        let status = host
            .request("daemon_state", json!({"action":"status"}))
            .unwrap();
        assert_eq!(
            status["jobs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|j| text(j, "id"))
                .collect::<Vec<_>>(),
            vec!["z", "a"]
        );
        host.request("daemon_state", json!({"action":"remove","id":"z"}))
            .unwrap();
        let status = host
            .request("daemon_state", json!({"action":"status"}))
            .unwrap();
        assert_eq!(status["jobs"].as_array().unwrap().len(), 1);
        assert_eq!(status["jobs"][0]["id"], "a");
    }
}
