use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

type Tree = BTreeMap<String, Entry>;
type Baseline = BTreeMap<String, Value>;
type Paths = BTreeSet<String>;
const FOLDER_BLOCK: &str = "Folder deletion is blocked: the public Proton SDK cannot verify all children of a trashed folder. Review this folder's contents manually. Individual files in existing folders remain supported.";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    kind: Kind,
    #[serde(default)]
    sha1: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    uid: String,
    #[serde(default)]
    revision: String,
    #[serde(default)]
    remote_path: String,
    #[serde(default)]
    mtime_ns: i64,
    #[serde(default)]
    inode: u64,
    #[serde(default)]
    device: u64,
    #[serde(default)]
    ctime_ns: i64,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    File,
    Folder,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    direction: Side,
    operation_id: String,
}
#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Side {
    Local,
    Remote,
}
#[derive(Deserialize)]
struct Request {
    version: u32,
    #[serde(flatten)]
    command: Command,
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Info,
    InotifyDecode {
        data: Vec<u8>,
    },
    ValidateEvents {
        value: Value,
        paths: Vec<String>,
    },
    EventNotifications {
        batch: Value,
        known_roots: Vec<Value>,
        announced: bool,
        ready: bool,
    },
    EventRetry {
        interval: f64,
        failures: u64,
        jitter: f64,
    },
    Plan {
        local: Tree,
        remote: Tree,
        baseline: Baseline,
        pending: BTreeMap<String, Pending>,
        blocked: BTreeMap<String, String>,
        proofs: BTreeMap<String, Option<String>>,
    },
    BaselineMatches {
        entry: Entry,
        previous: Value,
        side: Side,
        ancestor: bool,
    },
    DeletionProof {
        relative: String,
        target: Tree,
        baseline: Baseline,
        side: Side,
    },
    Convergence {
        local: Tree,
        remote: Tree,
        actions: Vec<Action>,
        failed: Paths,
        blocked: Paths,
    },
    Baseline {
        local: Tree,
        remote: Tree,
        baseline: Baseline,
        blocked: Paths,
        pending: Paths,
        absent: Paths,
    },
    Schedule {
        state: Schedule,
        reason: String,
        relative: Option<String>,
        immediate: bool,
        now: f64,
        debounce: f64,
    },
    Retry {
        failures: u64,
        now: f64,
        base: f64,
        maximum: f64,
        jitter: f64,
    },
    Select {
        jobs: Vec<Candidate>,
        now: f64,
    },
    Echo {
        previous: Tree,
        current: Tree,
        remote: Tree,
        trashed: Vec<String>,
    },
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Action {
    kind: String,
    relative_path: String,
    #[serde(default)]
    detail: String,
}
#[derive(Serialize)]
struct Conflict {
    relative_path: String,
    reason: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Schedule {
    paused: bool,
    state: String,
    pending: bool,
    due: f64,
    first_dirty: f64,
    paths: BTreeSet<Option<String>>,
    reasons: BTreeSet<String>,
    rebuild_watch: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    id: String,
    pending: bool,
    paused: bool,
    state: String,
    due: f64,
}

fn under(path: &str, root: &str) -> bool {
    path == root || path.strip_prefix(root).is_some_and(|s| s.starts_with('/'))
}
fn blocked(path: &str, paths: &Paths) -> bool {
    paths.iter().any(|root| under(path, root))
}
fn paths<'a>(trees: &[&'a Tree], baseline: Option<&'a Baseline>) -> Paths {
    trees
        .iter()
        .flat_map(|tree| tree.keys())
        .chain(baseline.into_iter().flat_map(|tree| tree.keys()))
        .cloned()
        .collect()
}
fn str_field<'a>(v: &'a Value, name: &str) -> &'a str {
    v.get(name).and_then(Value::as_str).unwrap_or("")
}
fn matches(entry: &Entry, previous: &Value, side: Side, ancestor: bool) -> bool {
    if previous.get("identity_version") != Some(&json!(2))
        || previous.get("kind") != Some(&json!(entry.kind))
    {
        return false;
    }
    if entry.kind == Kind::File
        && (previous.get("sha1") != Some(&json!(entry.sha1))
            || previous.get("size") != Some(&json!(entry.size)))
    {
        return false;
    }
    match side {
        Side::Local => {
            previous.get("local_device") == Some(&json!(entry.device))
                && previous.get("local_inode") == Some(&json!(entry.inode))
                && ((entry.kind == Kind::Folder && ancestor)
                    || (previous.get("local_mtime_ns") == Some(&json!(entry.mtime_ns))
                        && previous.get("local_ctime_ns") == Some(&json!(entry.ctime_ns))))
        }
        Side::Remote => {
            previous.get("remote_uid") == Some(&json!(entry.uid))
                && previous.get("remote_revision") == Some(&json!(entry.revision))
                && (entry.kind == Kind::Folder || !entry.revision.is_empty())
        }
    }
}
fn deletion_proof(
    relative: &str,
    target: &Tree,
    baseline: &Baseline,
    side: Side,
) -> Result<(), String> {
    let previous = baseline.get(relative).unwrap_or(&Value::Null);
    if str_field(previous, "kind") == "folder" {
        return Err(FOLDER_BLOCK.into());
    }
    if !target
        .get(relative)
        .is_some_and(|entry| matches(entry, previous, side, false))
    {
        return Err(
            "Deletion conflicts with an edit, replacement, or unverified old baseline".into(),
        );
    }
    Ok(())
}
fn ancestors(
    relative: &str,
    local: &Tree,
    remote: &Tree,
    baseline: &Baseline,
) -> Result<(), String> {
    for (index, _) in relative.match_indices('/') {
        let path = &relative[..index];
        let previous = baseline.get(path).unwrap_or(&Value::Null);
        if !local
            .get(path)
            .is_some_and(|e| e.kind == Kind::Folder && matches(e, previous, Side::Local, true))
            || !remote
                .get(path)
                .is_some_and(|e| e.kind == Kind::Folder && matches(e, previous, Side::Remote, true))
        {
            return Err("Deletion ancestor disappeared, moved, or was replaced".into());
        }
    }
    Ok(())
}
fn validate_tree(tree: &Tree) -> Result<(), String> {
    for path in tree.keys() {
        validate_path(path)?;
    }
    Ok(())
}
fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.split('/').any(|p| {
            p.is_empty()
                || p == "."
                || p == ".."
                || p.starts_with(".neutrondrive-")
                || p.chars()
                    .any(|c| c < ' ' || c == '\u{7f}' || "\\*?[{".contains(c))
        })
    {
        return Err("Invalid path in Rust core request".into());
    }
    Ok(())
}
fn validate_baseline(baseline: &Baseline) -> Result<(), String> {
    for (path, entry) in baseline {
        validate_path(path)?;
        if !entry.is_object() {
            return Err("Invalid baseline entry".into());
        }
    }
    Ok(())
}
fn finite(values: &[f64]) -> Result<(), String> {
    if values.iter().any(|v| !v.is_finite() || *v < 0.0) {
        Err("Invalid daemon timing value".into())
    } else {
        Ok(())
    }
}

