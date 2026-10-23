//! Cloud namespace changes with native tree reconciliation and revision guards.
use crate::{
    host::Services,
    ondemand_io::{fail, field, revision, Error, Result},
    ondemand_replace,
    ondemand_tree::{self as tree, Entry, Tree},
    process::Failure,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Mkdir {
        parent: u64,
        name: String,
    },
    Rename {
        source: u64,
        parent: u64,
        name: String,
        no_replace: bool,
    },
    Unlink {
        source: u64,
        directory: bool,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    cache: PathBuf,
    objects: PathBuf,
    recovery: PathBuf,
    staging: PathBuf,
    remote_root: String,
    tree: Tree,
    open_inodes: BTreeSet<u64>,
    observed: BTreeMap<String, Value>,
    operation: Action,
}
struct Context {
    request: Request,
    refreshed: BTreeSet<u64>,
    committed: Option<Value>,
}
impl Context {
    fn entry(&self, inode: u64) -> Result<Entry> {
        self.request
            .tree
            .entries
            .iter()
            .find(|e| e.inode == inode)
            .cloned()
            .ok_or_else(|| fail(libc::ENOENT, "Entry no longer exists"))
    }
    fn check_root(&self, host: &mut impl Services) -> Result<()> {
        let root = host.request("info", json!({"path":self.request.remote_root}))?;
        if root["uid"] != self.entry(1)?.node["uid"] || root["type"] != "folder" {
            return Err(fail(libc::EACCES,"Cloud account or mount root changed. Unmount before reconnecting; unsaved content remains in recovery."));
        }
        Ok(())
    }
    fn save(&self) -> Result<()> {
        tree::save(
            &self.request.cache.join("metadata.json"),
            &self.request.remote_root,
            self.request.tree.clone(),
        )
    }
    fn refresh(&mut self, inode: u64, host: &mut impl Services) -> Result<usize> {
        let parent = self.entry(inode)?;
        if parent.node["type"] != "folder" {
            return Err(fail(libc::ENOTDIR, "Not a directory"));
        }
        self.check_root(host)?;
        let nodes = host.request("list_nodes", json!({"path":parent.node["path"]}))?;
        let nodes = nodes
            .as_array()
            .ok_or_else(|| fail(libc::EIO, "Invalid cloud directory response"))?
            .clone();
        let count = nodes.len();
        self.request.tree = tree::reconcile(
            self.request.tree.clone(),
            inode,
            nodes,
            self.request.open_inodes.clone(),
        )?;
        self.refreshed.insert(inode);
        self.save()?;
        Ok(count)
    }
    fn child(&self, parent: u64, name: &str) -> Option<u64> {
        self.request
            .tree
            .children
            .get(&parent)
            .and_then(|children| children.get(name))
            .copied()
    }
    fn not_open(&self, inode: u64) -> Result<()> {
        if self.request.open_inodes.contains(&inode) {
            return Err(fail(
                libc::EBUSY,
                "Close this file before changing its cloud name or deleting it",
            ));
        }
        Ok(())
    }
    fn execute(&mut self, host: &mut impl Services) -> Result<Value> {
        match &self.request.operation {
            Action::Mkdir { parent, name } => {
                let (parent, name) = (*parent, name.clone());
                tree::name(&name)?;
                self.refresh(parent, host)?;
                if self.child(parent, &name).is_some() {
                    return Err(fail(libc::EEXIST, "Destination already exists"));
                }
                let node = host.request(
                    "mkdir",
                    json!({"parent":self.entry(parent)?.node["path"],"name":name}),
                )?;
                tree::node(&node)?;
                self.refresh(parent, host)?;
                Ok(json!(self
                    .child(parent, &name)
                    .ok_or_else(|| fail(libc::ENOENT, "No such file"))?))
            }
            Action::Rename {
                source,
                parent,
                name,
                no_replace,
            } => {
                let (source, parent, name, no_replace) =
                    (self.entry(*source)?, *parent, name.clone(), *no_replace);
                tree::name(&name)?;
                self.not_open(source.inode)?;
                if source.parent == parent && source.node["name"] == name {
                    return Ok(Value::Null);
                }
                let expected = self
                    .child(parent, &name)
                    .map(|id| self.entry(id))
                    .transpose()?
                    .map(|e| e.node);
                self.refresh(parent, host)?;
                if let Some(id) = self.child(parent, &name) {
                    if no_replace {
                        return Err(fail(libc::EEXIST, "Destination already exists"));
                    }
                    let target = self.entry(id)?;
                    if expected
                        .as_ref()
                        .is_some_and(|e| revision(e) != revision(&target.node))
                    {
                        return Err(fail(
                            libc::ESTALE,
                            "Destination changed before replacement; source retained",
                        ));
                    }
                    let target_revision = revision(&target.node);
                    let mut replacement = ondemand_replace::Request {
                        cache: self.request.cache.clone(),
                        objects: self.request.objects.clone(),
                        recovery: self.request.recovery.clone(),
                        staging: self.request.staging.clone(),
                        remote_root: self.request.remote_root.clone(),
                        tree: std::mem::take(&mut self.request.tree),
                        source: source.inode,
                        target: id,
                        expected: self
                            .request
                            .observed
                            .get(field(&target.node, "uid"))
                            .cloned()
                            .unwrap_or(target.node),
                        open_inodes: self.request.open_inodes.clone(),
                    };
                    let result = ondemand_replace::execute(&mut replacement, host);
                    self.request.tree = replacement.tree;
                    let updated = self.entry(id)?.node;
                    if revision(&updated) != target_revision {
                        self.committed = Some(updated);
                    }
                    result?;
                    return Ok(Value::Null);
                }
                if source.parent != parent && source.node["name"] != name {
                    return Err(fail(
                        libc::EXDEV,
                        "Move and rename in two separate operations",
                    ));
                }
                self.check_root(host)?;
                let current = host.request("info", json!({"path":source.node["path"]}))?;
                if revision(&current) != revision(&source.node) {
                    return Err(fail(
                        libc::ESTALE,
                        "Remote entry changed before renaming or moving",
                    ));
                }
                if source.parent == parent {
                    host.request("rename", json!({"path":source.node["path"],"name":name}))?;
                } else {
                    host.request("move",json!({"paths":[source.node["path"]],"destination":self.entry(parent)?.node["path"]}))?;
                }
                self.refresh(source.parent, host)?;
                if source.parent != parent {
                    self.refresh(parent, host)?;
                }
                Ok(Value::Null)
            }
            Action::Unlink { source, directory } => {
                let (entry, directory) = (self.entry(*source)?, *directory);
                self.not_open(entry.inode)?;
                if directory != (entry.node["type"] == "folder") {
                    return Err(fail(
                        if directory {
                            libc::ENOTDIR
                        } else {
                            libc::EISDIR
                        },
                        "Wrong entry type",
                    ));
                }
                if directory {
                    // Even unsupported/ambiguous children make the cloud folder nonempty.
                    if self.refresh(entry.inode, host)? != 0 {
                        return Err(fail(libc::ENOTEMPTY, "Directory is not empty"));
                    }
                }
                self.check_root(host)?;
                let current = host.request("info", json!({"path":entry.node["path"]}))?;
                if revision(&current) != revision(&entry.node) {
                    return Err(fail(libc::ESTALE, "Remote entry changed before trashing"));
                }
                host.request("trash", json!({"paths":[entry.node["path"]]}))?;
                if let Some(children) = self.request.tree.children.get_mut(&entry.parent) {
                    children.remove(field(&entry.node, "name"));
                }
                self.request.tree.pinned.remove(&entry.inode);
                self.save()?;
                Ok(Value::Null)
            }
        }
    }
}
pub fn run(request: Request, host: &mut impl Services) -> std::result::Result<Value, Failure> {
    let mut context = Context {
        request,
        refreshed: BTreeSet::new(),
        committed: None,
    };
    let result = context.execute(host);
    let mut value = json!({"tree":context.request.tree,"refreshed":context.refreshed,"committed":context.committed});
    match result {
        Ok(result) => value["value"] = result,
        Err(Error::Fs(errno, message)) => {
            value["errno"] = json!(errno);
            value["message"] = json!(message);
        }
        Err(Error::Backend(error)) => {
            value["backend_error"] = json!(error.message);
            value["cancelled"] = json!(error.cancelled);
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ondemand_io;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nd-namespace-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            ondemand_io::private_directory(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn file() -> Value {
        json!({"uid":"file","name":"file","path":"/my-files/root/file","type":"file","size":0,"sha1":"da39a3ee5e6b4b0d3255bfef95601890afd80709","revision":"v1","safe":true})
    }
    struct Host {
        changed: bool,
        wrong_account: bool,
        mutations: usize,
        nonempty: bool,
        unsupported: bool,
    }
    impl Services for Host {
        fn poll(&mut self) -> std::result::Result<(), Failure> {
            Ok(())
        }
        fn request(&mut self, method: &str, args: Value) -> std::result::Result<Value, Failure> {
            match method {
                "info" if args["path"] == "/my-files" => {
                    Ok(json!({"uid":if self.wrong_account{"other"}else{"root"},"type":"folder"}))
                }
                "info" => {
                    let mut n = file();
                    if self.changed {
                        n["revision"] = json!("v2");
                    }
                    Ok(n)
                }
                "list_nodes" => Ok(if self.nonempty {
                    {
                        let mut child = file();
                        if self.unsupported {
                            child["sha1"] = json!("");
                        }
                        child["uid"] = json!("child");
                        child["name"] = json!("child");
                        child["path"] = json!("/my-files/root/file/child");
                        json!([child])
                    }
                } else {
                    json!([])
                }),
                _ => {
                    self.mutations += 1;
                    Ok(Value::Null)
                }
            }
        }
    }
    fn request(temp: &Temp, action: Value) -> Request {
        serde_json::from_value(json!({"cache":temp.0,"objects":temp.0.join("objects"),"staging":temp.0.join("staging"),"recovery":temp.0.join("recovery"),"remote_root":"/my-files","tree":{"entries":[{"inode":1,"parent":1,"node":{"uid":"root","type":"folder","path":"/my-files/root"}},{"inode":2,"parent":1,"node":file()}],"children":{"1":{"file":2}},"listed":[1],"pinned":[],"next_inode":3},"observed":{},"open_inodes":[],"operation":action})).unwrap()
    }
    #[test]
    fn open_stale_and_wrong_account_guards_prevent_namespace_mutations() {
        for variant in 0..4 {
            let temp = Temp::new();
            let mut host = Host {
                changed: variant == 1 || variant == 3,
                wrong_account: variant == 2,
                mutations: 0,
                nonempty: false,
                unsupported: false,
            };
            let mut request = request(
                &temp,
                if variant == 3 {
                    json!({"action":"rename","source":2,"parent":1,"name":"renamed","no_replace":false})
                } else {
                    json!({"action":"unlink","source":2,"directory":false})
                },
            );
            if variant == 0 {
                request.open_inodes.insert(2);
            }
            let value = run(request, &mut host).unwrap();
            assert_eq!(
                value["errno"],
                match variant {
                    0 => libc::EBUSY,
                    2 => libc::EACCES,
                    _ => libc::ESTALE,
                }
            );
            assert_eq!(host.mutations, 0);
            assert!(value["committed"].is_null());
        }
    }
    #[test]
    fn nonempty_folder_is_never_trashed() {
        for unsupported in [false, true] {
            let temp = Temp::new();
            let mut request = request(
                &temp,
                json!({"action":"unlink","source":2,"directory":true}),
            );
            request.tree.entries[1].node["type"] = json!("folder");
            let mut host = Host {
                changed: false,
                wrong_account: false,
                mutations: 0,
                nonempty: true,
                unsupported,
            };
            let value = run(request, &mut host).unwrap();
            assert_eq!(value["errno"], libc::ENOTEMPTY);
            assert_eq!(host.mutations, 0);
            if unsupported {
                assert_eq!(value["tree"]["children"]["2"], json!({}));
            }
        }
    }
}
