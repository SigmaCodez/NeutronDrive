//! Desktop event-source ownership, polling, durable acknowledgement and scheduling.
use crate::{
    event_runtime as runtime,
    host::{Callback, Services},
    process::Failure,
    storage,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, LazyLock, Mutex,
    },
    thread::ThreadId,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
type Result<T> = std::result::Result<T, Failure>;
const ERROR: &str =
    "Drive-Ereignisse sind vorübergehend nicht verfügbar; ausstehende Änderungen bleiben erhalten.";
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Create {
        state_dir: PathBuf,
        roots: Vec<String>,
        interval: f64,
        dynamic_roots: bool,
    },
    Snapshot {
        id: u64,
    },
    Prepare {
        id: u64,
    },
    Load {
        id: u64,
    },
    Poll {
        id: u64,
    },
    Start {
        id: u64,
    },
    Run {
        id: u64,
    },
    Stop {
        id: u64,
        timeout: f64,
    },
    AbortStart {
        id: u64,
    },
    Forget {
        id: u64,
    },
}
struct State {
    roots: Vec<String>,
    file: PathBuf,
    cursors: Value,
    known: Vec<Value>,
    announced: bool,
    status: Value,
    running: bool,
    owner: Option<ThreadId>,
    poll_owner: Option<ThreadId>,
}
struct Source {
    directory: PathBuf,
    interval: f64,
    dynamic: bool,
    state: Mutex<State>,
    polling: Mutex<()>,
    control: Mutex<()>,
    wake: Condvar,
    stop: AtomicBool,
    released: AtomicBool,
}
static SOURCES: LazyLock<Mutex<BTreeMap<u64, Arc<Source>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static NEXT: AtomicU64 = AtomicU64::new(1);
fn cancelled() -> Failure {
    Failure {
        message: "Ereignisquelle angehalten.".into(),
        cancelled: true,
    }
}
pub(crate) fn state_file(directory: &Path, roots: &[String]) -> PathBuf {
    let encoded = format!(
        "[{}]",
        roots
            .iter()
            .map(|r| crate::ondemand_io::ascii_json(&json!(r)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let key = format!("{:x}", Sha256::digest(encoded.as_bytes()));
    directory.join(format!("events-{}.json", &key[..24]))
}
fn jitter() -> f64 {
    let mut data = [0; 8];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut data))
        .is_ok()
    {
        (u64::from_ne_bytes(data) >> 11) as f64 / (1u64 << 53) as f64
    } else {
        0.5
    }
}
impl Source {
    fn new(directory: PathBuf, roots: Vec<String>, interval: f64, dynamic: bool) -> Result<Self> {
        runtime::roots(&roots)?;
        if !interval.is_finite() || interval > 1e9 {
            return Err("Invalid event interval".into());
        }
        let directory = if directory.is_absolute() {
            directory
        } else {
            std::env::current_dir()
                .map_err(|_| Failure::from("Working directory unavailable"))?
                .join(directory)
        };
        let file = state_file(&directory, &roots);
        Ok(Self {
            directory,
            interval: interval.max(30.),
            dynamic,
            state: Mutex::new(State {
                roots,
                file,
                cursors: json!({}),
                known: vec![],
                announced: false,
                status: json!({"state":"stopped","error":"","last_success":null,"retry_seconds":0}),
                running: false,
                owner: None,
                poll_owner: None,
            }),
            polling: Mutex::new(()),
            control: Mutex::new(()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            released: AtomicBool::new(false),
        })
    }
    fn alive(&self) -> Result<()> {
        if self.released.load(Ordering::Acquire) {
            return Err("Event source was released".into());
        }
        Ok(())
    }
    fn snapshot(&self) -> Result<Value> {
        self.alive()?;
        let s = self.state.lock().unwrap();
        Ok(
            json!({"roots":s.roots,"file":s.file,"cursors":s.cursors,"known_roots":s.known,"announced":s.announced,"status":s.status,"running":s.running,"interval":self.interval}),
        )
    }
    fn load_inner(&self) -> Result<()> {
        let (file, roots) = {
            let s = self.state.lock().unwrap();
            (s.file.clone(), s.roots.clone())
        };
        let loaded = runtime::load(&file, &roots)?;
        let mut s = self.state.lock().unwrap();
        s.cursors = loaded["cursors"].clone();
        s.known = loaded["roots"]
            .as_array()
            .ok_or("Invalid cursor roots")?
            .clone();
        Ok(())
    }
    fn load(&self) -> Result<Value> {
        let _poll = self
            .polling
            .try_lock()
            .map_err(|_| "Event source is busy")?;
        self.alive()?;
        self.load_inner()?;
        Ok(Value::Null)
    }
    fn external(&self, v: Value, callback: &mut impl FnMut(Value) -> i32) -> i32 {
        if self.stop.load(Ordering::Acquire) {
            return 1;
        }
        let code = callback(v);
        if code != 0 {
            self.stop.store(true, Ordering::Release);
            self.wake.notify_all();
        }
        code
    }
    fn host(
        &self,
        method: &str,
        args: Value,
        callback: &mut impl FnMut(Value) -> i32,
    ) -> Result<Value> {
        Callback(&mut |v| self.external(v, callback)).request(method, args)
    }
    fn poll(
        &self,
        executable: Option<&str>,
        host_delivery: bool,
        callback: &mut impl FnMut(Value) -> i32,
    ) -> Result<Value> {
        let _poll = self
            .polling
            .try_lock()
            .map_err(|_| "Backend ist noch beschäftigt")?;
        self.alive()?;
        {
            let mut s = self.state.lock().unwrap();
            if s.running && s.owner != Some(std::thread::current().id()) {
                return Err("Backend ist noch beschäftigt".into());
            }
            s.poll_owner = Some(std::thread::current().id());
        }
        struct Clear<'a>(&'a Source);
        impl Drop for Clear<'_> {
            fn drop(&mut self) {
                self.0.state.lock().unwrap().poll_owner = None;
                self.0.wake.notify_all();
            }
        }
        let _clear = Clear(self);
        if self.stop.load(Ordering::Acquire) {
            return Err(cancelled());
        }
        if self.dynamic {
            let roots: Vec<String> =
                serde_json::from_value(self.host("event_roots", Value::Null, callback)?)
                    .map_err(|_| "Invalid event root provider response")?;
            runtime::roots(&roots)?;
            let changed = self.state.lock().unwrap().roots != roots;
            if changed {
                let file = state_file(&self.directory, &roots);
                let loaded = runtime::load(&file, &roots)?;
                let mut s = self.state.lock().unwrap();
                s.roots = roots;
                s.file = file;
                s.cursors = loaded["cursors"].clone();
                s.known = loaded["roots"]
                    .as_array()
                    .ok_or("Invalid cursor roots")?
                    .clone();
                s.announced = false;
            }
        }
        let (file, roots, cursors, known, announced, ready) = {
            let s = self.state.lock().unwrap();
            (
                s.file.clone(),
                s.roots.clone(),
                s.cursors.clone(),
                s.known.clone(),
                s.announced,
                s.status["state"] == "ready",
            )
        };
        let batch = match executable {
            Some(executable) => runtime::query(executable.into(), &roots, cursors, &mut |v| {
                self.external(v, callback)
            })?,
            None => self.host(
                "events_query",
                json!({"paths":roots,"cursors":cursors}),
                callback,
            )?,
        };
        let mut delivery_error = None;
        let delivered = runtime::deliver(
            runtime::Delivery {
                file: file.to_string_lossy().into_owned(),
                paths: roots,
                known_roots: known,
                announced,
                ready,
            },
            batch,
            &mut |v| {
                if self.stop.load(Ordering::Acquire) {
                    return 1;
                }
                if host_delivery && v["kind"] == "remote_event" {
                    if let Err(e) = self.host("event_accept", v["event"].clone(), callback) {
                        delivery_error = Some(e);
                        return 1;
                    }
                    i32::from(self.stop.load(Ordering::Acquire))
                } else {
                    self.external(v, callback)
                }
            },
        );
        if let Some(e) = delivery_error {
            return Err(e);
        }
        let batch = delivered?;
        let mut s = self.state.lock().unwrap();
        s.cursors = batch["cursors"].clone();
        s.known = batch["roots"]
            .as_array()
            .ok_or("Invalid event roots")?
            .clone();
        s.announced = true;
        s.status = json!({"state":"ready","error":"","last_success":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64(),"retry_seconds":self.interval});
        Ok(batch)
    }
    fn start(&self) -> Result<Value> {
        let _control = self.control.lock().unwrap();
        self.alive()?;
        if self.state.lock().unwrap().running {
            return Ok(json!(false));
        }
        let _poll = self
            .polling
            .try_lock()
            .map_err(|_| "Event source is busy")?;
        self.load_inner()?;
        let mut s = self.state.lock().unwrap();
        self.stop.store(false, Ordering::Release);
        s.announced = false;
        s.running = true;
        s.owner = None;
        s.status["state"] = json!("starting");
        s.status["error"] = json!("");
        Ok(json!(true))
    }
    fn finish(&self) {
        let mut s = self.state.lock().unwrap();
        s.running = false;
        s.owner = None;
        s.status["state"] = json!("stopped");
        s.status["retry_seconds"] = json!(0);
        self.wake.notify_all();
    }
    fn run(
        &self,
        executable: Option<&str>,
        host_delivery: bool,
        callback: &mut impl FnMut(Value) -> i32,
    ) -> Result<Value> {
        {
            let mut s = self.state.lock().unwrap();
            self.alive()?;
            if !s.running || s.owner.is_some() {
                return Err("Event source is not prepared".into());
            }
            s.owner = Some(std::thread::current().id());
        }
        struct Finish<'a>(&'a Source);
        impl Drop for Finish<'_> {
            fn drop(&mut self) {
                self.0.finish();
            }
        }
        let _finish = Finish(self);
        let mut failures = 0u64;
        while !self.stop.load(Ordering::Acquire) {
            let mut delay = self.interval;
            match self.poll(executable, host_delivery, callback) {
                Ok(_) => failures = 0,
                Err(e) => {
                    if self.stop.load(Ordering::Acquire) {
                        break;
                    }
                    if e.cancelled {
                        let mut s = self.state.lock().unwrap();
                        s.status["state"] = json!("waiting");
                        s.status["error"] = json!("");
                    } else if e.message.contains("andere NeutronDrive-Instanz")
                        || e.message.contains("Backend ist noch beschäftigt")
                    {
                        let mut s = self.state.lock().unwrap();
                        s.status["state"] = json!("busy");
                        s.status["error"] = json!("");
                        s.status["retry_seconds"] = json!(delay);
                    } else {
                        let retry=crate::policy::dispatch(json!({"version":1,"op":"event_retry","interval":self.interval,"failures":failures,"jitter":jitter()}).to_string().as_bytes()).map_err(Failure::from)?;
                        delay = retry["delay"].as_f64().ok_or("Invalid event retry delay")?;
                        failures = retry["failures"].as_u64().ok_or("Invalid retry count")?;
                        {
                            let mut s = self.state.lock().unwrap();
                            s.status["state"] = json!("error");
                            s.status["error"] = json!(ERROR);
                            s.status["retry_seconds"] = json!(delay);
                        }
                        let event = json!({"kind":"error","message":ERROR});
                        if host_delivery {
                            let _ = self.host("event_accept", event, callback);
                        } else {
                            self.external(json!({"kind":"remote_event","event":event}), callback);
                        }
                    }
                }
            }
            let deadline = Instant::now() + Duration::from_secs_f64(delay);
            while !self.stop.load(Ordering::Acquire) {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let s = self.state.lock().unwrap();
                let (s, _) = self
                    .wake
                    .wait_timeout_while(s, remaining.min(Duration::from_millis(100)), |_| {
                        !self.stop.load(Ordering::Acquire)
                    })
                    .unwrap();
                drop(s);
                // Also observe a standalone executable's signal callback while
                // waiting; this does not issue another cloud request.
                self.external(json!({"kind":"poll"}), callback);
            }
        }
        Ok(Value::Null)
    }
    fn stop(&self, timeout: f64) -> Result<Value> {
        self.alive()?;
        if !timeout.is_finite() || !(0.0..=3600.0).contains(&timeout) {
            return Err("Invalid event stop timeout".into());
        }
        self.stop.store(true, Ordering::Release);
        let mut s = self.state.lock().unwrap();
        self.wake.notify_all();
        if s.running {
            s.status["state"] = json!("stopping");
        }
        let current = std::thread::current().id();
        if s.owner != Some(current) && s.poll_owner != Some(current) {
            s = self
                .wake
                .wait_timeout_while(s, Duration::from_secs_f64(timeout), |s| {
                    s.running || s.poll_owner.is_some()
                })
                .unwrap()
                .0;
        }
        if !s.running {
            s.status["state"] = json!("stopped");
            s.status["retry_seconds"] = json!(0);
        }
        Ok(Value::Null)
    }
}
fn get(id: u64) -> Result<Arc<Source>> {
    SOURCES
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| "Unknown event source".into())
}
pub fn run(
    operation: Operation,
    executable: Option<String>,
    host_delivery: bool,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    match operation {
        Operation::Create {
            state_dir,
            roots,
            interval,
            dynamic_roots,
        } => {
            let source = Source::new(state_dir, roots, interval, dynamic_roots)?;
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            SOURCES.lock().unwrap().insert(id, Arc::new(source));
            Ok(json!(id))
        }
        Operation::Forget { id } => {
            let mut registry = SOURCES.lock().unwrap();
            if let Some(source) = registry.get(&id).cloned() {
                let _control = source
                    .control
                    .try_lock()
                    .map_err(|_| "Event source is busy")?;
                let _poll = source
                    .polling
                    .try_lock()
                    .map_err(|_| "Event source is busy")?;
                if source.state.lock().unwrap().running {
                    return Err("Cannot release a running event source".into());
                }
                source.released.store(true, Ordering::Release);
                registry.remove(&id);
            }
            Ok(Value::Null)
        }
        operation => {
            let id = match &operation {
                Operation::Snapshot { id }
                | Operation::Prepare { id }
                | Operation::Load { id }
                | Operation::Poll { id }
                | Operation::Start { id }
                | Operation::Run { id }
                | Operation::Stop { id, .. }
                | Operation::AbortStart { id } => *id,
                _ => unreachable!(),
            };
            let source = get(id)?;
            source.alive()?;
            match operation {
                Operation::Snapshot { .. } => source.snapshot(),
                Operation::Prepare { .. } => {
                    storage::directory(&source.directory, true, true)?;
                    Ok(Value::Null)
                }
                Operation::Load { .. } => source.load(),
                Operation::Poll { .. } => {
                    source.poll(executable.as_deref(), host_delivery, callback)
                }
                Operation::Start { .. } => source.start(),
                Operation::Run { .. } => source.run(executable.as_deref(), host_delivery, callback),
                Operation::Stop { timeout, .. } => source.stop(timeout),
                Operation::AbortStart { .. } => {
                    let _control = source.control.lock().unwrap();
                    if source.state.lock().unwrap().owner.is_some() {
                        return Err("Event source already started".into());
                    }
                    source.stop.store(true, Ordering::Release);
                    source.finish();
                    Ok(Value::Null)
                }
                _ => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "nd-event-owner-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            storage::directory(&p, true, true).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn batch() -> Value {
        json!({"protocol":1,"type":"batch","roots":[{"path":"/my-files","uid":"root","scopeId":"scope"}],"events":[{"type":"node_updated","treeEventScopeId":"scope","eventId":"next","nodeUid":"file"}],"cursors":{"scope":"next"},"resets":[]})
    }
    #[test]
    fn cursor_filename_keeps_python_ascii_json_and_spacing_compatibility() {
        assert_eq!(
            state_file(
                Path::new("/state"),
                &["/my-files".into(), "/devices/Ä😀".into()]
            ),
            Path::new("/state/events-c05ad4c296c3034dd281468a.json")
        );
    }
    #[test]
    fn callback_rejection_preserves_owned_cursor_and_retry_delivers_again() {
        let t = Temp::new();
        let source = Source::new(t.0.clone(), vec!["/my-files".into()], 30., false).unwrap();
        let file = source.state.lock().unwrap().file.clone();
        let mut accepted = 0;
        assert!(source
            .poll(None, true, &mut |v| {
                if v["kind"] == "host_request" {
                    let id = v["id"].as_u64().unwrap();
                    if v["method"] == "events_query" {
                        crate::host::reply(id, batch()).unwrap();
                    } else {
                        accepted += 1;
                        assert!(!file.exists());
                        crate::host::fail(id, "consumer rejected".into(), false).unwrap();
                    }
                }
                0
            })
            .is_err());
        assert_eq!(accepted, 1);
        assert_eq!(source.snapshot().unwrap()["cursors"], json!({}));
        assert!(!file.exists());
        source
            .poll(None, true, &mut |v| {
                if v["kind"] == "host_request" {
                    let id = v["id"].as_u64().unwrap();
                    let result = if v["method"] == "events_query" {
                        batch()
                    } else {
                        accepted += 1;
                        assert!(!file.exists());
                        Value::Null
                    };
                    crate::host::reply(id, result).unwrap();
                }
                0
            })
            .unwrap();
        assert_eq!(accepted, 3);
        assert_eq!(
            source.snapshot().unwrap()["cursors"],
            json!({"scope":"next"})
        );
        assert!(file.exists());
    }
    #[test]
    fn a_reconnected_empty_batch_reannounces_ready_without_reset() {
        let t = Temp::new();
        let source = Source::new(t.0.clone(), vec!["/my-files".into()], 30., false).unwrap();
        let mut notifications = vec![];
        let mut exchange = |v: Value| {
            if v["kind"] == "host_request" {
                let id = v["id"].as_u64().unwrap();
                let response = if v["method"] == "events_query" {
                    let mut b = batch();
                    b["events"] = json!([]);
                    b
                } else {
                    notifications.push(v["arguments"].clone());
                    Value::Null
                };
                crate::host::reply(id, response).unwrap();
            }
            0
        };
        source.poll(None, true, &mut exchange).unwrap();
        {
            let mut state = source.state.lock().unwrap();
            state.status["state"] = json!("error");
            state.status["error"] = json!(ERROR);
        }
        source.poll(None, true, &mut exchange).unwrap();
        assert_eq!(notifications.len(), 2);
        assert_eq!(notifications[1]["kind"], "ready");
        assert_ne!(notifications[1]["reset"], true);
        assert_eq!(source.snapshot().unwrap()["status"]["error"], "");
    }
    #[test]
    fn loop_redacts_failures_and_stop_inside_receiver_never_deadlocks() {
        let t = Temp::new();
        let source = Source::new(t.0.clone(), vec!["/my-files".into()], 30., false).unwrap();
        source.start().unwrap();
        source
            .run(None, true, &mut |v| {
                if v["kind"] == "host_request" {
                    let id = v["id"].as_u64().unwrap();
                    if v["method"] == "events_query" {
                        crate::host::fail(id, "secret-token".into(), false).unwrap();
                    } else {
                        assert_eq!(v["arguments"]["kind"], "error");
                        assert!(!v.to_string().contains("secret-token"));
                        source.stop(5.).unwrap();
                        crate::host::reply(id, Value::Null).unwrap();
                    }
                }
                0
            })
            .unwrap();
        let s = source.snapshot().unwrap();
        assert_eq!(s["running"], false);
        assert_eq!(s["status"]["state"], "stopped");
        assert!(!s.to_string().contains("secret-token"));
        assert_eq!(s["cursors"], json!({}));
    }
    #[test]
    fn signal_callback_interrupts_retry_wait_without_another_sdk_query() {
        let t = Temp::new();
        let source = Source::new(t.0.clone(), vec!["/my-files".into()], 30., false).unwrap();
        source.start().unwrap();
        let mut queries = 0;
        let start = Instant::now();
        source
            .run(None, true, &mut |v| {
                if v["kind"] == "host_request" {
                    let id = v["id"].as_u64().unwrap();
                    let response = if v["method"] == "events_query" {
                        queries += 1;
                        batch()
                    } else {
                        Value::Null
                    };
                    crate::host::reply(id, response).unwrap();
                }
                i32::from(
                    v["kind"] == "poll" && source.snapshot().unwrap()["status"]["state"] == "ready",
                )
            })
            .unwrap();
        assert_eq!(queries, 1);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_eq!(source.snapshot().unwrap()["running"], false);
    }
}
