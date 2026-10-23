//! Editor save replacement: verify target writeback before trashing the source.
use crate::{
    host::Services,
    ondemand_handles::{self as handles, Operation as H},
    ondemand_io::{self as io, fail, field, revision, Error, Result},
    ondemand_tree::{self as tree, Tree},
    ondemand_write,
    process::Failure,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::PathBuf};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub(crate) cache: PathBuf,
    pub(crate) objects: PathBuf,
    pub(crate) recovery: PathBuf,
    pub(crate) staging: PathBuf,
    pub(crate) remote_root: String,
    pub(crate) tree: Tree,
    pub(crate) source: u64,
    pub(crate) target: u64,
    pub(crate) expected: Value,
    pub(crate) open_inodes: BTreeSet<u64>,
}
const INTERRUPTED: &str =
    "File replacement was interrupted; recovery retains its content and operation details.";
struct Edit {
    id: u64,
    record: Value,
}
impl Edit {
    fn record(&mut self, state: &str, error: &str) -> Result<()> {
        self.record["state"] = json!(state);
        self.record["error"] = json!(error);
        handles::apply(H::Journal {
            id: self.id,
            value: self.record.clone(),
        })?;
        Ok(())
    }
}
impl Drop for Edit {
    fn drop(&mut self) {
        if self.id != 0 {
            let _ = handles::apply(H::Release { id: self.id });
        }
    }
}
fn journal_node(node: &Value) -> Value {
    let mut node = node.clone();
    if let Some(object) = node.as_object_mut() {
        object.remove("raw");
    }
    node
}
fn outcome(value: &Value) -> Result<()> {
    if let Some(errno) = value["errno"].as_i64() {
        return Err(fail(errno as i32, field(value, "message")));
    }
    if let Some(error) = value["backend_error"].as_str() {
        return Err(Error::Backend(Failure {
            message: error.into(),
            cancelled: value["cancelled"] == true,
        }));
    }
    Ok(())
}
pub(crate) fn execute(request: &mut Request, host: &mut impl Services) -> Result<()> {
    let source = request
        .tree
        .entries
        .iter()
        .find(|e| e.inode == request.source)
        .cloned()
        .ok_or_else(|| fail(libc::ENOENT, "Entry no longer exists"))?;
    let target = request
        .tree
        .entries
        .iter()
        .find(|e| e.inode == request.target)
        .cloned()
        .ok_or_else(|| fail(libc::ENOENT, "Entry no longer exists"))?;
    if request.source == request.target
        || field(&source.node, "type") != "file"
        || field(&target.node, "type") != "file"
    {
        return Err(fail(
            libc::ENOTSUP,
            "Only distinct regular files support replacement",
        ));
    }
    tree::node(&source.node)?;
    tree::node(&target.node)?;
    if request.open_inodes.contains(&source.inode) || request.open_inodes.contains(&target.inode) {
        return Err(fail(
            libc::EBUSY,
            "Close this file before changing its cloud name or deleting it",
        ));
    }
    if source.local || target.local {
        return Err(fail(libc::EBUSY, "The new file is already being created"));
    }
    let parent = request
        .tree
        .entries
        .iter()
        .find(|e| e.inode == target.parent)
        .ok_or_else(|| fail(libc::ENOENT, "Entry no longer exists"))?
        .node
        .clone();
    let root_uid = field(
        &request
            .tree
            .entries
            .iter()
            .find(|e| e.inode == 1)
            .ok_or_else(|| fail(libc::ENOENT, "Missing mount root"))?
            .node,
        "uid",
    )
    .to_string();
    let root = host.request("info", json!({"path":request.remote_root}))?;
    if root["uid"] != root_uid || root["type"] != "folder" {
        return Err(fail(libc::EACCES,"Cloud account or mount root changed. Unmount before reconnecting; unsaved content remains in recovery."));
    }
    let current = host.request("info", json!({"path":target.node["path"]}))?;
    if revision(&current) != revision(&request.expected) {
        return Err(fail(
            libc::ESTALE,
            "Destination changed since it was read; source file was retained",
        ));
    }
    for recovery in handles::pending(&request.recovery, &[])?
        .as_array()
        .unwrap()
    {
        if recovery["baseline"]["uid"] == target.node["uid"] {
            return Err(fail(
                libc::EBUSY,
                "Recover the previous unsaved edit before editing again",
            ));
        }
    }
    io::hydrate(
        &request.objects,
        &request.staging,
        &source.node,
        false,
        &request.remote_root,
        &root_uid,
        host,
    )?;
    let current = host.request("info", json!({"path":target.node["path"]}))?;
    tree::node(&current)?;
    if revision(&current) != revision(&request.expected) {
        return Err(fail(
            libc::ESTALE,
            "Destination changed before replacement; source retained",
        ));
    }
    let handle = handles::apply(H::Edit {
        recovery: request.recovery.clone(),
        source: Some(request.objects.join(io::key(&source.node)?)),
        node: Some(source.node.clone()),
        append: false,
    })?;
    let id = handle["id"]
        .as_u64()
        .ok_or_else(|| fail(libc::EIO, "Invalid native edit handle"))?;
    let mut edit = Edit {
        id,
        record: json!({"version":1,"state":"dirty","data":PathBuf::from(handle["work"].as_str().unwrap()).file_name().unwrap().to_str().unwrap(),"name":target.node["name"],"parent":parent["path"],"baseline":journal_node(&current),"new_file":false,"error":"","intent":{"operation":"replace","source":journal_node(&source.node),"destination":journal_node(&request.expected)}}),
    };
    let result = (|| -> Result<()> {
        edit.record("dirty", "")?;
        let write_request=serde_json::from_value(json!({"fd":handle["fd"],"work":handle["work"],"journal":handle["journal"],"objects":request.objects,"staging":request.staging,"node":target.node,"parent":parent,"baseline":current,"new_file":false,"intent":edit.record["intent"],"error":"","remote_root":request.remote_root,"root_uid":root_uid})).map_err(|_|fail(libc::EINVAL,"Invalid replacement write request"))?;
        let result = ondemand_write::run(write_request, host)?;
        if !result["after"].is_null() {
            let after = result["after"].clone();
            let target = request
                .tree
                .entries
                .iter_mut()
                .find(|e| e.inode == request.target)
                .unwrap();
            target.node = after.clone();
            target.local = false;
            edit.record["baseline"] = journal_node(&after);
        }
        outcome(&result)?;
        // Match ordinary flush: metadata must be durable before the namespace step.
        tree::save(
            &request.cache.join("metadata.json"),
            &request.remote_root,
            request.tree.clone(),
        )?;
        edit.record("replacement-uploaded-source-trash-pending", "")?;
        let current_source = host.request("info", json!({"path":source.node["path"]}))?;
        if revision(&current_source) != revision(&source.node) {
            return Err(fail(
                libc::ESTALE,
                "Replacement uploaded but source changed; both files retained",
            ));
        }
        host.request("trash", json!({"paths":[source.node["path"]]}))?;
        if let Some(children) = request.tree.children.get_mut(&source.parent) {
            children.remove(field(&source.node, "name"));
        }
        request.tree.pinned.remove(&source.inode);
        tree::save(
            &request.cache.join("metadata.json"),
            &request.remote_root,
            request.tree.clone(),
        )?;
        edit.record("clean", "")?;
        handles::apply(H::Settled { id })?;
        Ok(())
    })();
    let result = match result {
        Ok(()) => Ok(()),
        Err(error) => match edit.record("recovery", INTERRUPTED) {
            Ok(()) => Err(error),
            Err(record_error) => Err(record_error),
        },
    };
    let released = handles::apply(H::Release { id });
    edit.id = 0;
    result?;
    released?;
    Ok(())
}

