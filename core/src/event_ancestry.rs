//! Resolve unknown event parents outside the durable cursor-acknowledgement path.
use crate::{host::Services, process::Failure};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
type Result<T> = std::result::Result<T, Failure>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    id: String,
    paused: bool,
    remote_uid: String,
    account_root_uid: String,
    known_uids: BTreeSet<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    root_uid: Option<String>,
    node_uid: Option<String>,
    parent_uid: Option<String>,
}
pub fn resolve(event: Event, jobs: Vec<Job>, host: &mut impl Services) -> Result<Value> {
    let active: Vec<_> = jobs.into_iter().filter(|j| !j.paused).collect();
    let all: Vec<_> = active.iter().map(|j| j.id.clone()).collect();
    let accounts: BTreeSet<_> = active
        .iter()
        .map(|j| j.account_root_uid.clone())
        .filter(|s| !s.is_empty())
        .collect();
    let mut known = BTreeMap::new();
    for job in &active {
        for uid in job
            .known_uids
            .iter()
            .chain(std::iter::once(&job.remote_uid))
            .filter(|s| !s.is_empty())
        {
            known.insert(uid.clone(), job.id.clone());
        }
    }
    let outcome = (|| -> Result<Value> {
        if event
            .root_uid
            .as_ref()
            .is_some_and(|v| !v.is_empty() && !accounts.is_empty() && !accounts.contains(v))
        {
            return Err("Account identity changed in event stream".into());
        }
        let mut current = event.parent_uid.unwrap_or_default();
        if current.is_empty() {
            let uid = event
                .node_uid
                .as_deref()
                .filter(|v| !v.is_empty())
                .ok_or("Unknown event node")?;
            let node = host.request("info", json!({"path":format!("/my-files/{uid}")}))?;
            if node["safe"] != true || node["uid"] != uid {
                return Err("Unsafe event node".into());
            }
            current = node["raw"]["parentUid"].as_str().unwrap_or("").into();
        }
        let mut visited = BTreeSet::new();
        for _ in 0..128 {
            if let Some(id) = known.get(&current) {
                return Ok(json!({"affected":[id],"ancestor":current,"cancelled":false}));
            }
            if accounts.contains(&current) {
                return Ok(json!({"affected":[],"ancestor":null,"cancelled":false}));
            }
            if current.is_empty() || !visited.insert(current.clone()) {
                return Err("Unknown event ancestry".into());
            }
            let node = host.request("info", json!({"path":format!("/my-files/{current}")}))?;
            if node["safe"] != true || node["uid"] != current {
                return Err("Unsafe event ancestry".into());
            }
            current = node["raw"]["parentUid"].as_str().unwrap_or("").into();
        }
        Err("Event ancestry exceeds maximum depth".into())
    })();
    // Missing ancestors and offline metadata require one conservative reconciliation.
    Ok(match outcome {
        Ok(v) => v,
        Err(error) => json!({"affected":all,"ancestor":null,"cancelled":error.cancelled}),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Host {
        offline: bool,
        calls: usize,
    }
    impl Services for Host {
        fn poll(&mut self) -> Result<()> {
            Ok(())
        }
        fn request(&mut self, _: &str, args: Value) -> Result<Value> {
            self.calls += 1;
            if self.offline {
                return Err("offline".into());
            }
            let uid = args["path"].as_str().unwrap().rsplit('/').next().unwrap();
            Ok(json!({"safe":true,"uid":uid,"raw":{"parentUid":"selected"}}))
        }
    }
    fn jobs() -> Vec<Job> {
        vec![
            Job {
                id: "active".into(),
                paused: false,
                remote_uid: "selected".into(),
                account_root_uid: "account".into(),
                known_uids: BTreeSet::new(),
            },
            Job {
                id: "paused".into(),
                paused: true,
                remote_uid: "paused-root".into(),
                account_root_uid: "account".into(),
                known_uids: BTreeSet::new(),
            },
        ]
    }
    fn event(parent: &str) -> Event {
        Event {
            root_uid: Some("account".into()),
            node_uid: Some("node".into()),
            parent_uid: Some(parent.into()),
        }
    }
    #[test]
    fn resolves_ancestry_and_avoids_account_wide_sync_for_known_outside() {
        let mut host = Host {
            offline: false,
            calls: 0,
        };
        let result = resolve(event("unknown"), jobs(), &mut host).unwrap();
        assert_eq!(result["affected"], json!(["active"]));
        assert_eq!(result["ancestor"], "selected");
        assert_eq!(host.calls, 1);
        assert_eq!(
            resolve(event("account"), jobs(), &mut host).unwrap()["affected"],
            json!([])
        );
        assert_eq!(host.calls, 1);
    }
    #[test]
    fn unavailable_or_different_account_falls_back_only_to_active_jobs() {
        let mut host = Host {
            offline: true,
            calls: 0,
        };
        let result = resolve(event("deleted-parent"), jobs(), &mut host).unwrap();
        assert_eq!(result["affected"], json!(["active"]));
        assert!(result["ancestor"].is_null());
        let mut changed = event("unknown");
        changed.root_uid = Some("other-account".into());
        assert_eq!(
            resolve(changed, jobs(), &mut host).unwrap()["affected"],
            json!(["active"])
        );
        assert_eq!(host.calls, 1);
    }
}
