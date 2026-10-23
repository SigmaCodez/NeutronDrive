//! Fresh, identity-bound deletion preflight. Folder deletion stays unsupported.
use crate::{
    filesystem,
    host::Services,
    policy,
    process::Failure,
    storage,
    sync_remote::{self, Operation as Remote},
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::{os::unix::fs::MetadataExt, path::Path};
type Result<T> = std::result::Result<T, Failure>;
const FOLDER_BLOCK:&str="Folder deletion is blocked: the public Proton SDK cannot verify all children of a trashed folder. Review this folder's contents manually. Individual files in existing folders remain supported.";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub local_root: String,
    pub remote_root: String,
    pub remote_root_uid: String,
    pub root_identity: [u64; 2],
    pub baseline: Map<String, Value>,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Roots {
        plan: Plan,
        root_fd: i32,
    },
    Remote {
        plan: Plan,
        root: Value,
        relative: String,
    },
    Preflight {
        plan: Plan,
        root_fd: i32,
        relative: String,
        direction: String,
    },
}
pub(crate) fn evaluate(mut value: Value) -> Result<Value> {
    value["version"] = json!(1);
    policy::dispatch(&serde_json::to_vec(&value).map_err(|_| "Invalid sync proof")?)
        .map_err(Failure::from)
}
fn matches(entry: &Value, previous: &Value, side: &str, ancestor: bool) -> Result<bool> {
    if entry.is_null() {
        return Ok(false);
    }
    Ok(evaluate(
        json!({"op":"baseline_matches","entry":entry,"previous":previous,"side":side,"ancestor":ancestor}),
    )? == true)
}
fn parts(relative: &str) -> Result<Vec<&str>> {
    let parts: Vec<_> = relative.split('/').collect();
    if parts.iter().any(|p| !sync_remote::name_error(p).is_empty()) {
        return Err("Unsafe relative path".into());
    }
    Ok(parts)
}
fn under(path: &str, root: &str) -> bool {
    path == root || path.strip_prefix(root).is_some_and(|s| s.starts_with('/'))
}
fn subtree(tree: &Map<String, Value>, relative: &str) -> Map<String, Value> {
    tree.iter()
        .filter(|(p, _)| under(p, relative))
        .map(|(p, v)| (p.clone(), v.clone()))
        .collect()
}
fn file_only(plan: &Plan, relative: &str) -> Result<()> {
    parts(relative)?;
    if plan
        .baseline
        .get(relative)
        .is_some_and(|v| v["kind"] == "folder")
    {
        return Err(FOLDER_BLOCK.into());
    }
    Ok(())
}
fn lookup(root: &Value, relative: &str, host: &mut impl Services) -> Result<Value> {
    sync_remote::run(
        Remote::Lookup {
            root: root.clone(),
            relative: relative.into(),
        },
        host,
    )
}
pub fn roots(plan: &Plan, root_fd: i32, host: &mut impl Services) -> Result<Value> {
    let current = storage::directory(Path::new(&plan.local_root), false, false)?;
    let opened = filesystem::duplicate(root_fd)?;
    for fd in [&current, &opened] {
        let m = fd.metadata().map_err(|e| Failure::from(e.to_string()))?;
        if [m.dev(), m.ino()] != plan.root_identity {
            return Err("Local root was moved or replaced during synchronization".into());
        }
    }
    let root = sync_remote::run(
        Remote::Root {
            path: plan.remote_root.clone(),
        },
        host,
    )?;
    if root["uid"] != plan.remote_root_uid {
        return Err("Remote root/account changed during synchronization".into());
    }
    Ok(root)
}
fn ancestors(plan: &Plan, root: &Value, relative: &str, host: &mut impl Services) -> Result<()> {
    let parts = parts(relative)?;
    for i in 1..parts.len() {
        let parent = parts[..i].join("/");
        let current = lookup(root, &parent, host)?;
        if current["kind"] != "folder"
            || !matches(
                &current,
                plan.baseline.get(&parent).unwrap_or(&Value::Null),
                "remote",
                false,
            )?
        {
            return Err("Remote deletion ancestor disappeared, moved, or was replaced".into());
        }
    }
    Ok(())
}
pub fn remote(plan: &Plan, root: &Value, relative: &str, host: &mut impl Services) -> Result<()> {
    file_only(plan, relative)?;
    ancestors(plan, root, relative, host)?;
    if !lookup(root, relative, host)?.is_null() {
        return Err("Remote deletion source reappeared".into());
    }
    let expected = subtree(&plan.baseline, relative);
    for (path, previous) in &expected {
        let response = host.request("try_info", json!({"path":previous["remote_path"]}))?;
        if response["unavailable"] == true {
            return Err("Remote disappearance is not confirmed trash; it may be a move or unavailable metadata".into());
        }
        let node = &response["node"];
        let entry = sync_remote::entry(node, true)?;
        if !matches(&entry, previous, "remote", false)? {
            return Err("Remote deleted identity was edited or replaced".into());
        }
        let parent = path.rsplit_once('/').map(|(p, _)| p);
        if path == relative {
            if node["raw"]["isTrashed"] != true {
                return Err(
                    "Remote UID is still active elsewhere; a move is not deletion proof".into(),
                );
            }
            let parent_uid = if let Some(p) = parent {
                plan.baseline
                    .get(p)
                    .and_then(|v| v["remote_uid"].as_str())
                    .unwrap_or("")
            } else {
                &plan.remote_root_uid
            };
            if parent_uid.is_empty() || node["raw"]["parentUid"] != parent_uid {
                return Err(
                    "Remote deleted identity moved to a different parent before trash".into(),
                );
            }
        } else if node["raw"]["parentUid"]
            != expected.get(parent.unwrap_or("")).unwrap_or(&Value::Null)["remote_uid"]
        {
            return Err("A descendant of the deleted folder moved elsewhere".into());
        }
    }
    ancestors(plan, root, relative, host)
}
pub(crate) fn preflight(
    plan: &Plan,
    root_fd: i32,
    relative: &str,
    direction: &str,
    host: &mut impl Services,
    after_scan: &mut impl FnMut(),
) -> Result<Value> {
    file_only(plan, relative)?;
    if !matches!(direction, "trash_local" | "trash_remote") {
        return Err("Invalid deletion direction".into());
    }
    let root = roots(plan, root_fd, host)?;
    let local_scan = filesystem::scan(root_fd, &mut |_| if host.poll().is_ok() { 0 } else { 1 })?;
    after_scan();
    let local = local_scan["entries"]
        .as_object()
        .ok_or("Invalid local scan")?;
    let blocked = local_scan["blocked"]
        .as_object()
        .ok_or("Invalid local scan")?;
    if blocked.keys().any(|p| under(relative, p)) {
        return Err(
            "Local absence or target cannot be proved through an unreadable ancestor".into(),
        );
    }
    let mut cloud = Map::new();
    let parts = parts(relative)?;
    for i in 1..parts.len() {
        let parent = parts[..i].join("/");
        let right = lookup(&root, &parent, host)?;
        let left = local.get(&parent).unwrap_or(&Value::Null);
        let previous = plan.baseline.get(&parent).unwrap_or(&Value::Null);
        if left["kind"] != "folder"
            || right["kind"] != "folder"
            || !matches(left, previous, "local", true)?
            || !matches(&right, previous, "remote", false)?
        {
            return Err("Deletion ancestor disappeared, moved, or was replaced".into());
        }
        cloud.insert(parent, right);
    }
    if direction == "trash_local" {
        remote(plan, &root, relative, host)?;
        evaluate(
            json!({"op":"deletion_proof","relative":relative,"target":local,"baseline":plan.baseline,"side":"local"}),
        )?;
        roots(plan, root_fd, host)?;
        ancestors(plan, &root, relative, host)?;
        return Ok(json!(subtree(local, relative)));
    }
    if local.contains_key(relative) {
        return Err("Locally deleted path reappeared".into());
    }
    let previous = subtree(&plan.baseline, relative);
    if local.values().any(|v| {
        previous
            .values()
            .any(|p| v["device"] == p["local_device"] && v["inode"] == p["local_inode"])
    }) {
        return Err("Locally deleted identity moved elsewhere in the sync tree".into());
    }
    let current = lookup(&root, relative, host)?;
    if current.is_null() {
        return Err("Remote trash target disappeared after preview".into());
    }
    let node = host.request("info", json!({"path":current["remote_path"]}))?;
    let parent = relative.rsplit_once('/').map(|(p, _)| p);
    let parent_uid = parent
        .and_then(|p| cloud.get(p))
        .map(|v| &v["uid"])
        .unwrap_or(&root["uid"]);
    if node["raw"]["parentUid"] != *parent_uid {
        return Err("Remote trash target moved to a different parent after preview".into());
    }
    let current = sync_remote::run(
        Remote::NodeEntry {
            node: node.clone(),
            allow_trashed: false,
        },
        host,
    )?;
    let mut target = Map::from_iter([(relative.to_owned(), current.clone())]);
    if current["kind"] == "folder" {
        let children = sync_remote::run(
            Remote::Scan {
                root: node,
                fresh: true,
                allow_trashed: false,
            },
            host,
        )?;
        for (path, value) in children["entries"]
            .as_object()
            .ok_or("Invalid remote scan")?
        {
            target.insert(format!("{relative}/{path}"), value.clone());
        }
    }
    evaluate(
        json!({"op":"deletion_proof","relative":relative,"target":target,"baseline":plan.baseline,"side":"remote"}),
    )?;
    roots(plan, root_fd, host)?;
    ancestors(plan, &root, relative, host)?;
    Ok(json!(target))
}
pub fn run(operation: Operation, host: &mut impl Services) -> Result<Value> {
    match operation {
        Operation::Roots { plan, root_fd } => roots(&plan, root_fd, host),
        Operation::Remote {
            plan,
            root,
            relative,
        } => {
            remote(&plan, &root, &relative, host)?;
            Ok(Value::Null)
        }
        Operation::Preflight {
            plan,
            root_fd,
            relative,
            direction,
        } => preflight(&plan, root_fd, &relative, &direction, host, &mut || {}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        fs::File,
        os::fd::AsRawFd,
        path::PathBuf,
        rc::Rc,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Host {
        ancestor_trashed: Rc<Cell<bool>>,
        file: Value,
        trashed: bool,
    }
    impl Services for Host {
        fn poll(&mut self) -> Result<()> {
            Ok(())
        }
        fn request(&mut self, method: &str, args: Value) -> Result<Value> {
            let path = args["path"].as_str().unwrap();
            let root = json!({"uid":"selected","name":"selected","path":"/my-files/selected","type":"folder","raw":{"parentUid":"ancestor"}});
            if method.starts_with("list_nodes") {
                return Ok(if self.trashed {
                    json!([])
                } else {
                    json!([self.file])
                });
            }
            let value = match path {
                "/my-files" => {
                    json!({"uid":"account","name":"root","path":"/my-files","type":"folder"})
                }
                "/my-files/selected" => root,
                "/my-files/ancestor" => {
                    json!({"uid":"ancestor","name":"ancestor","path":"/my-files/ancestor","type":"folder","raw":{"parentUid":"account","isTrashed":self.ancestor_trashed.get()}})
                }
                "/my-files/file" => self.file.clone(),
                _ => return Err("Unexpected test SDK path".into()),
            };
            if method == "try_info" {
                Ok(json!({"node":value}))
            } else {
                Ok(value)
            }
        }
    }
    struct Tree {
        root: PathBuf,
        fd: File,
        plan: Plan,
        host: Host,
    }
    impl Tree {
        fn new(trashed: bool) -> Self {
            let root = std::env::temp_dir().join(format!(
                "nd-proof-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let fd = storage::directory(&root, true, true).unwrap();
            std::fs::write(root.join("data"), b"valuable").unwrap();
            let scan = filesystem::scan(fd.as_raw_fd(), &mut |_| 0).unwrap();
            let entry = &scan["entries"]["data"];
            let meta = fd.metadata().unwrap();
            let baseline = json!({"kind":"file","identity_version":2,"sha1":entry["sha1"],"size":entry["size"],"local_device":entry["device"],"local_inode":entry["inode"],"local_mtime_ns":entry["mtime_ns"],"local_ctime_ns":entry["ctime_ns"],"remote_uid":"file","remote_revision":"rev1","remote_path":"/my-files/file"});
            let file = json!({"name":"data","uid":"file","type":"file","path":"/my-files/file","sha1":entry["sha1"],"size":entry["size"],"revision":"rev1","raw":{"parentUid":"selected","isTrashed":trashed}});
            let plan = Plan {
                local_root: root.to_str().unwrap().into(),
                remote_root: "/my-files/selected".into(),
                remote_root_uid: "selected".into(),
                root_identity: [meta.dev(), meta.ino()],
                baseline: Map::from_iter([("data".into(), baseline)]),
            };
            let host = Host {
                ancestor_trashed: Rc::new(Cell::new(false)),
                file,
                trashed,
            };
            if !trashed {
                std::fs::remove_file(root.join("data")).unwrap();
            }
            Self {
                root,
                fd,
                plan,
                host,
            }
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn ancestor_trashed_after_local_scan_blocks_local_deletion() {
        let mut tree = Tree::new(true);
        let state = tree.host.ancestor_trashed.clone();
        let result = preflight(
            &tree.plan,
            tree.fd.as_raw_fd(),
            "data",
            "trash_local",
            &mut tree.host,
            &mut || state.set(true),
        );
        assert!(result.unwrap_err().message.contains("Trashed"));
        assert_eq!(std::fs::read(tree.root.join("data")).unwrap(), b"valuable");
    }
    #[test]
    fn ancestor_trashed_in_second_preflight_blocks_remote_deletion() {
        let mut tree = Tree::new(false);
        let state = tree.host.ancestor_trashed.clone();
        assert!(preflight(
            &tree.plan,
            tree.fd.as_raw_fd(),
            "data",
            "trash_remote",
            &mut tree.host,
            &mut || {}
        )
        .is_ok());
        let result = preflight(
            &tree.plan,
            tree.fd.as_raw_fd(),
            "data",
            "trash_remote",
            &mut tree.host,
            &mut || state.set(true),
        );
        assert!(result.unwrap_err().message.contains("Trashed"));
        assert!(!tree.host.trashed);
    }
    #[test]
    fn moved_remote_uid_is_not_a_deletion_proof() {
        let mut tree = Tree::new(true);
        tree.host.file["raw"]["parentUid"] = json!("another-parent");
        let root = roots(&tree.plan, tree.fd.as_raw_fd(), &mut tree.host).unwrap();
        assert!(remote(&tree.plan, &root, "data", &mut tree.host)
            .unwrap_err()
            .message
            .contains("different parent"));
        tree.host.file["raw"]["parentUid"] = json!("selected");
        tree.host.file["revision"] = json!("changed-revision");
        assert!(remote(&tree.plan, &root, "data", &mut tree.host)
            .unwrap_err()
            .message
            .contains("edited or replaced"));
    }
}
