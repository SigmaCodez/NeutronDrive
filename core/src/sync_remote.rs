//! Remote traversal and preflight validation independent of a UI/interpreter.
use crate::{host::Services, process::Failure};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
type Result<T> = std::result::Result<T, Failure>;
/// Production services for callers that already own the serialized SDK lifecycle.
pub struct Sdk<'a, F: FnMut(Value) -> i32> {
    pub executable: String,
    pub callback: &'a mut F,
}
impl<F: FnMut(Value) -> i32> Services for Sdk<'_, F> {
    fn poll(&mut self) -> Result<()> {
        crate::filesystem::poll(self.callback)
    }
    fn request(&mut self, method: &str, arguments: Value) -> Result<Value> {
        let action = match method {
            "info" | "try_info" => "info",
            "list_nodes" | "list_nodes_fresh" => "list",
            "mkdir" => "mkdir",
            "rename" => "rename",
            "move" => "move",
            "trash" => "trash",
            "upload" => "upload",
            "download" => "download",
            _ => return Err("Unsupported native sync service".into()),
        };
        let mut arguments = arguments;
        arguments["action"] = json!(action);
        if matches!(action, "upload" | "download") {
            let values = arguments
                .as_object_mut()
                .ok_or("Invalid transfer arguments")?;
            values
                .entry("file_strategy")
                .or_insert_with(|| json!("rename"));
            values
                .entry("folder_strategy")
                .or_insert_with(|| json!("merge"));
        }
        let operation =
            serde_json::from_value(arguments).map_err(|_| "Invalid native SDK request")?;
        let result = crate::drive::run(self.executable.clone(), operation, self.callback);
        if method == "try_info" {
            match result {
                Ok(node) => Ok(json!({"node":node})),
                Err(error) if error.cancelled => Err(error),
                Err(_) => Ok(json!({"unavailable":true})),
            }
        } else {
            result
        }
    }
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Root {
        path: String,
    },
    Entry {
        node: Value,
        allow_trashed: bool,
    },
    NodeEntry {
        node: Value,
        allow_trashed: bool,
    },
    Scan {
        root: Value,
        fresh: bool,
        allow_trashed: bool,
    },
    Lookup {
        root: Value,
        relative: String,
    },
}
fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn kind(node: &Value) -> String {
    let kind = string(node, "type").to_lowercase();
    if kind == "directory" {
        "folder".into()
    } else {
        kind
    }
}
pub fn name_error(name: &str) -> &'static str {
    if name.is_empty() || matches!(name, "." | "..") || name.contains(['/', '\\']) {
        "Unsafe or ambiguous path component"
    } else if name.chars().any(|c| (c as u32) < 32 || c == '\x7f') {
        "Control characters in file name are unsupported"
    } else if name.contains(['*', '?', '[', '{']) {
        "Name contains glob syntax interpreted by the Proton CLI"
    } else if name.starts_with(".neutrondrive-") {
        "Reserved local backup/staging name; excluded from sync"
    } else {
        ""
    }
}
pub fn entry(node: &Value, allow_trashed: bool) -> Result<Value> {
    if node["safe"] == false {
        return Err("Remote metadata or decrypted name is not trustworthy".into());
    }
    if node["raw"]["isTrashed"] == true && !allow_trashed {
        return Err("Trashed remote nodes are not active sync entries".into());
    }
    let media = string(node, "media_type").to_lowercase();
    if media.contains("proton") && (media.contains("doc") || media.contains("sheet")) {
        return Err("Native Proton Docs/Sheets cannot be exported by the CLI".into());
    }
    let kind = kind(node);
    if kind != "folder" && kind != "file" {
        return Err(format!("Unsupported remote node type: {kind}").into());
    }
    let digest = string(node, "sha1").to_lowercase();
    if kind == "file" && (digest.len() != 40 || !digest.bytes().all(|c| c.is_ascii_hexdigit())) {
        return Err(
            "Remote file has no usable SHA-1 digest (draft/native document/metadata unavailable)"
                .into(),
        );
    }
    let uid = string(node, "uid");
    let path = string(node, "path");
    if uid.is_empty() || path.is_empty() {
        return Err("Remote node has no stable identifier or address".into());
    }
    let size = if node["size"].is_null() {
        0
    } else {
        node["size"].as_u64().ok_or("Invalid remote file size")?
    };
    Ok(
        json!({"kind":kind,"sha1":digest,"size":size,"uid":uid,"revision":string(node,"revision"),"remote_path":path}),
    )
}
fn node_entry(mut node: Value, allow_trashed: bool, host: &mut impl Services) -> Result<Value> {
    if kind(&node) == "file" && string(&node, "sha1").is_empty() {
        let uid = string(&node, "uid").to_owned();
        node = host.request("info", json!({"path":string(&node,"path")}))?;
        if uid.is_empty() || string(&node, "uid") != uid {
            return Err("Remote node identity changed while fetching metadata".into());
        }
    }
    entry(&node, allow_trashed)
}
struct Scan {
    entries: Map<String, Value>,
    blocked: Map<String, Value>,
    visited: BTreeSet<String>,
    fresh: bool,
    allow_trashed: bool,
}
impl Scan {
    fn visit(
        &mut self,
        path: &str,
        prefix: &str,
        depth: usize,
        host: &mut impl Services,
    ) -> Result<()> {
        host.poll()?;
        if depth > 256 {
            return Err("Remote tree exceeds the safe maximum depth".into());
        }
        let nodes = host.request(
            if self.fresh {
                "list_nodes_fresh"
            } else {
                "list_nodes"
            },
            json!({"path":path}),
        )?;
        let mut grouped: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for node in nodes.as_array().ok_or("Invalid remote listing")? {
            grouped
                .entry(string(node, "name").into())
                .or_default()
                .push(node.clone());
        }
        for (name, nodes) in grouped {
            host.poll()?;
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let reason = name_error(&name);
            if !reason.is_empty() || nodes.len() != 1 {
                self.blocked.insert(
                    relative,
                    json!(if reason.is_empty() {
                        "Multiple remote nodes have the same name"
                    } else {
                        reason
                    }),
                );
                continue;
            }
            let value = node_entry(nodes.into_iter().next().unwrap(), self.allow_trashed, host);
            let value = match value {
                Ok(v) => v,
                Err(e) if e.cancelled => return Err(e),
                Err(e) => {
                    self.blocked.insert(relative, json!(e.message));
                    continue;
                }
            };
            if !self.visited.insert(string(&value, "uid").into()) {
                self.blocked.insert(
                    relative,
                    json!("Remote tree contains a repeated node identifier"),
                );
                continue;
            }
            if self.visited.len() > 100_000 {
                return Err("Remote tree exceeds the safe maximum entries".into());
            }
            self.entries.insert(relative.clone(), value.clone());
            if value["kind"] == "folder" {
                self.visit(string(&value, "remote_path"), &relative, depth + 1, host)?;
            }
        }
        Ok(())
    }
}
pub fn run(operation: Operation, host: &mut impl Services) -> Result<Value> {
    match operation {
        Operation::Root { path } => {
            let node = host.request("info", json!({"path":path}))?;
            if entry(&node, false)?["kind"] != "folder" {
                return Err("The remote sync root must be a folder".into());
            }
            let boundary = crate::computer::boundary(&path)?;
            if !boundary.is_empty() {
                let account = if path == boundary {
                    node.clone()
                } else {
                    host.request("info", json!({"path":boundary}))?
                };
                if entry(&account, false)?["kind"] != "folder" {
                    return Err("The account root is not safely accessible".into());
                }
                let mut current = node.clone();
                let mut visited = BTreeSet::new();
                let mut verified = false;
                for _ in 0..128 {
                    if current["uid"] == account["uid"] {
                        verified = true;
                        break;
                    }
                    if !visited.insert(string(&current, "uid").to_owned()) {
                        return Err("Remote root has a cyclic ancestor chain".into());
                    }
                    let parent = string(&current["raw"], "parentUid");
                    if parent.is_empty() {
                        return Err("Remote root ancestry cannot be verified".into());
                    }
                    if parent == string(&account, "uid") {
                        verified = true;
                        break;
                    }
                    let parent = parent.to_owned();
                    current =
                        host.request("info", json!({"path":format!("{boundary}/{parent}")}))?;
                    if entry(&current, false)?["kind"] != "folder"
                        || string(&current, "uid") != parent
                    {
                        return Err("Remote ancestor disappeared or was replaced".into());
                    }
                }
                if !verified {
                    return Err("Remote root ancestry exceeds the safe maximum depth".into());
                }
            }
            Ok(node)
        }

        Operation::Entry {
            node,
            allow_trashed,
        } => entry(&node, allow_trashed),
        Operation::NodeEntry {
            node,
            allow_trashed,
        } => node_entry(node, allow_trashed, host),
        Operation::Scan {
            root,
            fresh,
            allow_trashed,
        } => {
            let mut scan = Scan {
                entries: Map::new(),
                blocked: Map::new(),
                visited: BTreeSet::from([string(&root, "uid").into()]),
                fresh,
                allow_trashed,
            };
            scan.visit(string(&root, "path"), "", 0, host)?;
            Ok(json!({"entries":scan.entries,"blocked":scan.blocked}))
        }
        Operation::Lookup { root, relative } => {
            let parts: Vec<_> = relative.split('/').collect();
            if parts.iter().any(|p| !name_error(p).is_empty()) {
                return Err("Unsafe relative path".into());
            }
            let mut path = string(&root, "path").to_owned();
            let mut value = Value::Null;
            for (index, name) in parts.iter().enumerate() {
                host.poll()?;
                let nodes = host.request("list_nodes_fresh", json!({"path":path}))?;
                let matches: Vec<_> = nodes
                    .as_array()
                    .ok_or("Invalid remote listing")?
                    .iter()
                    .filter(|n| string(n, "name") == *name)
                    .collect();
                if matches.is_empty() {
                    return Ok(Value::Null);
                }
                if matches.len() != 1 {
                    return Err("Ambiguous remote name appeared after preview".into());
                }
                value = node_entry(matches[0].clone(), false, host)?;
                if index < parts.len() - 1 && value["kind"] != "folder" {
                    return Err("Remote parent is no longer a folder".into());
                }
                path = string(&value, "remote_path").into();
            }
            Ok(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Host {
        listing: Value,
        calls: Vec<String>,
        cancelled: bool,
    }
    impl Services for Host {
        fn poll(&mut self) -> Result<()> {
            if self.cancelled {
                Err(Failure {
                    message: "cancelled".into(),
                    cancelled: true,
                })
            } else {
                Ok(())
            }
        }
        fn request(&mut self, method: &str, _arguments: Value) -> Result<Value> {
            self.calls.push(method.into());
            if method == "info" {
                Ok(
                    json!({"uid":"file","path":"/my-files/file","name":"file","type":"file","sha1":"0000000000000000000000000000000000000000","size":4}),
                )
            } else {
                Ok(self.listing.clone())
            }
        }
    }
    fn root() -> Value {
        json!({"uid":"root","path":"/my-files"})
    }
    #[test]
    fn unsafe_duplicate_and_repeated_nodes_are_blocked() {
        let mut host = Host {
            listing: json!([
                {"name":"duplicate","type":"folder","uid":"a","path":"/a"},
                {"name":"duplicate","type":"folder","uid":"b","path":"/b"},
                {"name":"unsafe*","type":"folder","uid":"c","path":"/c"},
                {"name":"cycle","type":"folder","uid":"root","path":"/my-files"},
                {"name":"document","type":"file","uid":"doc","path":"/doc","media_type":"application/proton-doc"}
            ]),
            calls: vec![],
            cancelled: false,
        };
        let result = run(
            Operation::Scan {
                root: root(),
                fresh: true,
                allow_trashed: false,
            },
            &mut host,
        )
        .unwrap();
        assert_eq!(result["entries"], json!({}));
        assert_eq!(result["blocked"].as_object().unwrap().len(), 4);
        assert_eq!(host.calls, ["list_nodes_fresh", "info"]);
    }
    #[test]
    fn missing_digest_is_fetched_and_lookup_is_fresh() {
        let mut host = Host {
            listing: json!([{"name":"file","type":"file","uid":"file","path":"/my-files/file"}]),
            calls: vec![],
            cancelled: false,
        };
        let result = run(
            Operation::Lookup {
                root: root(),
                relative: "file".into(),
            },
            &mut host,
        )
        .unwrap();
        assert_eq!(result["size"], 4);
        assert_eq!(host.calls, ["list_nodes_fresh", "info"]);
        host.cancelled = true;
        assert!(
            run(
                Operation::Scan {
                    root: root(),
                    fresh: false,
                    allow_trashed: false
                },
                &mut host
            )
            .unwrap_err()
            .cancelled
        );
        assert_eq!(host.calls.len(), 2);
    }
    #[test]
    fn malformed_metadata_is_never_an_active_entry() {
        for node in [
            json!({"type":"file","sha1":"bad"}),
            json!({"type":"folder","safe":false}),
            json!({"type":"folder","uid":"id","path":"/id","raw":{"isTrashed":true}}),
        ] {
            assert!(entry(&node, false).is_err());
        }
        let node = json!({"type":"directory","uid":"id","path":"/id","raw":{"isTrashed":true}});
        assert_eq!(entry(&node, true).unwrap()["kind"], "folder");
    }
}