pub fn run(mut request: Request, host: &mut impl Services) -> std::result::Result<Value, Failure> {
    let result = execute(&mut request, host);
    let mut value = json!({"tree":request.tree});
    if let Err(error) = result {
        match error {
            Error::Fs(errno, message) => {
                value["errno"] = json!(errno);
                value["message"] = json!(message);
            }
            Error::Backend(error) => {
                value["backend_error"] = json!(error.message);
                value["cancelled"] = json!(error.cancelled);
            }
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nd-replace-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            io::private_directory(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn node(source: bool, uploaded: bool) -> Value {
        use sha1::{Digest, Sha1};
        let name = if source { ".temp" } else { "file" };
        let content = if source || uploaded { b"abc" } else { b"old" };
        json!({"uid":name,"name":name,"path":format!("/my-files/root/{name}"),"type":"file","safe":true,"raw":{"fixture":"metadata excluded from journals"},"size":3,"sha1":format!("{:x}",Sha1::digest(content)),"revision":if !source&&uploaded{"v2"}else{"v1"}})
    }
    struct Host {
        uploaded: bool,
        trashed: bool,
        fail_trash: bool,
        changed: bool,
        break_metadata: bool,
        cache: PathBuf,
    }
    impl Services for Host {
        fn poll(&mut self) -> std::result::Result<(), Failure> {
            Ok(())
        }
        fn request(&mut self, method: &str, args: Value) -> std::result::Result<Value, Failure> {
            match method {
                "info" if args["path"] == "/my-files" => Ok(json!({"uid":"root","type":"folder"})),
                "info" if args["path"] == "/my-files/root/.temp" => {
                    let mut n = node(true, false);
                    if self.uploaded && self.changed {
                        n["revision"] = json!("concurrent-edit");
                    }
                    Ok(n)
                }
                "info" => Ok(node(false, self.uploaded)),
                "list_nodes" => Ok(json!([node(true, false), node(false, self.uploaded)])),
                "download" => {
                    std::fs::write(
                        PathBuf::from(args["destination"].as_str().unwrap()).join(".temp"),
                        b"abc",
                    )
                    .unwrap();
                    Ok(Value::Null)
                }
                "upload" => {
                    assert_eq!(
                        std::fs::read(args["paths"][0].as_str().unwrap()).unwrap(),
                        b"abc"
                    );
                    self.uploaded = true;
                    if self.break_metadata {
                        std::fs::create_dir(self.cache.join("metadata.json")).unwrap();
                    }
                    Ok(Value::Null)
                }
                "trash" => {
                    assert!(self.uploaded);
                    assert_eq!(args["paths"], json!(["/my-files/root/.temp"]));
                    if self.fail_trash {
                        return Err("synthetic source trash failure".into());
                    }
                    self.trashed = true;
                    Ok(Value::Null)
                }
                _ => panic!("Unexpected service {method}"),
            }
        }
    }
    fn setup(temp: &Temp) -> (Request, Host) {
        for name in ["objects", "staging", "recovery"] {
            io::private_directory(&temp.0.join(name)).unwrap();
        }
        let tree = json!({"entries":[{"inode":1,"parent":1,"node":{"uid":"root","name":"My files","path":"/my-files/root","type":"folder"}},{"inode":2,"parent":1,"node":node(true,false)},{"inode":3,"parent":1,"node":node(false,false)}],"children":{"1":{".temp":2,"file":3}},"pinned":[2],"listed":[1],"next_inode":4});
        (serde_json::from_value(json!({"cache":temp.0,"objects":temp.0.join("objects"),"staging":temp.0.join("staging"),"recovery":temp.0.join("recovery"),"remote_root":"/my-files","tree":tree,"source":2,"target":3,"expected":node(false,false),"open_inodes":[]})).unwrap(),Host{uploaded:false,trashed:false,fail_trash:false,changed:false,break_metadata:false,cache:temp.0.clone()})
    }
    #[test]
    fn replacement_commits_target_then_trashes_source_and_cleans_recovery() {
        let temp = Temp::new();
        let (request, mut host) = setup(&temp);
        let value = run(request, &mut host).unwrap();
        assert!(value.get("errno").is_none(), "{value}");
        assert!(host.uploaded && host.trashed);
        assert_eq!(value["tree"]["children"]["1"], json!({"file":3}));
        assert_eq!(value["tree"]["pinned"], json!([]));
        assert_eq!(
            std::fs::read_dir(temp.0.join("recovery")).unwrap().count(),
            0
        );
        let saved = io::read_json(&temp.0.join("metadata.json")).unwrap();
        assert_eq!(saved["entries"].as_array().unwrap().len(), 2);
        assert_eq!(saved["entries"][1]["node"]["revision"], "v2");
    }
    #[test]
    fn post_commit_failures_keep_both_files_and_updated_recovery_baseline() {
        for variant in 0..3 {
            let temp = Temp::new();
            let (request, mut host) = setup(&temp);
            match variant {
                0 => host.fail_trash = true,
                1 => host.changed = true,
                _ => host.break_metadata = true,
            }
            let value = run(request, &mut host).unwrap();
            assert!(
                value.get("errno").is_some() || value.get("backend_error").is_some(),
                "{value}"
            );
            assert!(host.uploaded && !host.trashed);
            assert_eq!(value["tree"]["entries"][2]["node"]["revision"], "v2");
            let pending = handles::pending(&temp.0.join("recovery"), &[]).unwrap();
            assert_eq!(pending.as_array().unwrap().len(), 1);
            assert_eq!(pending[0]["baseline"]["revision"], "v2");
            assert_eq!(pending[0]["intent"]["operation"], "replace");
            assert!(pending[0]["baseline"].get("raw").is_none());
            assert!(pending[0]["intent"]["source"].get("raw").is_none());
            assert!(pending[0]["intent"]["destination"].get("raw").is_none());
            assert_eq!(
                std::fs::read(pending[0]["data_path"].as_str().unwrap()).unwrap(),
                b"abc"
            );
        }
    }
    #[test]
    fn stale_or_open_target_is_rejected_before_upload() {
        for open in [false, true] {
            let temp = Temp::new();
            let (mut request, mut host) = setup(&temp);
            if open {
                request.open_inodes.insert(3);
            } else {
                request.expected["revision"] = json!("old-observation");
            }
            let value = run(request, &mut host).unwrap();
            assert!(value.get("errno").is_some());
            assert!(!host.uploaded && !host.trashed);
            assert_eq!(
                std::fs::read_dir(temp.0.join("recovery")).unwrap().count(),
                0
            );
        }
    }
}
