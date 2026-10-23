//! Complete preview preparation: disk identity, baseline, scans and cloud proofs.
use crate::{
    filesystem,
    host::Services,
    process::Failure,
    storage, sync_proof,
    sync_remote::{self, Operation as Remote},
    sync_state::{self, Operation as State},
};
use serde_json::{json, Value};
use std::{
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::Path,
};
type Result<T> = std::result::Result<T, Failure>;
pub fn run(state: &str, root: &str, remote_path: &str, host: &mut impl Services) -> Result<Value> {
    host.poll()?;
    if Path::new(state).starts_with(Path::new(root)) {
        return Err(
            "The private sync state directory must be outside the synchronized tree".into(),
        );
    }
    sync_state::run(State::Prepare {
        state: state.into(),
    })?;
    let remote = sync_remote::run(
        Remote::Root {
            path: remote_path.into(),
        },
        host,
    )?;
    let uid = remote["uid"]
        .as_str()
        .ok_or("Missing remote root identity")?;
    let key = sync_state::run(State::Key {
        root: root.into(),
        uid: uid.into(),
    })?
    .as_str()
    .ok_or("Invalid state key")?
    .to_owned();
    let baseline = sync_state::run(State::Load {
        state: state.into(),
        key: key.clone(),
        root: root.into(),
        uid: uid.into(),
    })?;
    let fd = storage::directory(Path::new(root), false, false)?;
    let info = fd.metadata().map_err(|e| Failure::from(e.to_string()))?;
    let identity = [info.dev(), info.ino()];
    if !baseline["identity"].is_null() && baseline["identity"] != json!(identity) {
        return Err("Local sync root identity changed since the saved baseline".into());
    }
    let local = filesystem::scan(fd.as_raw_fd(), &mut |_| {
        if host.poll().is_ok() {
            0
        } else {
            1
        }
    })?;
    let cloud = sync_remote::run(
        Remote::Scan {
            root: remote.clone(),
            fresh: false,
            allow_trashed: false,
        },
        host,
    )?;
    let mut blocked = local["blocked"]
        .as_object()
        .ok_or("Invalid local scan")?
        .clone();
    blocked.extend(
        cloud["blocked"]
            .as_object()
            .ok_or("Invalid remote scan")?
            .clone(),
    );
    let skipped: Vec<_> = blocked
        .iter()
        .map(|(path, reason)| {
            format!(
                "{path}: {}",
                reason.as_str().unwrap_or("Invalid scan result")
            )
        })
        .collect();
    let mut request = json!({"op":"plan","local":local["entries"],"remote":cloud["entries"],"baseline":baseline["entries"],"pending":baseline["pending"],"blocked":blocked,"proofs":{}});
    let mut decisions = sync_proof::evaluate(request.clone())?;
    let plan = sync_proof::Plan {
        local_root: root.into(),
        remote_root: remote["path"]
            .as_str()
            .ok_or("Missing remote root path")?
            .into(),
        remote_root_uid: uid.into(),
        root_identity: identity,
        baseline: baseline["entries"]
            .as_object()
            .ok_or("Invalid baseline")?
            .clone(),
    };
    let checks = decisions["checks"]
        .as_array()
        .ok_or("Invalid deletion proof requests")?
        .clone();
    for relative in &checks {
        let relative = relative.as_str().ok_or("Invalid deletion proof path")?;
        request["proofs"][relative] = match sync_proof::remote(&plan, &remote, relative, host) {
            Ok(()) => Value::Null,
            Err(e) if e.cancelled => return Err(e),
            Err(e) => json!(e.message),
        };
    }
    if !checks.is_empty() {
        decisions = sync_proof::evaluate(request)?
    }
    if !decisions["checks"].as_array().is_some_and(Vec::is_empty) {
        return Err("Rust planner requires unresolved deletion proofs".into());
    }
    Ok(
        json!({"local_root":root,"remote_root":remote["path"],"remote_root_uid":uid,"actions":decisions["actions"],"conflicts":decisions["conflicts"],"skipped":skipped,"_local":local["entries"],"_remote":cloud["entries"],"_baseline":baseline["entries"],"_pending_deletions":baseline["pending"],"_state_digest":baseline["digest"],"_state_key":key,"_root_identity":identity,"_blocked":decisions["blocked"]}),
    )
}
