//! Command/reload orchestration. The host owns synchronization and runtime objects.
use crate::{host::Services, process::Failure};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
type Result<T> = std::result::Result<T, Failure>;
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Status,
    Stop,
    StopIdle,
    Reload,
    Pause,
    #[serde(rename = "pause_1h")]
    Pause1h,
    #[serde(rename = "pause_4h")]
    Pause4h,
    #[serde(rename = "pause_24h")]
    Pause24h,
    Resume,
    Sync,
}
fn call(host: &mut impl Services, action: &str, mut args: Value) -> Result<Value> {
    args["action"] = json!(action);
    host.request("daemon_state", args)
}
fn patch(host: &mut impl Services, id: &Value, fields: Value) -> Result<()> {
    call(host, "patch", json!({"id":id,"fields":fields}))?;
    Ok(())
}
fn jobs(host: &mut impl Services) -> Result<Vec<Value>> {
    call(host, "jobs", json!({}))?
        .as_array()
        .cloned()
        .ok_or_else(|| "Invalid daemon job list".into())
}
pub fn reload(host: &mut impl Services) -> Result<()> {
    let jobs = jobs(host)?;
    let snapshot = call(host, "snapshot", json!({}))?;
    let runtimes = snapshot["jobs"]
        .as_object()
        .ok_or("Invalid daemon snapshot")?;
    let ids: BTreeSet<_> = jobs.iter().map(|j| j["id"].clone().to_string()).collect();
    for id in runtimes.keys() {
        if !ids.contains(&json!(id).to_string()) {
            call(host, "stop_watcher", json!({"id":id}))?;
            call(host, "remove", json!({"id":id}))?;
            if snapshot["current"] == *id {
                call(host, "cancel", json!({}))?;
            }
        }
    }
    for job in jobs {
        let id = &job["id"];
        let old = runtimes.get(id.as_str().ok_or("Invalid job identity")?);
        let was_paused = old.is_none_or(|r| r["job"]["paused"] == true);
        call(host, "upsert", json!({"job":job}))?;
        if job["paused"] == true {
            patch(
                host,
                id,
                json!({"state":"paused","pending":false,"watch_token":""}),
            )?;
            call(host, "stop_watcher", json!({"id":id}))?;
            if snapshot["current"] == *id {
                call(host, "cancel", json!({}))?;
            }
        } else if was_paused || old.is_none_or(|r| r["watcher"] != true) {
            let watch = (|| -> Result<()> {
                call(host, "watch", json!({"id":id}))?;
                call(host, "invalidate", json!({"root":job["remote_root"]}))?;
                patch(host, id, json!({"state":"pending","error":""}))?;
                call(
                    host,
                    "schedule",
                    json!({"id":id,"reason":if was_paused {"startup"} else {"resume"}}),
                )?;
                Ok(())
            })();
            if let Err(error) = watch {
                patch(
                    host,
                    id,
                    json!({"state":"blocked","pending":false,"error":error.message}),
                )?;
            }
        }
    }
    call(host, "notify", json!({}))?;
    Ok(())
}
pub fn command(action: Action, job_id: Option<String>, host: &mut impl Services) -> Result<Value> {
    match action {
        Action::Status => return call(host, "status", json!({})),
        Action::StopIdle => {
            // Shared::command holds the same state lock used to select a job.
            let snapshot = call(host, "snapshot", json!({}))?;
            if snapshot["current"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
            {
                return Err(
                    "Synchronization is active; finish it before installing an update.".into(),
                );
            }
            call(host, "stop", json!({}))?;
            call(host, "notify", json!({}))?;
        }
        Action::Stop => {
            call(host, "stop", json!({}))?;
            call(host, "cancel", json!({}))?;
            call(host, "notify", json!({}))?;
        }
        Action::Reload => reload(host)?,
        Action::Pause
        | Action::Pause1h
        | Action::Pause4h
        | Action::Pause24h
        | Action::Resume
        | Action::Sync => {
            let selected: Vec<_> = jobs(host)?
                .into_iter()
                .filter(|j| job_id.as_ref().is_none_or(|id| j["id"] == *id))
                .collect();
            if selected.is_empty() {
                return Err("Sync-Job wurde nicht gefunden.".into());
            }
            if matches!(action, Action::Sync) {
                let snapshot = call(host, "snapshot", json!({}))?;
                for job in selected {
                    if job["paused"] != true
                        && snapshot["jobs"]
                            .get(job["id"].as_str().ok_or("Invalid job identity")?)
                            .is_some()
                    {
                        call(host, "invalidate", json!({"root":job["remote_root"]}))?;
                        call(host, "schedule", json!({"id":job["id"],"reason":"manual"}))?;
                    }
                }
            } else {
                let paused = !matches!(action, Action::Resume);
                let seconds = match action {
                    Action::Pause1h => 3600,
                    Action::Pause4h => 14400,
                    Action::Pause24h => 86400,
                    _ => 0,
                };
                let until = if seconds > 0 {
                    Some(
                        call(host, "wall", json!({}))?
                            .as_u64()
                            .ok_or("Invalid daemon clock")?
                            .checked_add(seconds)
                            .ok_or("Invalid pause deadline")?,
                    )
                } else {
                    None
                };
                for job in &selected {
                    call(
                        host,
                        "update_job",
                        json!({"id":job["id"],"paused":paused,"paused_until":until}),
                    )?;
                }
                if !paused {
                    let snapshot = call(host, "snapshot", json!({}))?;
                    for job in &selected {
                        if snapshot["jobs"]
                            .get(job["id"].as_str().ok_or("Invalid job identity")?)
                            .is_some()
                        {
                            patch(
                                host,
                                &job["id"],
                                json!({"state":"pending","watch_token":""}),
                            )?;
                            call(host, "stop_watcher", json!({"id":job["id"]}))?;
                        }
                    }
                }
                reload(host)?;
            }
        }
    }
    call(host, "persist", json!({}))?;
    call(host, "status", json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Host {
        jobs: Vec<Value>,
        runtime: Value,
        current: Value,
        calls: Vec<Value>,
        watch_error: bool,
    }
    impl Services for Host {
        fn poll(&mut self) -> Result<()> {
            Ok(())
        }
        fn request(&mut self, method: &str, args: Value) -> Result<Value> {
            assert_eq!(method, "daemon_state");
            self.calls.push(args.clone());
            let id = args["id"].as_str().unwrap_or("");
            match args["action"].as_str().unwrap() {
                "wall" => return Ok(json!(1000)),
                "jobs" => return Ok(json!(self.jobs)),
                "snapshot" => return Ok(json!({"jobs":self.runtime,"current":self.current})),
                "status" => return Ok(self.runtime.clone()),
                "update_job" => {
                    for job in &mut self.jobs {
                        if job["id"] == id {
                            job["paused"] = args["paused"].clone();
                            job["paused_until"] = args["paused_until"].clone();
                        }
                    }
                }
                "upsert" => {
                    let job = &args["job"];
                    self.runtime[job["id"].as_str().unwrap()]["job"] = job.clone();
                }
                "remove" => {
                    self.runtime.as_object_mut().unwrap().remove(id);
                }
                "stop_watcher" => self.runtime[id]["watcher"] = json!(false),
                "patch" => {
                    for (key, value) in args["fields"].as_object().unwrap() {
                        self.runtime[id][key] = value.clone();
                    }
                }
                "watch" => {
                    if self.watch_error {
                        return Err("replaced local root".into());
                    }
                    self.runtime[id]["watcher"] = json!(true);
                }
                _ => {}
            }
            Ok(Value::Null)
        }
    }
    fn host() -> Host {
        let job = json!({"id":"a","paused":false,"remote_root":"/my-files/a","remote_uid":"pinned-root","account_root_uid":"pinned-account"});
        Host {
            jobs: vec![job.clone()],
            runtime: json!({"a":{"job":job,"watcher":true,"state":"syncing"}}),
            current: json!("a"),
            calls: vec![],
            watch_error: false,
        }
    }
    #[test]
    fn timed_pause_uses_wall_clock_and_manual_resume_clears_deadline() {
        for (wire, seconds) in [
            ("pause_1h", 3600),
            ("pause_4h", 14400),
            ("pause_24h", 86400),
        ] {
            let mut h = host();
            let action: Action = serde_json::from_value(json!(wire)).unwrap();
            command(action, Some("a".into()), &mut h).unwrap();
            assert_eq!(h.jobs[0]["paused_until"], 1000 + seconds);
            assert!(has(&h, "cancel"));
            assert_eq!(h.runtime["a"]["watcher"], false);
            command(Action::Resume, Some("a".into()), &mut h).unwrap();
            assert!(h.jobs[0]["paused_until"].is_null());
            assert_eq!(h.jobs[0]["paused"], false);
            assert!(has(&h, "schedule"));
        }
        assert!(serde_json::from_value::<Action>(json!("pause_999h")).is_err());
    }
    #[test]
    fn idle_stop_is_atomic_and_never_cancels_an_active_job() {
        let mut active = host();
        assert!(command(Action::StopIdle, None, &mut active).is_err());
        assert!(!has(&active, "stop") && !has(&active, "cancel"));
        active.current = Value::Null;
        command(Action::StopIdle, None, &mut active).unwrap();
        assert!(has(&active, "stop") && has(&active, "notify"));
        assert!(!has(&active, "cancel"));
    }
    fn has(host: &Host, action: &str) -> bool {
        host.calls.iter().any(|c| c["action"] == action)
    }
    #[test]
    fn pause_cancels_current_job_and_persists_before_reply() {
        let mut h = host();
        command(Action::Pause, Some("a".into()), &mut h).unwrap();
        assert_eq!(h.jobs[0]["paused"], true);
        assert_eq!(h.runtime["a"]["pending"], false);
        assert_eq!(h.runtime["a"]["watcher"], false);
        assert!(has(&h, "cancel"));
        assert!(!has(&h, "schedule"));
        assert_eq!(h.calls[h.calls.len() - 2]["action"], "persist");
        assert_eq!(h.calls.last().unwrap()["action"], "status");
    }
    #[test]
    fn resume_retries_blocked_watch_without_erasing_identity() {
        let mut h = host();
        h.runtime["a"]["state"] = json!("blocked");
        command(Action::Resume, None, &mut h).unwrap();
        assert_eq!(h.jobs[0]["remote_uid"], "pinned-root");
        assert_eq!(h.jobs[0]["account_root_uid"], "pinned-account");
        assert_eq!(h.runtime["a"]["state"], "pending");
        assert!(has(&h, "watch"));
        assert!(has(&h, "schedule"));
        let mut h = host();
        h.watch_error = true;
        command(Action::Resume, None, &mut h).unwrap();
        assert_eq!(h.runtime["a"]["state"], "blocked");
        assert_eq!(h.runtime["a"]["pending"], false);
        assert!(!has(&h, "schedule"));
    }
    #[test]
    fn removal_cancels_work_and_manual_sync_does_not_resume_paused_jobs() {
        let mut h = host();
        h.jobs.clear();
        reload(&mut h).unwrap();
        assert!(h.runtime.as_object().unwrap().is_empty());
        assert!(has(&h, "cancel"));
        let mut h = host();
        h.jobs[0]["paused"] = json!(true);
        command(Action::Sync, None, &mut h).unwrap();
        assert!(!has(&h, "schedule"));
        assert!(!has(&h, "update_job"));
        let mut h = host();
        assert!(command(Action::Pause, Some("missing".into()), &mut h).is_err());
        assert!(!has(&h, "update_job"));
    }
}
