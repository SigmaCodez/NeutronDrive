//! Decide which jobs a cloud event affects. The host retains locks, I/O and ACKs.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    id: String,
    remote_uid: String,
    account_root_uid: String,
    known_uids: BTreeSet<String>,
    paused: bool,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    node_uid: Option<String>,
    parent_uid: Option<String>,
    root_uid: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    kind: String,
    reset: bool,
    identity: Identity,
}

fn contains(values: &BTreeSet<String>, value: &Option<String>) -> bool {
    value.as_ref().is_some_and(|uid| values.contains(uid))
}

pub fn route(event: Event, jobs: Vec<Job>, pending: Vec<Identity>) -> Result<Value, String> {
    if pending.len() > 1024 {
        return Err("Remote event queue exceeds its safe bound".into());
    }
    let mut ids = BTreeSet::new();
    if jobs
        .iter()
        .any(|job| job.id.is_empty() || !ids.insert(&job.id))
    {
        return Err("Invalid or duplicate job identity in event routing".into());
    }
    if event.kind == "error" {
        return Ok(json!({"offline":true,"affected":[],"reason":"remote",
            "resolve_unknown":false,"append":null,"overflow":[]}));
    }
    let mut affected = Vec::new();
    let mut known_all = BTreeSet::new();
    let mut accounts = BTreeSet::new();
    for job in &jobs {
        let mut known = job.known_uids.clone();
        known.insert(job.remote_uid.clone());
        let relevant = event.reset
            || contains(&known, &event.identity.node_uid)
            || contains(&known, &event.identity.parent_uid)
            || event
                .identity
                .root_uid
                .as_ref()
                .is_some_and(|uid| !uid.is_empty() && uid == &job.remote_uid);
        if relevant && (event.kind != "ready" || event.reset) {
            affected.push(job.id.clone());
        }
        known_all.extend(known.into_iter().filter(|uid| !uid.is_empty()));
        if !job.account_root_uid.is_empty() {
            accounts.insert(job.account_root_uid.clone());
        }
    }
    let unknown_parent = !contains(&known_all, &event.identity.parent_uid)
        && !contains(&accounts, &event.identity.parent_uid);
    let unknown_node = !contains(&known_all, &event.identity.node_uid);
    let resolve_unknown = !event.reset
        && event.kind != "ready"
        && event
            .identity
            .node_uid
            .as_ref()
            .is_some_and(|uid| !uid.is_empty())
        && unknown_parent
        && (unknown_node || !event.kind.ends_with("deleted"))
        && jobs.iter().any(|job| !job.paused);
    let mut append = None;
    let mut overflow = Vec::new();
    if resolve_unknown && !pending.contains(&event.identity) {
        if pending.len() < 1024 {
            append = Some(event.identity);
        } else {
            overflow.extend(jobs.iter().map(|job| job.id.clone()));
        }
    }
    Ok(json!({"offline":false,"affected":affected,
        "reason":if event.reset {"remote-reset"} else {"remote"},
        "resolve_unknown":resolve_unknown,"append":append,"overflow":overflow}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(kind: &str) -> Event {
        serde_json::from_value(json!({"kind":kind,"reset":false,
            "identity":{"node_uid":"file","parent_uid":"missing","root_uid":"account"}}))
        .unwrap()
    }
    fn jobs(paused: bool) -> Vec<Job> {
        serde_json::from_value(json!([{"id":"job","remote_uid":"folder",
            "account_root_uid":"account","known_uids":["file"],"paused":paused}]))
        .unwrap()
    }
    #[test]
    fn known_deletion_does_not_require_querying_deleted_parent() {
        let result = route(event("node_deleted"), jobs(false), vec![]).unwrap();
        assert_eq!(result["affected"], json!(["job"]));
        assert_eq!(result["resolve_unknown"], false);
        assert_eq!(
            route(event("node_updated"), jobs(false), vec![]).unwrap()["resolve_unknown"],
            true
        );
    }
    #[test]
    fn ready_and_paused_jobs_do_not_enqueue_unknown_ancestry() {
        for (kind, paused) in [("ready", false), ("node_updated", true)] {
            assert_eq!(
                route(event(kind), jobs(paused), vec![]).unwrap()["append"],
                Value::Null
            );
        }
    }
}
