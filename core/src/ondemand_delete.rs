//! Durable, local-first deletion. Kernel callbacks never wait for the SDK.
use crate::{
    host::Services,
    ondemand_io::{self as io, fail, field, revision, Result},
    ondemand_tree::Entry,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Task {
    pub entry: Entry,
    pub parent_path: String,
    pub state: String,
    pub error: String,
}
#[derive(Default)]
pub(crate) struct Queue {
    path: PathBuf,
    pub root: String,
    pub tasks: Vec<Task>,
    last: Option<Instant>,
}
pub(crate) type Shared = Arc<Mutex<Queue>>;
impl Queue {
    pub fn load(cache: &Path, root: &str) -> Result<Shared> {
        let path = cache.join("deletions.json");
        let mut q = Self {
            path,
            root: root.into(),
            ..Self::default()
        };
        if q.path.try_exists()? {
            let v = io::read_json(&q.path)?;
            if v["version"] != 1 || v["root"] != root {
                return Err(fail(
                    libc::ESTALE,
                    "Deletion journal belongs to another cloud root",
                ));
            }
            q.tasks = serde_json::from_value(v["tasks"].clone())
                .map_err(|_| fail(libc::EIO, "Invalid deletion journal"))?;
            for t in &q.tasks {
                crate::ondemand_tree::node(&t.entry.node)?;
                crate::backend::remote(&t.parent_path)?;
                if t.entry.inode == 1
                    || t.entry.local
                    || !matches!(t.state.as_str(), "pending" | "done" | "conflict")
                {
                    return Err(fail(libc::EIO, "Invalid deletion journal entry"));
                }
            }
        }
        Ok(Arc::new(Mutex::new(q)))
    }
    pub fn ready(&self) -> bool {
        self.tasks.iter().any(|t| t.state == "pending")
            && self
                .last
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(30))
    }
    fn save(&self) -> Result<()> {
        io::write_json(
            &self.path,
            json!({"version":1,"root":self.root,"tasks":self.tasks}),
        )
    }
    pub fn hidden(&self, node: &Value) -> bool {
        self.tasks
            .iter()
            .any(|t| t.entry.node["uid"] == node["uid"] && t.state != "conflict")
    }
    pub fn enqueue(&mut self, entry: Entry, parent_path: String) -> Result<()> {
        if self.hidden(&entry.node) {
            return Err(fail(libc::ENOENT, "Entry already deleted locally"));
        }
        if self.tasks.len() >= 10000 {
            return Err(fail(
                libc::ENOSPC,
                "Finish pending cloud deletions before deleting more files",
            ));
        }
        let old = self.tasks.clone();
        self.tasks
            .retain(|t| t.entry.node["uid"] != entry.node["uid"]);
        self.tasks.push(Task {
            entry,
            parent_path,
            state: "pending".into(),
            error: String::new(),
        });
        if let Err(e) = self.save() {
            self.tasks = old;
            return Err(e);
        }
        self.last = None;
        Ok(())
    }
    pub fn status(&self) -> Value {
        json!({"pending":self.tasks.iter().filter(|t|t.state=="pending").count(),"items":self.tasks.iter().filter(|t|t.state!="done").map(|t|json!({"name":t.entry.node["name"],"state":t.state,"error":t.error})).collect::<Vec<_>>()})
    }
    pub fn forget_confirmed(&mut self, parent: u64, nodes: &[Value]) -> Result<()> {
        let old = self.tasks.clone();
        self.tasks.retain(|t| {
            t.state != "done"
                || t.entry.parent != parent
                || nodes.iter().any(|n| n["uid"] == t.entry.node["uid"])
        });
        if self.tasks.len() != old.len() {
            if let Err(e) = self.save() {
                self.tasks = old;
                return Err(e);
            }
        }
        Ok(())
    }
}
/// One operation per idle turn; never keep the queue mutex during a cloud call.
pub(crate) fn drain(
    queue: &Shared,
    remote_root: &str,
    objects: &Path,
    host: &mut impl Services,
) -> Result<()> {
    let (root, task) = {
        let mut q = queue.lock().unwrap();
        if q.last
            .is_some_and(|t| t.elapsed() < Duration::from_secs(30))
        {
            return Ok(());
        }
        let Some(task) = q.tasks.iter().find(|t| t.state == "pending").cloned() else {
            return Ok(());
        };
        q.last = Some(Instant::now());
        (q.root.clone(), task)
    };
    let attempt = || -> Result<bool> {
        let identity = host.request("info", json!({"path":remote_root}))?;
        if field(&identity, "uid") != root || identity["type"] != "folder" {
            return Ok(false);
        }
        let probe = host.request("try_info", json!({"path":task.entry.node["path"]}))?;
        if probe["unavailable"] == true {
            let listing = host.request("list_nodes", json!({"path":task.parent_path}))?;
            let nodes = listing
                .as_array()
                .ok_or_else(|| fail(libc::EIO, "Invalid cloud folder response"))?;
            if !nodes.iter().any(|n| n["uid"] == task.entry.node["uid"]) {
                return Ok(true);
            }
            return Err(fail(libc::EAGAIN, "Cloud entry could not be verified"));
        }
        let current = &probe["node"];
        crate::ondemand_tree::node(current)?;
        if revision(current) != revision(&task.entry.node)
            || current["type"] != task.entry.node["type"]
            || current["name"] != task.entry.node["name"]
        {
            return Ok(false);
        }
        if current["type"] == "folder" {
            let children = host.request("list_nodes", json!({"path":current["path"]}))?;
            if !children
                .as_array()
                .ok_or_else(|| fail(libc::EIO, "Invalid cloud folder response"))?
                .is_empty()
            {
                return Ok(false);
            }
        }
        host.request("trash", json!({"paths":[task.entry.node["path"]]}))?;
        Ok(true)
    };
    let mut attempt = attempt;
    let result = attempt();
    let mut q = queue.lock().unwrap();
    let old = q.tasks.clone();
    if let Some(t) = q
        .tasks
        .iter_mut()
        .find(|t| t.entry.node["uid"] == task.entry.node["uid"] && t.state == "pending")
    {
        match result {
            Ok(true) => {
                t.state = "done".into();
                t.error.clear();
                q.last = None;
            }
            Ok(false) => {
                t.state = "conflict".into();
                t.error =
                    "Cloud entry changed; deletion stopped. Review the file before trying again."
                        .into();
                q.last = None;
            }
            Err(_) => {
                t.error="Cloud deletion pending; retrying automatically when the connection is available.".into();
            }
        }
    }
    if let Err(e) = q.save() {
        q.tasks = old;
        return Err(e);
    }
    if q.tasks
        .iter()
        .any(|t| t.entry.node["uid"] == task.entry.node["uid"] && t.state == "done")
    {
        if let Ok(key) = io::key(&task.entry.node) {
            let _ = std::fs::remove_file(objects.join(key));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!(
                "nd-delete-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            io::private_directory(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn entry() -> Entry {
        Entry {
            inode: 2,
            parent: 1,
            local: false,
            node: json!({"uid":"file-1","name":"test.txt","path":"/my-files/file-1","type":"file","safe":true,"size":1,"sha1":"0000000000000000000000000000000000000000","revision":"r1"}),
        }
    }
    struct Cloud {
        node: Value,
        root: &'static str,
        fail: bool,
        trash: usize,
        children: Value,
    }
    impl Services for Cloud {
        fn poll(&mut self) -> std::result::Result<(), crate::process::Failure> {
            Ok(())
        }
        fn request(
            &mut self,
            method: &str,
            _: Value,
        ) -> std::result::Result<Value, crate::process::Failure> {
            if self.fail {
                return Err("Offline".into());
            }
            Ok(match method {
                "info" => json!({"uid":self.root,"type":"folder"}),
                "try_info" => {
                    if self.node.is_null() {
                        json!({"unavailable":true})
                    } else {
                        json!({"node":self.node})
                    }
                }
                "list_nodes" => self.children.clone(),
                "trash" => {
                    self.trash += 1;
                    self.node = Value::Null;
                    Value::Null
                }
                _ => panic!("unexpected {method}"),
            })
        }
    }
    fn cloud() -> Cloud {
        Cloud {
            node: entry().node,
            root: "root",
            fail: false,
            trash: 0,
            children: json!([]),
        }
    }
    #[test]
    fn journal_survives_restart_and_cloud_ack_is_idempotent() {
        let temp = Temp::new();
        let q = Queue::load(&temp.0, "root").unwrap();
        q.lock()
            .unwrap()
            .enqueue(entry(), "/my-files/root".into())
            .unwrap();
        drop(q);
        let q = Queue::load(&temp.0, "root").unwrap();
        assert!(q.lock().unwrap().hidden(&entry().node));
        let mut c = cloud();
        drain(&q, "/my-files", &temp.0, &mut c).unwrap();
        drain(&q, "/my-files", &temp.0, &mut c).unwrap();
        assert_eq!(c.trash, 1);
        q.lock().unwrap().forget_confirmed(1, &[]).unwrap();
        assert!(q.lock().unwrap().tasks.is_empty());
    }
    #[test]
    fn failed_durable_write_never_acknowledges_or_hides_file() {
        let temp = Temp::new();
        let q = Queue::load(&temp.0, "root").unwrap();
        std::fs::create_dir(temp.0.join("deletions.json")).unwrap();
        assert!(q
            .lock()
            .unwrap()
            .enqueue(entry(), "/my-files/root".into())
            .is_err());
        assert!(!q.lock().unwrap().hidden(&entry().node));
    }
    #[test]
    fn offline_retry_preserves_intent_and_success_clears_warning() {
        let temp = Temp::new();
        let q = Queue::load(&temp.0, "root").unwrap();
        q.lock()
            .unwrap()
            .enqueue(entry(), "/my-files/root".into())
            .unwrap();
        let mut c = cloud();
        c.fail = true;
        drain(&q, "/my-files", &temp.0, &mut c).unwrap();
        assert_eq!(q.lock().unwrap().tasks[0].state, "pending");
        assert_eq!(c.trash, 0);
        c.fail = false;
        q.lock().unwrap().last = None;
        drain(&q, "/my-files", &temp.0, &mut c).unwrap();
        assert_eq!(c.trash, 1);
        assert!(q.lock().unwrap().tasks[0].error.is_empty());
    }
    #[test]
    fn changed_revision_or_account_is_never_trashed() {
        for account in [false, true] {
            let temp = Temp::new();
            let q = Queue::load(&temp.0, "root").unwrap();
            q.lock()
                .unwrap()
                .enqueue(entry(), "/my-files/root".into())
                .unwrap();
            let mut c = cloud();
            if account {
                c.root = "other";
            } else {
                c.node["revision"] = json!("r2");
            }
            drain(&q, "/my-files", &temp.0, &mut c).unwrap();
            assert_eq!(c.trash, 0);
            assert_eq!(q.lock().unwrap().tasks[0].state, "conflict");
            assert!(!q.lock().unwrap().hidden(&entry().node));
        }
    }
    #[test]
    fn lost_ack_absent_node_finishes_without_a_second_delete() {
        let temp = Temp::new();
        let q = Queue::load(&temp.0, "root").unwrap();
        q.lock()
            .unwrap()
            .enqueue(entry(), "/my-files/root".into())
            .unwrap();
        let mut c = cloud();
        c.node = Value::Null;
        drain(&q, "/my-files", &temp.0, &mut c).unwrap();
        assert_eq!(c.trash, 0);
        assert_eq!(q.lock().unwrap().tasks[0].state, "done");
    }
    #[test]
    fn new_cloud_child_blocks_folder_deletion() {
        let temp = Temp::new();
        let q = Queue::load(&temp.0, "root").unwrap();
        let mut e = entry();
        e.node["type"] = json!("folder");
        q.lock()
            .unwrap()
            .enqueue(e.clone(), "/my-files/root".into())
            .unwrap();
        let mut c = cloud();
        c.node = e.node;
        c.children = json!([{"uid":"new-child"}]);
        drain(&q, "/my-files", &temp.0, &mut c).unwrap();
        assert_eq!(c.trash, 0);
        assert_eq!(q.lock().unwrap().tasks[0].state, "conflict");
    }
}
