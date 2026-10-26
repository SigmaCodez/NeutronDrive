//! Read-only metadata mirror, independent of the SDK/mount mutation lock.
use crate::{
    ondemand_fs::Action,
    ondemand_io,
    ondemand_tree::{Entry, Tree},
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
#[derive(Default)]
pub(crate) struct Snapshot {
    pub generation: u64,
    pub tree: Tree,
    pub entries: BTreeMap<u64, Entry>,
    pub objects: PathBuf,
    pub writable: bool,
    pub blocked: bool,
    pub writers: BTreeSet<u64>,
    pub downloading: BTreeSet<u64>,
}
impl Snapshot {
    pub fn answer(&self, action: &Action) -> Option<Value> {
        if self.blocked {
            return None;
        }
        match action {
            Action::Metadata { inode } => {
                if self.writers.contains(inode) {
                    return None;
                }
                let e = self.entries.get(inode)?;
                Some(
                    json!({"inode":inode,"folder":e.node["type"]=="folder","size":e.node["size"].as_u64().unwrap_or(0),"mtime_ns":crate::ondemand_fs::timestamp(e.node["modified"].as_str().unwrap_or("")),"cached":self.cached(e),"pinned":self.tree.pinned.contains(inode),"writable":self.writable}),
                )
            }
            Action::Lookup { parent, name } => {
                let id = match name.as_str() {
                    "." => *parent,
                    ".." => self.entries.get(parent)?.parent,
                    _ => *self.tree.children.get(parent)?.get(name)?,
                };
                Some(json!(self.entries.get(&id)?))
            }
            Action::Listdir { inode, .. } if self.tree.listed.contains(inode) => {
                let mut result = Vec::new();
                if let Some(children) = self.tree.children.get(inode) {
                    for id in children.values() {
                        result.push(self.entries.get(id)?);
                    }
                }
                Some(json!(result))
            }
            _ => None,
        }
    }
    fn cached(&self, entry: &Entry) -> bool {
        !self.writers.contains(&entry.inode)
            && ondemand_io::key(&entry.node).is_ok_and(|key| self.objects.join(key).is_file())
    }
    fn complete(
        &self,
        id: u64,
        memo: &mut BTreeMap<u64, bool>,
        visiting: &mut BTreeSet<u64>,
    ) -> bool {
        if let Some(value) = memo.get(&id) {
            return *value;
        }
        if visiting.len() >= 256 || !visiting.insert(id) {
            return false;
        }
        let result = self.entries.get(&id).is_some_and(|e| {
            if e.node["type"] != "folder" {
                return self.cached(e);
            }
            self.tree.listed.contains(&id)
                && self.tree.children.get(&id).is_none_or(|children| {
                    children
                        .values()
                        .all(|child| self.complete(*child, memo, visiting))
                })
        });
        visiting.remove(&id);
        memo.insert(id, result);
        result
    }
    pub fn availability(&self) -> Value {
        let mut paths = BTreeMap::new();
        let mut downloading = BTreeMap::new();
        let mut memo = BTreeMap::new();
        if !self.blocked {
            for e in self.entries.values() {
                let folder = e.node["type"] == "folder";
                let complete = self.complete(e.inode, &mut memo, &mut BTreeSet::new());
                let available = complete && (!folder || self.tree.pinned.contains(&e.inode));
                let mut active = self.downloading.contains(&e.inode);
                let mut parts = Vec::new();
                let mut current = e;
                let mut visited = BTreeSet::new();
                while current.inode != 1 && visited.insert(current.inode) && visited.len() <= 256 {
                    parts.push(current.node["name"].as_str().unwrap_or(""));
                    let Some(parent) = self.entries.get(&current.parent) else {
                        break;
                    };
                    current = parent;
                    active |= self.downloading.contains(&current.inode);
                }
                if current.inode == 1 {
                    parts.reverse();
                    let path = parts.join("/");
                    if active && (!complete || self.downloading.contains(&e.inode)) {
                        downloading.insert(path, true);
                    } else if available {
                        paths.insert(path, true);
                    }
                }
            }
        }
        // Bind transient progress to this process instance, not just a reusable PID.
        let owner_start = std::fs::read_to_string("/proc/self/stat")
            .ok()
            .and_then(|s| s.rsplit_once(") ").map(|(_, fields)| fields.to_owned()))
            .and_then(|s| s.split_whitespace().nth(19).map(str::to_owned));
        json!({"version":1,"available":paths,"downloading":downloading,
            "owner_pid":std::process::id(),"owner_start":owner_start})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_check_requires_complete_descendants_and_no_writer() {
        let dir = std::env::temp_dir().join(format!("nd-emblem-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let folder = |inode, parent, name: &str| Entry {
            inode,
            parent,
            node: json!({"uid":format!("u{inode}"),"name":name,"type":"folder"}),
            local: false,
        };
        let file = Entry {
            inode: 3,
            parent: 2,
            node: json!({"uid":"f","name":"file","type":"file","revision":"r1","sha1":"hash","size":3}),
            local: false,
        };
        let mut s = Snapshot {
            objects: dir.clone(),
            ..Default::default()
        };
        s.entries = BTreeMap::from([
            (1, folder(1, 1, "root")),
            (2, folder(2, 1, "sub")),
            (3, file.clone()),
        ]);
        s.tree.children = BTreeMap::from([
            (1, BTreeMap::from([("sub".into(), 2)])),
            (2, BTreeMap::from([("file".into(), 3)])),
        ]);
        s.tree.pinned = BTreeSet::from([1, 2, 3]);
        s.tree.listed.insert(1);
        assert!(s.availability()["available"]
            .as_object()
            .unwrap()
            .is_empty());
        s.tree.listed.insert(2);
        assert!(s.availability()["available"]
            .as_object()
            .unwrap()
            .is_empty());
        let object = dir.join(ondemand_io::key(&file.node).unwrap());
        std::fs::write(&object, b"abc").unwrap();
        let v = s.availability();
        for path in ["", "sub", "sub/file"] {
            assert_eq!(v["available"][path], true);
        }
        s.downloading.insert(1);
        let progress = s.availability();
        assert_eq!(progress["downloading"][""], true);
        assert_eq!(progress["available"]["sub/file"], true);
        s.downloading.clear();
        s.writers.insert(3);
        assert!(s.availability()["available"]
            .as_object()
            .unwrap()
            .is_empty());
        s.writers.clear();
        std::fs::remove_file(object).unwrap();
        assert!(s.availability()["available"]
            .as_object()
            .unwrap()
            .is_empty());
        s.downloading.insert(1);
        let progress = s.availability();
        for path in ["", "sub", "sub/file"] {
            assert_eq!(progress["downloading"][path], true);
            assert_ne!(progress["available"][path], true);
        }
        s.blocked = true;
        assert!(s.availability()["downloading"]
            .as_object()
            .unwrap()
            .is_empty());
        std::fs::remove_dir(dir).unwrap();
    }
}
