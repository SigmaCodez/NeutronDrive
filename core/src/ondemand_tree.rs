//! Online-drive directory snapshots and reconciliation, shared by native callers.
use crate::{
    backend,
    ondemand_io::{self, fail, field, revision, Error, Result},
    process::Failure,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub(crate) inode: u64,
    pub(crate) parent: u64,
    pub(crate) node: Value,
    #[serde(default)]
    pub(crate) local: bool,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Tree {
    pub(crate) entries: Vec<Entry>,
    #[serde(deserialize_with = "children_map")]
    pub(crate) children: BTreeMap<u64, BTreeMap<String, u64>>,
    pub(crate) pinned: BTreeSet<u64>,
    pub(crate) listed: BTreeSet<u64>,
    pub(crate) next_inode: u64,
    #[serde(default)]
    pub(crate) warning: String,
}
fn children_map<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<u64, BTreeMap<String, u64>>, D::Error> {
    let values = BTreeMap::<String, BTreeMap<String, u64>>::deserialize(deserializer)?;
    values
        .into_iter()
        .map(|(key, value)| {
            key.parse()
                .map(|key| (key, value))
                .map_err(serde::de::Error::custom)
        })
        .collect()
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Name {
        name: String,
    },
    Node {
        node: Value,
    },
    Load {
        path: PathBuf,
        remote_root: String,
    },
    Save {
        path: PathBuf,
        remote_root: String,
        tree: Tree,
    },
    Reconcile {
        tree: Tree,
        inode: u64,
        nodes: Vec<Value>,
        open_inodes: BTreeSet<u64>,
    },
}
pub fn name(value: &str) -> Result<()> {
    backend::name(value)?;
    if value.chars().any(|c| "*?[{".contains(c) || c == '\u{7f}') {
        return Err(fail(
            libc::EINVAL,
            "Name contains unsupported CLI glob syntax or control characters",
        ));
    }
    if value.len() > 255 {
        return Err(fail(
            libc::ENAMETOOLONG,
            "Name exceeds the Linux component limit",
        ));
    }
    Ok(())
}
pub fn node(value: &Value) -> Result<()> {
    if !value.get("safe").and_then(Value::as_bool).unwrap_or(true)
        || !matches!(field(value, "type"), "file" | "folder")
        || field(value, "uid").is_empty()
    {
        return Err(fail(libc::EIO, "Unsafe or unsupported remote node"));
    }
    name(field(value, "name"))?;
    backend::remote(field(value, "path"))?;
    if field(value, "type") == "file"
        && (value["size"].as_u64().is_none()
            || field(value, "sha1").len() != 40
            || !field(value, "sha1").bytes().all(|b| b.is_ascii_hexdigit())
            || field(value, "revision").is_empty())
    {
        return Err(fail(
            libc::ENOTSUP,
            "File has no verifiable size, SHA1 or revision (native documents are unsupported)",
        ));
    }
    Ok(())
}
fn invalid() -> Error {
    fail(
        libc::EIO,
        "Saved mount metadata is invalid; preserve it and choose a fresh cache",
    )
}
pub(crate) fn load(path: &std::path::Path, root: &str) -> Result<Tree> {
    if !path.try_exists()? {
        return Ok(Tree {
            next_inode: 2,
            ..Tree::default()
        });
    }
    let value = ondemand_io::read_json(path).map_err(|_| invalid())?;
    let parse = || -> Result<Tree> {
        if value["version"] != 1 || value["remote_root"] != root {
            return Err(invalid());
        }
        let mut tree = Tree::default();
        let mut identities = BTreeMap::new();
        for item in value["entries"].as_array().ok_or_else(invalid)? {
            let entry: Entry = serde_json::from_value(item.clone()).map_err(|_| invalid())?;
            if entry.inode == 0
                || entry.parent == 0
                || entry.local
                || identities.insert(entry.inode, entry.clone()).is_some()
            {
                return Err(invalid());
            }
            if entry.inode != 1 {
                node(&entry.node)?;
            }
            tree.entries.push(entry);
        }
        let root_entry = identities.get(&1).ok_or_else(invalid)?;
        if field(&root_entry.node, "type") != "folder"
            || root_entry.parent != 1
            || field(&root_entry.node, "uid").is_empty()
        {
            return Err(invalid());
        }
        for entry in &tree.entries {
            if field(&entry.node, "type") == "folder" {
                tree.children.entry(entry.inode).or_default();
            }
            if entry.inode != 1 {
                let parent = identities.get(&entry.parent).ok_or_else(invalid)?;
                if field(&parent.node, "type") != "folder" {
                    return Err(invalid());
                }
                if tree
                    .children
                    .entry(entry.parent)
                    .or_default()
                    .insert(field(&entry.node, "name").into(), entry.inode)
                    .is_some()
                {
                    return Err(invalid());
                }
            }
        }
        if reachable(&tree).len() != tree.entries.len() {
            return Err(invalid());
        }
        for (key, target) in [("pinned", &mut tree.pinned), ("listed", &mut tree.listed)] {
            if let Some(values) = value.get(key) {
                for item in values.as_array().ok_or_else(invalid)? {
                    let number = item
                        .as_u64()
                        .or_else(|| item.as_str().and_then(|s| s.parse().ok()))
                        .ok_or_else(invalid)?;
                    if identities.contains_key(&number) {
                        target.insert(number);
                    }
                }
            }
        }
        tree.next_inode = identities
            .keys()
            .next_back()
            .unwrap()
            .checked_add(1)
            .ok_or_else(invalid)?;
        Ok(tree)
    };
    parse().map_err(|_| invalid())
}
fn reachable(tree: &Tree) -> BTreeSet<u64> {
    let mut found = BTreeSet::new();
    let mut pending = vec![1];
    while let Some(inode) = pending.pop() {
        if found.insert(inode) {
            if let Some(children) = tree.children.get(&inode) {
                pending.extend(children.values().copied());
            }
        }
    }
    found
}
pub(crate) fn save(path: &std::path::Path, root: &str, tree: Tree) -> Result<()> {
    let reachable = reachable(&tree);
    let entries: Vec<_> = tree
        .entries
        .into_iter()
        .filter(|e| !e.local && reachable.contains(&e.inode))
        .map(|mut e| {
            if let Some(object) = e.node.as_object_mut() {
                object.remove("raw");
            }
            json!({"inode":e.inode,"parent":e.parent,"node":e.node})
        })
        .collect();
    ondemand_io::write_json(
        path,
        json!({"version":1,"remote_root":root,"entries":entries,"listed":tree.listed,"pinned":tree.pinned}),
    )
}
pub(crate) fn reconcile(
    mut tree: Tree,
    inode: u64,
    nodes: Vec<Value>,
    open: BTreeSet<u64>,
) -> Result<Tree> {
    let entry = tree
        .entries
        .iter()
        .find(|e| e.inode == inode)
        .ok_or_else(|| fail(libc::ENOENT, "Entry no longer exists"))?;
    if field(&entry.node, "type") != "folder" {
        return Err(fail(libc::ENOTDIR, "Not a directory"));
    }
    let mut counts = BTreeMap::new();
    for n in &nodes {
        *counts.entry(field(n, "name").to_string()).or_insert(0usize) += 1;
    }
    let known: BTreeMap<_, _> = tree
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| (field(&e.node, "uid").to_string(), i))
        .collect();
    let mut updated = BTreeMap::new();
    tree.warning.clear();
    for mut n in nodes {
        if node(&n).is_err() || counts[field(&n, "name")] != 1 {
            continue;
        }
        let visible_name = field(&n, "name").to_string();
        let folder = field(&n, "type") == "folder";
        let child = if let Some(index) = known.get(field(&n, "uid")) {
            let existing = &mut tree.entries[*index];
            if !open.contains(&existing.inode) {
                if tree.pinned.contains(&existing.inode) && revision(&existing.node) != revision(&n)
                {
                    let path = n["path"].clone();
                    n = existing.node.clone();
                    n["name"] = json!(visible_name);
                    n["path"] = path;
                }
                existing.node = n;
                existing.parent = inode;
            }
            existing.inode
        } else {
            let child = tree.next_inode;
            if child < 2 || tree.entries.iter().any(|e| e.inode == child) {
                return Err(invalid());
            }
            tree.next_inode = child.checked_add(1).ok_or_else(invalid)?;
            tree.entries.push(Entry {
                inode: child,
                parent: inode,
                node: n,
                local: false,
            });
            child
        };
        updated.insert(visible_name, child);
        if folder {
            tree.children.entry(child).or_default();
        }
    }
    if let Some(children) = tree.children.get(&inode) {
        for (name, child) in children {
            if tree.entries.iter().any(|e| e.inode == *child && e.local) {
                if updated.contains_key(name) {
                    tree.warning =
                        "Remote name conflicts with a pending local create; local data is retained"
                            .into();
                }
                updated.insert(name.clone(), *child);
            }
        }
    }
    tree.children.insert(inode, updated);
    tree.listed.insert(inode);
    Ok(tree)
}
pub fn run(operation: Operation) -> std::result::Result<Value, Failure> {
    let result = (|| -> Result<Value> {
        Ok(match operation {
            Operation::Name { name: value } => {
                name(&value)?;
                json!(value)
            }
            Operation::Node { node: value } => {
                node(&value)?;
                value
            }
            Operation::Load { path, remote_root } => json!(load(&path, &remote_root)?),
            Operation::Save {
                path,
                remote_root,
                tree,
            } => {
                save(&path, &remote_root, tree)?;
                Value::Null
            }
            Operation::Reconcile {
                tree,
                inode,
                nodes,
                open_inodes,
            } => json!(reconcile(tree, inode, nodes, open_inodes)?),
        })
    })();
    match result {
        Ok(value) => Ok(json!({"value":value})),
        Err(Error::Fs(errno, message)) => Ok(json!({"errno":errno,"message":message})),
        Err(Error::Backend(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(uid: &str, name: &str, revision: &str) -> Value {
        json!({"uid":uid,"name":name,"type":"file","path":format!("/my-files/root/{name}"),
               "size":3,"sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","revision":revision,"safe":true})
    }
    fn tree() -> Tree {
        Tree {
            entries: vec![Entry {
                inode: 1,
                parent: 1,
                local: false,
                node: json!({"uid":"root","type":"folder","name":"My files","path":"/my-files/root"}),
            }],
            next_inode: 2,
            ..Tree::default()
        }
    }
    #[test]
    fn reconciliation_retains_pins_open_snapshots_and_local_creates() {
        let mut t = reconcile(
            tree(),
            1,
            vec![file("one", "a", "v1"), file("two", "b", "v1")],
            BTreeSet::new(),
        )
        .unwrap();
        t.pinned.insert(2);
        t.entries.push(Entry {
            inode: 4,
            parent: 1,
            node: file("local-one", "new", "local"),
            local: true,
        });
        t.next_inode = 5;
        t.children.get_mut(&1).unwrap().insert("new".into(), 4);
        let t = reconcile(
            t,
            1,
            vec![
                file("one", "renamed", "v2"),
                file("two", "b", "v2"),
                file("three", "new", "v1"),
            ],
            BTreeSet::from([3]),
        )
        .unwrap();
        assert_eq!(t.entries[1].node["revision"], "v1");
        assert_eq!(t.entries[1].node["name"], "renamed");
        assert_eq!(t.entries[2].node["revision"], "v1");
        assert_eq!(t.children[&1]["new"], 4);
        assert!(!t.warning.is_empty());
        assert_eq!(t.next_inode, 6);
    }
    #[test]
    fn ambiguous_and_unverifiable_nodes_never_become_files() {
        let mut invalid = file("three", "doc", "v1");
        invalid["sha1"] = json!("");
        let t = reconcile(
            tree(),
            1,
            vec![
                file("one", "same", "v1"),
                file("two", "same", "v1"),
                invalid,
            ],
            BTreeSet::new(),
        )
        .unwrap();
        assert!(t.children[&1].is_empty());
        assert_eq!(t.next_inode, 2);
        assert!(matches!(
            name(&"é".repeat(128)),
            Err(Error::Fs(libc::ENAMETOOLONG, _))
        ));
        assert!(matches!(name("wild*"), Err(Error::Fs(libc::EINVAL, _))));
    }
    #[test]
    fn persistent_tree_roundtrip_and_corruption_rejection() {
        let dir = std::env::temp_dir().join(format!(
            "nd-tree-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        ondemand_io::private_directory(&dir).unwrap();
        let path = dir.join("metadata.json");
        let mut t =
            reconcile(tree(), 1, vec![file("one", "é.txt", "v1")], BTreeSet::new()).unwrap();
        t.pinned.insert(2);
        save(&path, "/my-files", t).unwrap();
        let t = load(&path, "/my-files").unwrap();
        assert_eq!(t.children[&1]["é.txt"], 2);
        assert!(t.pinned.contains(&2));
        assert!(t.listed.contains(&1));
        assert_eq!(t.next_inode, 3);
        assert!(load(&path, "/my-files/other").is_err());
        let original = ondemand_io::read_json(&path).unwrap();
        for mutation in 0..3 {
            let mut corrupt = original.clone();
            match mutation {
                0 => corrupt["entries"][1]["parent"] = json!(2),
                1 => {
                    let copy = corrupt["entries"][1].clone();
                    corrupt["entries"].as_array_mut().unwrap().push(copy);
                }
                _ => corrupt["entries"][1]["parent"] = json!(999),
            }
            ondemand_io::write_json(&path, corrupt.clone()).unwrap();
            assert!(matches!(
                load(&path, "/my-files"),
                Err(Error::Fs(libc::EIO, _))
            ));
            assert_eq!(ondemand_io::read_json(&path).unwrap(), corrupt);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    use std::time::SystemTime;
}
