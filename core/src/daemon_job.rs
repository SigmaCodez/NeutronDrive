//! Job state transitions shared by the daemon loop and compatibility binding.
use crate::{filesystem, policy, process::Failure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::Path};
type Result<T> = std::result::Result<T, Failure>;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Runtime {
    state: String,
    pending: bool,
    due: f64,
    first_dirty: f64,
    paths: BTreeSet<Option<String>>,
    reasons: BTreeSet<String>,
    known_uids: BTreeSet<String>,
    failures: u64,
    last_sync: Option<f64>,
    error: String,
    conflicts: Vec<String>,
    skipped: Vec<String>,
    completed: usize,
    #[serde(default)]
    progress: Value,
    rebuild_watch: bool,
    local_snapshot: Value,
    snapshot_ready: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    paused: bool,
    stopping: bool,
    root: String,
    now: f64,
    wall: f64,
    debounce: f64,
    retry_base: f64,
    retry_max: f64,
    jitter: f64,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Begin,
    Cancelled,
    Blocked { error: String },
    Retry { error: String },
    Finalize,
    Complete { previous: Value, result: Completion },
}
#[derive(Deserialize)]
pub struct Completion {
    errors: Vec<String>,
    completed: Vec<Value>,
    conflicts: Vec<String>,
    skipped: Vec<String>,
    #[serde(default)]
    trashed: Vec<String>,
    _local_snapshot: Value,
    _remote_snapshot: Value,
}
fn policy_call(mut args: Value) -> Result<Value> {
    args["version"] = json!(1);
    policy::dispatch(&serde_json::to_vec(&args).map_err(|e| e.to_string())?).map_err(Into::into)
}
fn retry(state: &mut Runtime, error: String, context: &Context) -> Result<()> {
    let update = policy_call(
        json!({"op":"retry","failures":state.failures,"now":context.now,"base":context.retry_base,"maximum":context.retry_max,"jitter":context.jitter}),
    )?;
    state.failures = update["failures"].as_u64().ok_or("Invalid retry count")?;
    state.due = update["due"].as_f64().ok_or("Invalid retry deadline")?;
    state.state = "offline".into();
    state.pending = true;
    state.error = error;
    state.reasons.insert("retry".into());
    Ok(())
}
pub fn transition(mut runtime: Runtime, outcome: Outcome, context: Context) -> Result<Value> {
    if [
        context.now,
        context.wall,
        context.debounce,
        context.retry_base,
        context.retry_max,
        context.jitter,
        runtime.due,
        runtime.first_dirty,
    ]
    .iter()
    .any(|x| !x.is_finite() || *x < 0.0)
    {
        return Err("Invalid daemon clock or timing configuration".into());
    }
    match outcome {
        Outcome::Begin => {
            runtime.state = "syncing".into();
            runtime.progress = Value::Null;
            runtime.pending = false;
            runtime.paths.clear();
            runtime.reasons.clear();
            runtime.first_dirty = 0.0;
        }
        Outcome::Cancelled => {
            runtime.state = if context.paused {
                "paused"
            } else if context.stopping {
                "stopped"
            } else {
                "pending"
            }
            .into();
            runtime.pending = !context.paused && !context.stopping;
            runtime.due = context.now + context.debounce;
        }
        Outcome::Blocked { error } => {
            runtime.state = "blocked".into();
            runtime.pending = false;
            runtime.error = error;
        }
        Outcome::Retry { error } => retry(&mut runtime, error, &context)?,
        Outcome::Finalize => {
            if context.paused {
                runtime.state = "paused".into();
                runtime.pending = false;
            }
        }
        Outcome::Complete { previous, result } => {
            if result.errors.is_empty() {
                runtime.failures = 0;
            }
            runtime.last_sync = Some(context.wall);
            runtime.completed = result.completed.len();
            runtime.conflicts = result.conflicts;
            runtime.skipped = result.skipped;
            runtime.local_snapshot = policy_call(
                json!({"op":"echo","previous":previous,"current":result._local_snapshot,"remote":result._remote_snapshot,"trashed":result.trashed}),
            )?;
            runtime.snapshot_ready = result.errors.is_empty();
            let remote = result
                ._remote_snapshot
                .as_object()
                .ok_or("Invalid daemon remote snapshot")?;
            for node in remote.values() {
                runtime
                    .known_uids
                    .insert(node["uid"].as_str().unwrap_or("").to_owned());
            }
            runtime.error = result
                .errors
                .iter()
                .take(10)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            runtime.state = if runtime.conflicts.is_empty() {
                "idle"
            } else {
                "conflicts"
            }
            .into();
            if !result.errors.is_empty() {
                let error = runtime.error.clone();
                retry(&mut runtime, error, &context)?;
            } else if !runtime.paths.is_empty() {
                runtime.paths.retain(|path| {
                    path.as_ref().is_none_or(|p| {
                        !filesystem::matches_snapshot(
                            Path::new(&context.root),
                            p,
                            runtime.local_snapshot.clone(),
                        )
                    })
                });
                if runtime.paths.is_empty()
                    && runtime
                        .reasons
                        .iter()
                        .all(|s| matches!(s.as_str(), "local" | "startup" | "resume"))
                {
                    runtime.pending = false;
                    runtime.reasons.clear();
                }
            }
        }
    }
    serde_json::to_value(runtime).map_err(|e| e.to_string().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn runtime() -> Runtime {
        serde_json::from_value(json!({"state":"syncing","pending":true,"due":0.0,"first_dirty":1.0,"paths":[null],"reasons":["local"],"known_uids":[],"failures":2,"last_sync":null,"error":"old","conflicts":[],"skipped":[],"completed":0,"rebuild_watch":false,"local_snapshot":{},"snapshot_ready":false})).unwrap()
    }
    fn context() -> Context {
        Context {
            paused: false,
            stopping: false,
            root: "/unused".into(),
            now: 10.0,
            wall: 20.0,
            debounce: 1.5,
            retry_base: 30.0,
            retry_max: 900.0,
            jitter: 1.0,
        }
    }
    fn result(errors: Vec<String>) -> Completion {
        Completion {
            errors,
            completed: vec![],
            conflicts: vec![],
            skipped: vec![],
            trashed: vec![],
            _local_snapshot: json!({}),
            _remote_snapshot: json!({}),
        }
    }
    #[test]
    fn cancellation_pause_and_stop_do_not_restart_jobs() {
        for (paused, stopping, state, pending) in [
            (true, false, "paused", false),
            (false, true, "stopped", false),
            (false, false, "pending", true),
        ] {
            let mut c = context();
            c.paused = paused;
            c.stopping = stopping;
            let value = transition(runtime(), Outcome::Cancelled, c).unwrap();
            assert_eq!(value["state"], state);
            assert_eq!(value["pending"], pending);
            assert_eq!(value["due"], 11.5);
        }
    }
    #[test]
    fn partial_failure_keeps_dirty_work_and_uses_bounded_retry() {
        let value = transition(
            runtime(),
            Outcome::Complete {
                previous: json!({}),
                result: result(vec!["offline".into()]),
            },
            context(),
        )
        .unwrap();
        assert_eq!(value["state"], "offline");
        assert_eq!(value["pending"], true);
        assert_eq!(value["failures"], 3);
        assert_eq!(value["due"], 130.0);
        assert_eq!(value["snapshot_ready"], false);
        assert_eq!(value["paths"], json!([null]));
    }
    #[test]
    fn successful_run_preserves_unknown_or_remote_followup_work() {
        let value = transition(
            runtime(),
            Outcome::Complete {
                previous: json!({}),
                result: result(vec![]),
            },
            context(),
        )
        .unwrap();
        assert_eq!(value["pending"], true);
        assert_eq!(value["paths"], json!([null]));
        assert_eq!(value["failures"], 0);
        let mut r = runtime();
        r.paths = BTreeSet::from([Some("absent".into())]);
        r.reasons = BTreeSet::from(["remote".into()]);
        let value = transition(
            r,
            Outcome::Complete {
                previous: json!({}),
                result: result(vec![]),
            },
            context(),
        )
        .unwrap();
        assert_eq!(value["pending"], true);
        assert_eq!(value["reasons"], json!(["remote"]));
    }
}
