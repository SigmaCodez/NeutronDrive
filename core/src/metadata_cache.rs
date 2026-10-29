//! Event-invalidated directory metadata with owned handles and generation guards.
use crate::{cache::Invalidation, host::Services, process::Failure};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
};
type Result<T> = std::result::Result<T, Failure>;
#[derive(Default)]
struct Cache {
    listings: BTreeMap<String, Vec<Value>>,
    uid_paths: BTreeMap<String, String>,
    parents: BTreeMap<String, BTreeSet<String>>,
    children: BTreeMap<String, BTreeSet<String>>,
    aliases: BTreeMap<String, String>,
    generation: u64,
}
static CACHES: LazyLock<Mutex<BTreeMap<u64, Arc<Mutex<Cache>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static NEXT: AtomicU64 = AtomicU64::new(1);
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Create,
    Release {
        id: u64,
    },
    Info {
        id: u64,
        path: String,
    },
    List {
        id: u64,
        path: String,
        fresh: bool,
    },
    Invalidate {
        id: u64,
        path: String,
    },
    Event {
        id: u64,
        roots: Vec<String>,
        node_uid: Option<String>,
        parent_uid: Option<String>,
        reset: bool,
    },
    Upload {
        id: u64,
        paths: Vec<String>,
        parent: String,
        file_strategy: String,
        folder_strategy: String,
    },
    Mkdir {
        id: u64,
        parent: String,
        name: String,
    },
    Trash {
        id: u64,
        paths: Vec<String>,
    },
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .ok_or_else(|| "Invalid cached node metadata".into())
}
fn lock(cache: &Mutex<Cache>) -> Result<std::sync::MutexGuard<'_, Cache>> {
    cache
        .lock()
        .map_err(|_| "Metadata cache unavailable".into())
}
impl Cache {
    fn path(&self, path: &str) -> String {
        self.aliases
            .get(path)
            .map(String::as_str)
            .unwrap_or(path)
            .into()
    }
    fn invalidate(&mut self, path: &str) {
        self.generation = self.generation.wrapping_add(1);
        self.listings.remove(&self.path(path));
    }
    fn event(
        &mut self,
        roots: Vec<String>,
        node_uid: Option<String>,
        parent_uid: Option<String>,
        reset: bool,
    ) {
        self.generation = self.generation.wrapping_add(1);
        let plan = Invalidation {
            roots,
            children: self.children.clone(),
            aliases: self.aliases.clone(),
            parents: if reset {
                BTreeMap::new()
            } else {
                self.parents.clone()
            },
            uid_paths: if reset {
                BTreeMap::new()
            } else {
                self.uid_paths.clone()
            },
            node_uid,
            parent_uid,
            reset,
        }
        .plan();
        for path in plan {
            self.listings.remove(&path);
        }
    }
}
pub fn run(operation: Operation, host: &mut impl Services) -> Result<Value> {
    match operation {
        Operation::Create => {
            let mut caches = CACHES.lock().map_err(|_| "Metadata registry unavailable")?;
            if caches.len() >= 1024 {
                return Err("Too many metadata caches".into());
            }
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            caches.insert(id, Arc::new(Mutex::new(Cache::default())));
            Ok(json!(id))
        }
        Operation::Release { id } => {
            CACHES
                .lock()
                .map_err(|_| "Metadata registry unavailable")?
                .remove(&id);
            Ok(Value::Null)
        }
        operation => {
            let id = match &operation {
                Operation::Info { id, .. }
                | Operation::List { id, .. }
                | Operation::Invalidate { id, .. }
                | Operation::Event { id, .. }
                | Operation::Upload { id, .. }
                | Operation::Mkdir { id, .. }
                | Operation::Trash { id, .. } => *id,
                _ => unreachable!(),
            };
            let cache = CACHES
                .lock()
                .map_err(|_| "Metadata registry unavailable")?
                .get(&id)
                .cloned()
                .ok_or("Unknown metadata cache")?;
            match operation {
                Operation::Info { path, .. } => {
                    let generation = lock(&cache)?.generation;
                    let node = host.request("info", json!({"path":path}))?;
                    let uid = string(&node, "uid")?.to_owned();
                    let actual = string(&node, "path")?.to_owned();
                    let mut cache = lock(&cache)?;
                    // Do not overwrite cache state refreshed after a cloud event.
                    if cache.generation != generation {
                        return Ok(node);
                    }
                    cache.uid_paths.insert(uid.clone(), actual.clone());
                    cache.aliases.insert(path, actual.clone());
                    let parents = cache.parents.get(&uid).cloned().unwrap_or_default();
                    if node["raw"]["isTrashed"] == true {
                        cache.event(vec![actual], None, None, true);
                        for parent in parents {
                            cache.invalidate(&parent)
                        }
                    } else {
                        for parent in parents {
                            if let Some(listing) = cache.listings.get_mut(&parent) {
                                for value in listing.iter_mut() {
                                    if value["uid"] == uid {
                                        *value = node.clone()
                                    }
                                }
                            }
                        }
                    }
                    Ok(node)
                }
                Operation::List { path, fresh, .. } => {
                    let (path, generation) = {
                        let cache = lock(&cache)?;
                        let path = cache.path(&path);
                        if !fresh {
                            if let Some(nodes) = cache.listings.get(&path) {
                                return Ok(json!(nodes));
                            }
                        }
                        (path, cache.generation)
                    };
                    let nodes = host.request("list_nodes", json!({"path":path}))?;
                    let nodes = nodes.as_array().ok_or("Invalid remote listing")?.clone();
                    // Validate before changing any cache state.
                    for node in &nodes {
                        string(node, "uid")?;
                        string(node, "path")?;
                        string(node, "type")?;
                    }
                    let mut cache = lock(&cache)?;
                    if cache.generation == generation {
                        cache.listings.insert(path.clone(), nodes.clone());
                        cache.children.insert(
                            path.clone(),
                            nodes
                                .iter()
                                .filter(|n| n["type"] == "folder")
                                .map(|n| string(n, "path").unwrap().into())
                                .collect(),
                        );
                        for node in &nodes {
                            let uid = string(node, "uid")?.to_owned();
                            cache
                                .uid_paths
                                .insert(uid.clone(), string(node, "path")?.into());
                            cache.parents.entry(uid).or_default().insert(path.clone());
                        }
                    }
                    Ok(json!(nodes))
                }
                Operation::Invalidate { path, .. } => {
                    lock(&cache)?.invalidate(&path);
                    Ok(Value::Null)
                }
                Operation::Event {
                    roots,
                    node_uid,
                    parent_uid,
                    reset,
                    ..
                } => {
                    lock(&cache)?.event(roots, node_uid, parent_uid, reset);
                    Ok(Value::Null)
                }
                Operation::Upload {
                    paths,
                    parent,
                    file_strategy,
                    folder_strategy,
                    ..
                } => {
                    let result=host.request("upload",json!({"paths":paths,"parent":parent,"file_strategy":file_strategy,"folder_strategy":folder_strategy}));
                    lock(&cache)?.invalidate(&parent);
                    result
                }
                Operation::Mkdir { parent, name, .. } => {
                    let result = host.request("mkdir", json!({"parent":parent,"name":name}));
                    lock(&cache)?.invalidate(&parent);
                    result
                }
                Operation::Trash { paths, .. } => {
                    let (targets, known, parents) = {
                        let cache = lock(&cache)?;
                        let targets: BTreeSet<_> = paths.iter().map(|p| cache.path(p)).collect();
                        let known: BTreeSet<_> = cache
                            .uid_paths
                            .iter()
                            .filter(|(uid, path)| {
                                targets.contains(*path)
                                    && cache.parents.get(*uid).is_some_and(|v| !v.is_empty())
                            })
                            .map(|(_, p)| p.clone())
                            .collect();
                        let parents: BTreeSet<_> = cache
                            .uid_paths
                            .iter()
                            .filter(|(_, p)| targets.contains(*p))
                            .flat_map(|(uid, _)| {
                                cache.parents.get(uid).into_iter().flatten().cloned()
                            })
                            .collect();
                        (targets, known, parents)
                    };
                    let result = host.request("trash", json!({"paths":paths}));
                    let mut cache = lock(&cache)?;
                    for target in &targets {
                        cache.event(vec![target.clone()], None, None, true)
                    }
                    if targets.is_subset(&known) {
                        for parent in parents {
                            cache.invalidate(&parent)
                        }
                    } else {
                        cache.generation = cache.generation.wrapping_add(1);
                        cache.listings.clear();
                    }
                    result
                }
                _ => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Host<F>(F);
    impl<F: FnMut(&str, Value) -> Result<Value>> Services for Host<F> {
        fn poll(&mut self) -> Result<()> {
            Ok(())
        }
        fn request(&mut self, method: &str, args: Value) -> Result<Value> {
            (self.0)(method, args)
        }
    }
    fn create() -> u64 {
        run(
            Operation::Create,
            &mut Host(|_: &str, _: Value| panic!("no SDK call")),
        )
        .unwrap()
        .as_u64()
        .unwrap()
    }
    fn release(id: u64) {
        run(
            Operation::Release { id },
            &mut Host(|_: &str, _: Value| panic!("no SDK call")),
        )
        .unwrap();
    }
    fn listing(id: u64, host: &mut impl Services) -> Result<Value> {
        run(
            Operation::List {
                id,
                path: "/root".into(),
                fresh: false,
            },
            host,
        )
    }
    fn node() -> Value {
        json!({"uid":"file","name":"data","type":"file","path":"/root/file","revision":"old","raw":{}})
    }
    #[test]
    fn info_started_before_invalidation_cannot_overwrite_new_listing() {
        let id = create();
        listing(id, &mut Host(|_: &str, _: Value| Ok(json!([node()])))).unwrap();
        let mut latest = node();
        latest["revision"] = json!("new");
        let expected = latest.clone();
        run(
            Operation::Info {
                id,
                path: "/root/file".into(),
            },
            &mut Host(|_: &str, _: Value| {
                run(
                    Operation::Invalidate {
                        id,
                        path: "/root".into(),
                    },
                    &mut Host(|_: &str, _: Value| panic!("no SDK")),
                )?;
                listing(
                    id,
                    &mut Host(|_: &str, _: Value| Ok(json!([latest.clone()]))),
                )?;
                Ok(node())
            }),
        )
        .unwrap();
        let result = listing(
            id,
            &mut Host(|_: &str, _: Value| panic!("new listing must remain cached")),
        )
        .unwrap();
        assert_eq!(result, json!([expected]));
        release(id);
    }

    #[test]
    fn event_during_fetch_prevents_stale_repopulation() {
        let id = create();
        let (started, wait_started) = std::sync::mpsc::channel();
        let (resume, wait_resume) = std::sync::mpsc::channel();
        let fetch = std::thread::spawn(move || {
            listing(
                id,
                &mut Host(|_: &str, _: Value| {
                    started.send(()).unwrap();
                    wait_resume.recv().unwrap();
                    Ok(json!([node()]))
                }),
            )
        });
        wait_started.recv().unwrap();
        run(
            Operation::Invalidate {
                id,
                path: "/root".into(),
            },
            &mut Host(|_: &str, _: Value| panic!("no SDK call")),
        )
        .unwrap();
        resume.send(()).unwrap();
        assert_eq!(fetch.join().unwrap().unwrap().as_array().unwrap().len(), 1);
        let mut count = 0;
        let mut host = Host(|_: &str, _: Value| {
            count += 1;
            Ok(json!([]))
        });
        assert_eq!(listing(id, &mut host).unwrap(), json!([]));
        assert_eq!(listing(id, &mut host).unwrap(), json!([]));
        assert_eq!(count, 1);
        release(id);
    }
    #[test]
    fn release_during_fetch_keeps_active_operation_owned() {
        let id = create();
        let (started, wait_started) = std::sync::mpsc::channel();
        let (resume, wait_resume) = std::sync::mpsc::channel();
        let fetch = std::thread::spawn(move || {
            listing(
                id,
                &mut Host(|_: &str, _: Value| {
                    started.send(()).unwrap();
                    wait_resume.recv().unwrap();
                    Ok(json!([]))
                }),
            )
        });
        wait_started.recv().unwrap();
        release(id);
        resume.send(()).unwrap();
        assert_eq!(fetch.join().unwrap().unwrap(), json!([]));
        assert!(listing(
            id,
            &mut Host(|_: &str, _: Value| panic!("expired handle must not call SDK"))
        )
        .is_err());
    }
    #[test]
    fn failed_write_still_invalidates_and_info_is_always_fresh() {
        let id = create();
        let mut calls = 0;
        let mut host = Host(|method: &str, _: Value| {
            calls += 1;
            match method {
                "list_nodes" => Ok(json!([node()])),
                "info" => {
                    let mut n = node();
                    n["revision"] = json!("fresh");
                    Ok(n)
                }
                "trash" => Err("synthetic lost acknowledgement".into()),
                _ => panic!("unexpected request"),
            }
        });
        listing(id, &mut host).unwrap();
        listing(id, &mut host).unwrap();
        run(
            Operation::Info {
                id,
                path: "/root/file".into(),
            },
            &mut host,
        )
        .unwrap();
        assert_eq!(listing(id, &mut host).unwrap()[0]["revision"], "fresh");
        assert!(run(
            Operation::Trash {
                id,
                paths: vec!["/root/file".into()]
            },
            &mut host
        )
        .is_err());
        listing(id, &mut host).unwrap();
        assert_eq!(calls, 4);
        release(id);
    }
}