pub fn dispatch(input: &[u8]) -> Result<Value, String> {
    let request: Request = serde_json::from_slice(input)
        .map_err(|_| "Invalid Rust core protocol request".to_string())?;
    if request.version != 1 {
        return Err("Unsupported Rust core protocol version".into());
    }
    match request.command {
        Command::InotifyDecode { data } => crate::inotify::decode(&data),
        Command::ValidateEvents { value, paths } => crate::events::validate(value, &paths),
        Command::EventNotifications {
            batch,
            known_roots,
            announced,
            ready,
        } => crate::events::notifications(batch, known_roots, announced, ready),
        Command::EventRetry {
            interval,
            failures,
            jitter,
        } => {
            finite(&[interval, jitter])?;
            if interval < 30.0 || jitter >= 1.0 {
                return Err("Invalid event retry timing".into());
            }
            Ok(
                json!({"failures":failures.saturating_add(1),"delay":(interval * 2f64.powi(failures.min(5) as i32)).min(600.0) + jitter}),
            )
        }
        Command::Info => Ok(
            json!({"protocol":1,"version":env!("CARGO_PKG_VERSION"),"implementation":"rust","components":["sync-planner","baseline","deletion-policy","daemon-scheduler","echo-policy","event-validation","event-delivery","event-retry","inotify-decoder"]}),
        ),
        Command::BaselineMatches {
            entry,
            previous,
            side,
            ancestor,
        } => Ok(json!(matches(&entry, &previous, side, ancestor))),
        Command::DeletionProof {
            relative,
            target,
            baseline,
            side,
        } => {
            validate_path(&relative)?;
            validate_tree(&target)?;
            validate_baseline(&baseline)?;
            deletion_proof(&relative, &target, &baseline, side)?;
            Ok(Value::Null)
        }
        Command::Plan {
            local,
            remote,
            baseline,
            pending,
            blocked: initial,
            proofs,
        } => {
            validate_tree(&local)?;
            validate_tree(&remote)?;
            validate_baseline(&baseline)?;
            for (path, intent) in &pending {
                validate_path(path)?;
                if intent.operation_id.len() != 32
                    || !intent.operation_id.bytes().all(|c| c.is_ascii_hexdigit())
                {
                    return Err("Invalid pending operation ID".into());
                }
            }
            let mut blocked: Paths = initial.keys().cloned().collect();
            let mut conflicts = Vec::new();
            let mut actions = Vec::new();
            let mut checks = Vec::new();
            for (old, previous) in &baseline {
                let mut moved = Paths::new();
                if !local.contains_key(old) && previous.get("identity_version") == Some(&json!(2)) {
                    moved.extend(
                        local
                            .iter()
                            .filter(|(_, e)| {
                                previous.get("local_device") == Some(&json!(e.device))
                                    && previous.get("local_inode") == Some(&json!(e.inode))
                            })
                            .map(|(p, _)| p.clone()),
                    );
                }
                if !remote.contains_key(old) && !str_field(previous, "remote_uid").is_empty() {
                    moved.extend(
                        remote
                            .iter()
                            .filter(|(_, e)| e.uid == str_field(previous, "remote_uid"))
                            .map(|(p, _)| p.clone()),
                    );
                }
                if !moved.is_empty() {
                    conflicts.push(Conflict {
                        relative_path: old.clone(),
                        reason: "A baseline identity moved to another path; resolve the move first"
                            .into(),
                    });
                    blocked.insert(old.clone());
                    blocked.extend(moved);
                }
            }
            let mut ordered: Vec<_> = paths(&[&local, &remote], Some(&baseline))
                .into_iter()
                .collect();
            ordered.sort_by_key(|p| (p.matches('/').count(), p.clone()));
            let mut covered = Paths::new();
            for path in ordered {
                if blocked_fn(&path, &blocked) || blocked_fn(&path, &covered) {
                    continue;
                }
                let left = local.get(&path);
                let right = remote.get(&path);
                let previous = baseline.get(&path);
                if left.is_none() && right.is_none() {
                    continue;
                }
                let uncertain = pending
                    .iter()
                    .find(|(root, _)| under(&path, root))
                    .is_some_and(|(_, intent)| {
                        (left.is_some() && right.is_some())
                            || intent.direction
                                != if left.is_some() {
                                    Side::Local
                                } else {
                                    Side::Remote
                                }
                    });
                let mut reason = None;
                let mut action = None;
                if uncertain {
                    reason=Some("Earlier trash operation has an uncertain outcome; retained deletion intent requires review".to_string());
                } else if let (Some(l), Some(r)) = (left, right) {
                    if l.kind != r.kind {
                        reason = Some("File/folder type conflict".into());
                    } else if l.kind == Kind::Folder || l.sha1 == r.sha1 {
                        continue;
                    } else if !previous.is_some_and(|p| str_field(p, "kind") == "file") {
                        reason = Some(
                            "Both sides already exist with different contents; no common baseline"
                                .into(),
                        );
                    } else if l.sha1 == str_field(previous.unwrap(), "sha1") {
                        action = Some(("download", "Remote changed; retain a local backup"));
                    } else if r.sha1 == str_field(previous.unwrap(), "sha1") {
                        action = Some(("upload", "Local changed; create a remote revision"));
                    } else {
                        reason = Some(
                            "Both sides changed since the last successful synchronization".into(),
                        );
                    }
                } else if previous.is_some_and(|p| p.as_object().is_some_and(|o| !o.is_empty())) {
                    let side = if left.is_some() {
                        Side::Local
                    } else {
                        Side::Remote
                    };
                    let proof = ancestors(&path, &local, &remote, &baseline).and_then(|_| {
                        deletion_proof(
                            &path,
                            if left.is_some() { &local } else { &remote },
                            &baseline,
                            side,
                        )
                    });
                    match proof {
                        Err(error) => reason = Some(error),
                        Ok(()) => {
                            if left.is_some() {
                                match proofs.get(&path) {
                                    None => {
                                        checks.push(path.clone());
                                        continue;
                                    }
                                    Some(Some(error)) => reason = Some(error.clone()),
                                    Some(None) => {}
                                }
                            }
                            if reason.is_none() {
                                action = Some((
                                    if left.is_some() {
                                        "trash_local"
                                    } else {
                                        "trash_remote"
                                    },
                                    "Propagate verified deletion to recoverable trash",
                                ));
                                covered.insert(path.clone());
                            }
                        }
                    }
                } else if let Some(l) = left {
                    action = Some((
                        if l.kind == Kind::Folder {
                            "mkdir_remote"
                        } else {
                            "upload"
                        },
                        "",
                    ));
                } else if let Some(r) = right {
                    action = Some((
                        if r.kind == Kind::Folder {
                            "mkdir_local"
                        } else {
                            "download"
                        },
                        "",
                    ));
                }
                if let Some((kind, detail)) = action {
                    actions.push(Action {
                        kind: kind.into(),
                        relative_path: path.clone(),
                        detail: detail.into(),
                    });
                }
                if let Some(reason) = reason {
                    conflicts.push(Conflict {
                        relative_path: path.clone(),
                        reason,
                    });
                    blocked.insert(path);
                }
            }
            Ok(json!({"actions":actions,"conflicts":conflicts,"blocked":blocked,"checks":checks}))
        }
        Command::Convergence {
            local,
            remote,
            actions,
            failed,
            mut blocked,
        } => {
            validate_tree(&local)?;
            validate_tree(&remote)?;
            let mut conflicts = Vec::new();
            for action in actions {
                let path = action.relative_path;
                if blocked_fn(&path, &failed) {
                    continue;
                }
                let l = local.get(&path);
                let r = remote.get(&path);
                if ["trash_local", "trash_remote"].contains(&action.kind.as_str())
                    && l.is_none()
                    && r.is_none()
                {
                    continue;
                }
                if blocked_fn(&path, &blocked)
                    || !l.zip(r).is_some_and(|(a, b)| {
                        a.kind == b.kind && (a.kind == Kind::Folder || a.sha1 == b.sha1)
                    })
                {
                    conflicts.push(format!(
                        "{path}: content changed during sync; convergence was not confirmed"
                    ));
                    blocked.insert(path);
                }
            }
            Ok(json!({"conflicts":conflicts,"blocked":blocked}))
        }
        Command::Baseline {
            local,
            remote,
            mut baseline,
            blocked,
            pending,
            absent,
        } => {
            validate_tree(&local)?;
            validate_tree(&remote)?;
            validate_baseline(&baseline)?;
            let blocked: Paths = blocked.union(&pending).cloned().collect();
            let mut checks = Vec::new();
            for path in paths(&[&local, &remote], Some(&baseline)) {
                if blocked_fn(&path, &blocked) {
                    continue;
                }
                match (local.get(&path), remote.get(&path)) {
                    (None, None) => {
                        if absent.contains(&path) {
                            baseline.remove(&path);
                        } else {
                            checks.push(path);
                        }
                    }
                    (Some(l), Some(r))
                        if l.kind == r.kind && (l.kind == Kind::Folder || l.sha1 == r.sha1) =>
                    {
                        if l.kind == Kind::Folder && blocked.iter().any(|p| under(p, &path)) {
                            continue;
                        }
                        let left: Paths =
                            local.keys().filter(|p| under(p, &path)).cloned().collect();
                        let right: Paths =
                            remote.keys().filter(|p| under(p, &path)).cloned().collect();
                        let complete = left == right
                            && left.iter().all(|p| {
                                local[p].kind == remote[p].kind
                                    && (local[p].kind == Kind::Folder
                                        || local[p].sha1 == remote[p].sha1)
                            });
                        baseline.insert(path,json!({"identity_version":2,"kind":l.kind,"sha1":l.sha1,"size":l.size,"local_device":l.device,"local_inode":l.inode,"local_mtime_ns":l.mtime_ns,"local_ctime_ns":l.ctime_ns,"remote_uid":r.uid,"remote_revision":r.revision,"remote_path":r.remote_path,"subtree_complete":complete}));
                    }
                    _ => {}
                }
            }
            Ok(json!({"baseline":baseline,"checks":checks}))
        }
        Command::Schedule {
            mut state,
            reason,
            relative,
            immediate,
            now,
            debounce,
        } => {
            finite(&[state.due, state.first_dirty, now, debounce])?;
            if !state.paused && state.state != "blocked" {
                if !state.pending {
                    state.first_dirty = now;
                }
                state.pending = true;
                state.reasons.insert(reason.clone());
                if reason.starts_with("local")
                    || reason.starts_with("watch")
                    || reason == "inotify-overflow"
                {
                    if state.paths.len() < 1024 {
                        state.paths.insert(relative);
                    } else {
                        state.paths = BTreeSet::from([None]);
                    }
                }
                if ["watch-rebuild", "watch-error", "inotify-overflow"].contains(&reason.as_str()) {
                    state.rebuild_watch = true;
                }
                if state.state != "offline" || immediate {
                    state.due = if immediate {
                        now
                    } else {
                        (now + debounce).min(
                            (if state.first_dirty != 0.0 {
                                state.first_dirty
                            } else {
                                now
                            }) + 10.0,
                        )
                    };
                }
            }
            Ok(json!(state))
        }
        Command::Retry {
            failures,
            now,
            base,
            maximum,
            jitter,
        } => {
            finite(&[now, base, maximum, jitter])?;
            if !(0.9..=1.1).contains(&jitter) {
                return Err("Invalid retry jitter".into());
            }
            let delay =
                (maximum.min(base * 2f64.powi(failures.min(10) as i32)) * jitter).min(maximum);
            Ok(json!({"failures":failures.saturating_add(1),"due":now+delay}))
        }
        Command::Select { jobs, now } => {
            finite(&[now])?;
            for j in &jobs {
                finite(&[j.due])?;
            }
            let next = jobs
                .iter()
                .filter(|j| j.pending && !j.paused && j.state != "blocked")
                .min_by(|a, b| a.due.total_cmp(&b.due));
            Ok(match next {
                None => Value::Null,
                Some(j) => json!({"id":j.id,"delay":(j.due-now).max(0.0)}),
            })
        }
        Command::Echo {
            previous,
            current,
            remote,
            trashed,
        } => {
            validate_tree(&previous)?;
            validate_tree(&current)?;
            validate_tree(&remote)?;
            let mut result = Tree::new();
            for (path, local) in &current {
                let converged = remote.get(path).is_some_and(|r| {
                    local.kind == r.kind && (local.kind == Kind::Folder || local.sha1 == r.sha1)
                });
                let unchanged = previous.get(path).is_some_and(|p| {
                    let mut a = local.clone();
                    let mut b = p.clone();
                    a.ctime_ns = 0;
                    b.ctime_ns = 0;
                    a == b
                });
                if converged || unchanged {
                    result.insert(path.clone(), local.clone());
                }
            }
            let mut old = previous;
            old.extend(remote);
            for (path, entry) in old {
                if !current.contains_key(&path) && !trashed.iter().any(|root| under(&path, root)) {
                    result.insert(path, entry);
                }
            }
            Ok(json!(result))
        }
    }
}
fn blocked_fn(path: &str, roots: &Paths) -> bool {
    blocked(path, roots)
}
