//! Owned mount state and the complete online filesystem coordinator.
use crate::{
    backend,
    host::Services,
    ondemand_handles::{self as handles, Operation as H},
    ondemand_io::{self as io, fail, field, revision, Error, Result},
    ondemand_namespace,
    ondemand_tree::{self as tree, Entry, Tree},
    ondemand_write,
    process::Failure,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, Mutex, RwLock,
    },
    time::Instant,
};
const BLOCKED:&str="Cloud account or mount root changed. Unmount before reconnecting; unsaved content remains in recovery.";
const WRITE_FAILED: &str =
    "Writeback failed or conflicted; local content retained. Inspect recovery before retrying.";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    cache: PathBuf,
    remote_root: String,
    writable: bool,
    offline: bool,
    metadata_ttl: f64,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Snapshot,
    DrainDeletes,
    Invalidate,
    BlockAccount,
    SetError {
        message: String,
    },
    SetWritable {
        writable: bool,
    },
    Refresh {
        inode: u64,
    },
    Listdir {
        inode: u64,
        refresh: bool,
    },
    Lookup {
        parent: u64,
        name: String,
    },
    Metadata {
        inode: u64,
    },
    Pin {
        inode: u64,
        pinned: bool,
    },
    Evict {
        inode: u64,
    },
    Open {
        inode: u64,
        flags: i32,
    },
    Read {
        handle: u64,
        offset: i64,
        size: i64,
    },
    Write {
        handle: u64,
        offset: i64,
        content: String,
    },
    Truncate {
        inode: u64,
        size: i64,
        handle: Option<u64>,
    },
    Create {
        parent: u64,
        name: String,
        flags: i32,
    },
    Flush {
        handle: u64,
    },
    Release {
        handle: u64,
    },
    Recovery,
    Mkdir {
        parent: u64,
        name: String,
    },
    Rename {
        parent: u64,
        name: String,
        new_parent: u64,
        new_name: String,
        no_replace: bool,
    },
    Unlink {
        parent: u64,
        name: String,
        directory: bool,
    },
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Create { config: Config },
    Call { id: u64, operation: Action },
    Close { id: u64 },
    TryRecovery { id: u64 },
}
struct OpenFile {
    inode: u64,
    native: Value,
    writable: bool,
    baseline: Value,
    dirty: bool,
    failed: String,
    intent: Value,
}
struct Mount {
    snapshot: Arc<RwLock<crate::ondemand_snapshot::Snapshot>>,
    deletions: crate::ondemand_delete::Shared,
    mutation: Arc<Mutex<()>>,
    generation: u64,
    config: Config,
    objects: PathBuf,
    recovery: PathBuf,
    staging: PathBuf,
    _lock: File,
    tree: Tree,
    loaded: BTreeMap<u64, Instant>,
    files: BTreeMap<u64, OpenFile>,
    observed: BTreeMap<String, Value>,
    downloading: BTreeSet<u64>,
    error: String,
    cloud_errors: BTreeMap<(u64, &'static str), String>,
    blocked: bool,
    closed: bool,
    #[cfg(test)]
    journal_fault: Option<String>,
}
static MOUNTS: LazyLock<Mutex<BTreeMap<u64, Arc<Mutex<Mount>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static NEXT: AtomicU64 = AtomicU64::new(1);
fn result(value: &Value) -> Result<()> {
    if let Some(errno) = value["errno"].as_i64() {
        return Err(fail(errno as i32, field(value, "message")));
    }
    if let Some(message) = value["backend_error"].as_str() {
        return Err(Error::Backend(Failure {
            message: message.into(),
            cancelled: value["cancelled"] == true,
        }));
    }
    Ok(())
}
fn plain(node: &Value) -> Value {
    let mut node = node.clone();
    if let Some(o) = node.as_object_mut() {
        o.remove("raw");
    }
    node
}
fn token() -> Result<String> {
    let mut b = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b.iter().map(|b| format!("{b:02x}")).collect())
}
impl Mount {
    fn new(config: Config, host: &mut impl Services) -> Result<Self> {
        backend::remote(&config.remote_root)?;
        if config.remote_root.split('/').nth(1) != Some("my-files") {
            return Err(fail(
                libc::ENOTSUP,
                "Only My files supports an on-demand mount",
            ));
        }
        if config.offline && config.writable {
            return Err(fail(libc::EROFS, "Offline mounts are read-only"));
        }
        if !config.metadata_ttl.is_finite() || config.metadata_ttl < 0.0 {
            return Err(fail(libc::EINVAL, "Invalid metadata lifetime"));
        }
        let directory = io::private_directory(&config.cache)?;
        let objects = config.cache.join("objects");
        let recovery = config.cache.join("recovery");
        let staging = config.cache.join("staging");
        for path in [&objects, &recovery, &staging] {
            io::private_directory(path)?;
        }
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                c".lock".as_ptr(),
                libc::O_CREAT | libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let lock = unsafe { File::from_raw_fd(fd) };
        let stat = lock.metadata()?;
        if !stat.is_file()
            || stat.uid() != unsafe { libc::getuid() }
            || stat.nlink() != 1
            || stat.mode() & 0o077 != 0
            || unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) } < 0
        {
            return Err(fail(
                libc::EBUSY,
                "This cache is already used by another mount",
            ));
        }
        let mut tree = tree::load(&config.cache.join("metadata.json"), &config.remote_root)?;
        if config.offline {
            if !tree.entries.iter().any(|e| e.inode == 1) {
                return Err(fail(
                    libc::ENETDOWN,
                    "No saved directory metadata for an offline mount",
                ));
            }
        } else {
            let mut root = host.request("info", json!({"path":config.remote_root}))?;
            if root["type"] != "folder" || root["safe"] != true || field(&root, "uid").is_empty() {
                return Err(fail(libc::EIO, "Cannot verify the remote mount root"));
            }
            if tree
                .entries
                .iter()
                .any(|e| e.inode == 1 && e.node["uid"] != root["uid"])
            {
                return Err(fail(
                    libc::ESTALE,
                    "Cache belongs to another account or cloud root; choose a different cache",
                ));
            }
            backend::name(field(&root, "uid"))?;
            root["path"] = json!(format!("/my-files/{}", field(&root, "uid")));
            tree.entries.retain(|e| e.inode != 1);
            tree.entries.insert(
                0,
                Entry {
                    inode: 1,
                    parent: 1,
                    node: root,
                    local: false,
                },
            );
            tree.children.entry(1).or_default();
            tree::save(
                &config.cache.join("metadata.json"),
                &config.remote_root,
                tree.clone(),
            )?;
        }
        let deletions = crate::ondemand_delete::Queue::load(
            &config.cache,
            field(
                &tree.entries.iter().find(|e| e.inode == 1).unwrap().node,
                "uid",
            ),
        )?;
        Ok(Self {
            deletions,
            mutation: Arc::new(Mutex::new(())),
            snapshot: Arc::new(RwLock::new(crate::ondemand_snapshot::Snapshot::default())),
            generation: 0,
            config,
            objects,
            recovery,
            staging,
            _lock: lock,
            tree,
            loaded: BTreeMap::new(),
            files: BTreeMap::new(),
            observed: BTreeMap::new(),
            downloading: BTreeSet::new(),
            error: String::new(),
            cloud_errors: BTreeMap::new(),
            blocked: false,
            closed: false,
            #[cfg(test)]
            journal_fault: None,
        })
    }
    fn status(&self) -> Value {
        json!({"downloads":self.downloading.iter().filter_map(|id|self.tree.entries.iter().find(|e|e.inode==*id).map(|e|e.node["name"].clone())).collect::<Vec<_>>(),"deletions":self.deletions.lock().unwrap().status(),"last_error":if self.error.is_empty() { self.cloud_errors.values().next().map(String::as_str).unwrap_or("") } else { &self.error },"account_blocked":self.blocked,"writable":self.config.writable,"offline":self.config.offline,"root_uid":self.tree.entries.iter().find(|e|e.inode==1).map(|e|field(&e.node,"uid")).unwrap_or("")})
    }
    // A successful operation may only clear the warning for the work it
    // completed, not another pinned subtree or an unsaved-write error.
    fn cloud_failure(
        &mut self,
        inode: u64,
        operation: &'static str,
        context: &str,
        error: &Failure,
    ) {
        self.cloud_errors
            .insert((inode, operation), format!("{context} {}", error.message));
    }
    fn cloud_recovered(&mut self, inode: u64, operation: &'static str) {
        self.cloud_errors.remove(&(inode, operation));
    }
    fn entry(&self, inode: u64) -> Result<Entry> {
        self.tree
            .entries
            .iter()
            .find(|e| e.inode == inode && !self.deletions.lock().unwrap().hidden(&e.node))
            .cloned()
            .ok_or_else(|| fail(libc::ENOENT, "Entry no longer exists"))
    }
    fn replace_entry(&mut self, entry: Entry) {
        if let Some(old) = self
            .tree
            .entries
            .iter_mut()
            .find(|e| e.inode == entry.inode)
        {
            *old = entry;
        } else {
            self.tree.entries.push(entry);
        }
    }
    fn file(&self, id: u64) -> Result<&OpenFile> {
        self.files
            .get(&id)
            .ok_or_else(|| fail(libc::EBADF, "Unknown file handle"))
    }
    fn open_inodes(&self) -> BTreeSet<u64> {
        self.files.values().map(|f| f.inode).collect()
    }
    fn publish(&self, persist: bool) {
        let snapshot = crate::ondemand_snapshot::Snapshot {
            generation: self.generation,
            deletions: Some(self.deletions.clone()),
            open: self.open_inodes(),
            downloading: self.downloading.clone(),
            entries: self
                .tree
                .entries
                .iter()
                .map(|e| (e.inode, e.clone()))
                .collect(),
            tree: self.tree.clone(),
            objects: self.objects.clone(),
            writable: self.config.writable,
            offline: self.config.offline,
            blocked: self.blocked,
            writers: self
                .files
                .values()
                .filter(|f| f.writable)
                .map(|f| f.inode)
                .collect(),
        };
        if persist {
            // Decoration failures must never change the outcome of a file write.
            let _ = io::write_json(
                &self.config.cache.join("availability.json"),
                snapshot.availability(),
            );
        }
        *self.snapshot.write().unwrap() = snapshot;
    }
    fn save(&self) -> Result<()> {
        self.publish(true);
        tree::save(
            &self.config.cache.join("metadata.json"),
            &self.config.remote_root,
            self.tree.clone(),
        )
    }
    fn writable(&self) -> Result<()> {
        if self.blocked {
            return Err(fail(libc::EACCES, BLOCKED));
        }
        if !self.config.writable {
            return Err(fail(libc::EROFS, "This mount is read-only"));
        }
        Ok(())
    }
    fn check_root(&mut self, host: &mut impl Services) -> Result<()> {
        if self.blocked {
            return Err(fail(libc::EACCES, BLOCKED));
        }
        if !self.config.offline {
            let root = host.request("info", json!({"path":self.config.remote_root}))?;
            if root["uid"] != self.entry(1)?.node["uid"] || root["type"] != "folder" {
                self.blocked = true;
                self.error = BLOCKED.into();
                return Err(fail(libc::EACCES, BLOCKED));
            }
        }
        Ok(())
    }
    // A folder pin is a durable policy: later children inherit it during refresh.
    // Iterative traversal bounds cycles and avoids consuming the Rust stack.
    fn sync_pin_tree(&mut self, root: u64, host: &mut impl Services) -> Result<()> {
        self.downloading.insert(root);
        self.publish(true);
        let result = self.sync_pin_tree_inner(root, host);
        if result.is_ok() && !self.config.offline {
            self.cloud_recovered(root, "pin");
        }
        self.downloading.remove(&root);
        self.publish(true);
        result
    }
    fn sync_pin_tree_inner(&mut self, root: u64, host: &mut impl Services) -> Result<()> {
        let mut pending = vec![root];
        let mut visited = BTreeSet::new();
        // The mount mutation lock keeps handles stable throughout this traversal.
        let open = self.open_inodes();
        let mut deferred = false;
        while let Some(inode) = pending.pop() {
            host.poll()?;
            if !visited.insert(inode) {
                continue;
            }
            if visited.len() > 100000 {
                return Err(fail(
                    libc::EFBIG,
                    "Pinned folder exceeds the supported entry limit",
                ));
            }
            if self
                .tree
                .entries
                .iter()
                .find(|e| e.inode == inode)
                .is_some_and(|e| self.deletions.lock().unwrap().hidden(&e.node))
            {
                continue;
            }
            let entry = self.entry(inode)?;
            if entry.node["type"] == "folder" {
                self.tree.pinned.insert(inode);
                self.save()?;
                self.refresh_snapshot(inode, host)?;
                if let Some(children) = self.tree.children.get(&inode) {
                    pending.extend(children.values().copied());
                }
            } else {
                // Never replace the identity of an open file or accept a path now
                // pointing to a different cloud node.
                if open.contains(&inode) && !self.tree.pinned.contains(&inode) {
                    return Err(fail(
                        libc::EBUSY,
                        "Close the file before keeping it on this device",
                    ));
                }
                let current = if self.config.offline {
                    entry.node.clone()
                } else {
                    host.request("info", json!({"path":entry.node["path"]}))?
                };
                if current["uid"] != entry.node["uid"] {
                    return Err(fail(
                        libc::ESTALE,
                        "Pinned file identity changed; refresh its parent folder",
                    ));
                }
                tree::node(&current)?;
                if open.contains(&inode) {
                    // Preserve the open snapshot, but do not acknowledge an
                    // event whose changed revision has not been hydrated yet.
                    deferred |= revision(&current) != revision(&entry.node);
                    continue;
                }
                // Persist the user's offline policy before downloading. A failed
                // first transfer must remain eligible for refresh after restart.
                if self.tree.pinned.insert(inode) {
                    self.save()?;
                }
                self.downloading.insert(inode);
                self.publish(true);
                let hydrated = self.hydrate(&current, host);
                self.downloading.remove(&inode);
                hydrated?;
                // A local unlink may complete while the read-only download is in flight.
                // Keep the deletion visible and discard the newly finished cache object.
                if self.deletions.lock().unwrap().hidden(&current) {
                    let _ = std::fs::remove_file(self.objects.join(io::key(&current)?));
                    self.tree.pinned.remove(&inode);
                    self.save()?;
                    continue;
                }
                if !self.config.offline {
                    self.cloud_recovered(inode, "content");
                }
                if let Some(saved) = self.tree.entries.iter_mut().find(|e| e.inode == inode) {
                    saved.node = current;
                }
                self.tree.pinned.insert(inode);
                self.save()?;
            }
        }
        if deferred {
            return Err(fail(
                libc::EBUSY,
                "Close changed offline files to finish their cloud refresh",
            ));
        }
        if !self.config.offline {
            // A successful parent traversal also completes former child-root
            // refreshes. Do not clear errors outside the verified subtree.
            self.cloud_errors
                .retain(|(id, operation), _| *operation != "pin" || !visited.contains(id));
        }
        Ok(())
    }
    fn refresh(&mut self, inode: u64, host: &mut impl Services) -> Result<Value> {
        let result = self.refresh_snapshot(inode, host)?;
        if self.tree.pinned.contains(&inode) {
            self.sync_pin_tree(inode, host)?;
        }
        Ok(result)
    }
    fn refresh_snapshot(&mut self, inode: u64, host: &mut impl Services) -> Result<Value> {
        let entry = self.entry(inode)?;
        if entry.node["type"] != "folder" {
            return Err(fail(libc::ENOTDIR, "Not a directory"));
        }
        if self.config.offline {
            if !self.tree.listed.contains(&inode) {
                return Err(fail(
                    libc::ENETDOWN,
                    "This folder was not listed before going offline",
                ));
            }
            return self.listdir(inode, false, host);
        }
        self.check_root(host)?;
        let nodes = host.request("list_nodes", json!({"path":entry.node["path"]}))?;
        self.deletions.lock().unwrap().forget_confirmed(
            inode,
            nodes
                .as_array()
                .ok_or_else(|| fail(libc::EIO, "Invalid cloud directory response"))?,
        )?;
        self.tree = tree::reconcile(
            self.tree.clone(),
            inode,
            nodes
                .as_array()
                .ok_or_else(|| fail(libc::EIO, "Invalid cloud directory response"))?
                .clone(),
            self.open_inodes(),
        )?;
        if !self.tree.warning.is_empty() {
            self.error = self.tree.warning.clone();
        }
        self.loaded.insert(inode, Instant::now());
        self.save()?;
        self.cloud_recovered(inode, "metadata");
        self.cloud_errors
            .retain(|(id, _), _| self.tree.entries.iter().any(|entry| entry.inode == *id));
        self.listdir(inode, false, host)
    }
    fn listdir(&mut self, inode: u64, refresh: bool, host: &mut impl Services) -> Result<Value> {
        if self.entry(inode)?.node["type"] != "folder" {
            return Err(fail(libc::ENOTDIR, "Not a directory"));
        }
        if refresh
            && (!self.tree.listed.contains(&inode)
                || (!self.config.offline
                    && self.loaded.get(&inode).is_none_or(|time| {
                        time.elapsed().as_secs_f64() >= self.config.metadata_ttl
                    })))
        {
            // Browsing must not recursively hydrate a pinned subtree.
            // Pins are refreshed by explicit pin/refresh and cloud events.
            match self.refresh_snapshot(inode, host) {
                Ok(v) => return Ok(v),
                Err(Error::Backend(error))
                    if !error.cancelled && self.tree.listed.contains(&inode) =>
                {
                    // Avoid retrying the same failed network request for every stat/lookup.
                    self.loaded.insert(inode, Instant::now());
                    self.cloud_failure(
                        inode,
                        "metadata",
                        "Cloud unavailable; showing saved metadata. Pinned files remain readable.",
                        &error,
                    );
                }
                Err(e) => return Err(e),
            }
        }
        if self.config.offline && !self.tree.listed.contains(&inode) {
            return Err(fail(
                libc::ENETDOWN,
                "Folder metadata is unavailable offline",
            ));
        }
        let mut values = Vec::new();
        if let Some(children) = self.tree.children.get(&inode) {
            for id in children.values() {
                if self
                    .tree
                    .entries
                    .iter()
                    .find(|e| e.inode == *id)
                    .is_some_and(|e| self.deletions.lock().unwrap().hidden(&e.node))
                {
                    continue;
                }
                values.push(self.entry(*id)?);
            }
        }
        Ok(json!(values))
    }
    fn lookup(&mut self, parent: u64, name: &str, host: &mut impl Services) -> Result<Entry> {
        if name == "." {
            return self.entry(parent);
        }
        if name == ".." {
            return self.entry(self.entry(parent)?.parent);
        }
        self.listdir(parent, true, host)?;
        self.entry(
            *self
                .tree
                .children
                .get(&parent)
                .and_then(|c| c.get(name))
                .ok_or_else(|| fail(libc::ENOENT, "No such file"))?,
        )
    }
    fn hydrate(&mut self, node: &Value, host: &mut impl Services) -> Result<File> {
        if self.blocked {
            return Err(fail(libc::EACCES, BLOCKED));
        }
        io::hydrate(
            &self.objects,
            &self.staging,
            node,
            self.config.offline,
            &self.config.remote_root,
            field(&self.entry(1)?.node, "uid"),
            host,
        )
    }
    fn recovery(&self) -> Result<Value> {
        handles::pending(
            &self.recovery,
            &self
                .files
                .values()
                .filter_map(|f| f.native["journal"].as_str().map(PathBuf::from))
                .collect::<Vec<_>>(),
        )
    }
    fn journal(&self, file: &OpenFile, state: &str) -> Result<()> {
        #[cfg(test)]
        if self
            .journal_fault
            .as_deref()
            .is_some_and(|s| s == "all" || s == state || (s == "unclean" && state != "clean"))
        {
            return Err(fail(libc::ENOSPC, "synthetic disk full"));
        }
        let entry = self.entry(file.inode)?;
        let work = PathBuf::from(
            file.native["work"]
                .as_str()
                .ok_or_else(|| fail(libc::EBADF, "Read-only handle"))?,
        );
        handles::apply(H::Journal {
            id: file.native["id"].as_u64().unwrap(),
            value: json!({"version":1,"state":state,"data":work.file_name().unwrap().to_str().unwrap(),"name":entry.node["name"],"parent":self.entry(entry.parent)?.node["path"],"baseline":plain(&file.baseline),"new_file":entry.local,"error":file.failed,"intent":file.intent}),
        })?;
        Ok(())
    }
    fn open(&mut self, inode: u64, flags: i32, host: &mut impl Services) -> Result<u64> {
        if self.blocked {
            return Err(fail(libc::EACCES, BLOCKED));
        }
        let mut entry = self.entry(inode)?;
        if entry.node["type"] != "file" {
            return Err(fail(libc::EISDIR, "Cannot open a directory as a file"));
        }
        let writable = flags & libc::O_ACCMODE != libc::O_RDONLY;
        if !writable && flags & libc::O_TRUNC != 0 {
            return Err(fail(libc::EINVAL, "Truncation requires a writable handle"));
        }
        if writable {
            self.writable()?;
            if self.files.values().any(|f| f.inode == inode) {
                return Err(fail(
                    libc::EBUSY,
                    "Close other handles before editing this file",
                ));
            }
            if self
                .recovery()?
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["baseline"]["uid"] == entry.node["uid"])
            {
                return Err(fail(
                    libc::EBUSY,
                    "Recover the previous unsaved edit before editing again",
                ));
            }
        } else if self.files.values().any(|f| f.inode == inode && f.writable) {
            return Err(fail(libc::EBUSY, "A writer already has this file open"));
        }
        let mut cloud_confirmed = false;
        if !self.config.offline && !entry.local {
            let current = (|| -> Result<Value> {
                self.check_root(host)?;
                let n = host.request("info", json!({"path":entry.node["path"]}))?;
                tree::node(&n)?;
                Ok(n)
            })();
            match current {
                Err(Error::Backend(error))
                    if !error.cancelled && !writable && self.tree.pinned.contains(&inode) =>
                {
                    self.cloud_failure(
                        inode,
                        "content",
                        "Cloud unavailable; opening the verified pinned snapshot.",
                        &error,
                    );
                }
                Err(e) => return Err(e),
                Ok(current) => {
                    cloud_confirmed = true;
                    if revision(&current) != revision(&entry.node) {
                        if self.tree.pinned.contains(&inode) {
                            match self.hydrate(&current, host) {
                                Ok(_) => entry.node = current,
                                Err(Error::Backend(error)) if !error.cancelled && !writable => {
                                    cloud_confirmed = false;
                                    self.cloud_failure(inode, "content", "New cloud revision unavailable; using the verified pinned snapshot.", &error);
                                }
                                Err(e) => return Err(e),
                            }
                        } else {
                            entry.node = current;
                        }
                        self.replace_entry(entry.clone());
                        self.save()?;
                    }
                }
            }
        }
        if entry.local {
            return Err(fail(libc::EBUSY, "The new file is already being created"));
        }
        let source = self.hydrate(&entry.node, host)?;
        if cloud_confirmed {
            self.cloud_recovered(inode, "content");
        }
        self.observed
            .insert(field(&entry.node, "uid").into(), entry.node.clone());
        let native = if writable {
            handles::apply(H::Edit {
                recovery: self.recovery.clone(),
                source: Some(self.objects.join(io::key(&entry.node)?)),
                node: Some(entry.node.clone()),
                append: flags & libc::O_APPEND != 0,
            })?
        } else {
            handles::apply(H::Adopt {
                fd: source.as_raw_fd(),
            })?
        };
        let id = native["id"].as_u64().unwrap();
        let file = OpenFile {
            inode,
            native,
            writable,
            baseline: entry.node,
            dirty: false,
            failed: String::new(),
            intent: Value::Null,
        };
        if writable {
            if let Err(e) = self.journal(&file, "clean") {
                let _ = handles::apply(H::Abort { id });
                return Err(e);
            }
        }
        self.files.insert(id, file);
        if writable && flags & libc::O_TRUNC != 0 {
            if let Err(e) = self.truncate(inode, 0, Some(id), host) {
                let _ = self.release(id);
                return Err(e);
            }
        }
        Ok(id)
    }
    fn create(
        &mut self,
        parent: u64,
        name: &str,
        flags: i32,
        host: &mut impl Services,
    ) -> Result<Value> {
        self.writable()?;
        tree::name(name)?;
        self.refresh(parent, host)?;
        if self
            .tree
            .children
            .get(&parent)
            .is_some_and(|c| c.contains_key(name))
        {
            return Err(fail(libc::EEXIST, "Destination already exists"));
        }
        let inode = self.tree.next_inode;
        self.tree.next_inode = inode
            .checked_add(1)
            .ok_or_else(|| fail(libc::ENOSPC, "Inode identifiers exhausted"))?;
        let node = json!({"uid":format!("local-{}",token()?),"name":name,"type":"file","size":0,"revision":"local","sha1":"da39a3ee5e6b4b0d3255bfef95601890afd80709","path":format!("{}/{}",field(&self.entry(parent)?.node,"path").trim_end_matches('/'),name),"safe":true});
        let entry = Entry {
            inode,
            parent,
            node: node.clone(),
            local: true,
        };
        self.replace_entry(entry.clone());
        self.tree
            .children
            .entry(parent)
            .or_default()
            .insert(name.into(), inode);
        let created = (|| -> Result<u64> {
            let native = handles::apply(H::Edit {
                recovery: self.recovery.clone(),
                source: None,
                node: None,
                append: flags & libc::O_APPEND != 0,
            })?;
            let id = native["id"].as_u64().unwrap();
            let file = OpenFile {
                inode,
                native,
                writable: true,
                baseline: node,
                dirty: true,
                failed: String::new(),
                intent: Value::Null,
            };
            if let Err(e) = self.journal(&file, "dirty") {
                let _ = handles::apply(H::Abort { id });
                return Err(e);
            }
            self.files.insert(id, file);
            Ok(id)
        })();
        match created {
            Ok(id) => Ok(json!([id, entry])),
            Err(e) => {
                self.tree.entries.retain(|e| e.inode != inode);
                self.tree.children.get_mut(&parent).unwrap().remove(name);
                Err(e)
            }
        }
    }
    fn write(&mut self, id: u64, offset: i64, content: String) -> Result<Value> {
        if !self.file(id)?.writable {
            return Err(fail(libc::EBADF, "Read-only handle"));
        }
        if offset < 0 {
            return Err(fail(libc::EINVAL, "Invalid write offset"));
        }
        let file = self.files.get_mut(&id).unwrap();
        file.dirty = true;
        file.failed.clear();
        self.journal(self.file(id)?, "dirty")?;
        handles::apply(H::Write {
            id,
            offset,
            content,
        })
    }
    fn truncate(
        &mut self,
        inode: u64,
        size: i64,
        id: Option<u64>,
        host: &mut impl Services,
    ) -> Result<()> {
        if size < 0 {
            return Err(fail(libc::EINVAL, "Negative size"));
        }
        let number = match id {
            Some(id) => id,
            None => self.open(inode, libc::O_RDWR, host)?,
        };
        let result = (|| -> Result<()> {
            if !self.file(number)?.writable {
                return Err(fail(libc::EBADF, "Read-only handle"));
            }
            self.files.get_mut(&number).unwrap().dirty = true;
            self.journal(self.file(number)?, "dirty")?;
            handles::apply(H::Truncate { id: number, size })?;
            if id.is_none() {
                self.flush(number, host)?;
            }
            Ok(())
        })();
        if id.is_none() {
            let release = self.release(number);
            result?;
            release?;
        } else {
            result?;
        }
        Ok(())
    }
    fn flush(&mut self, id: u64, host: &mut impl Services) -> Result<()> {
        handles::apply(H::Sync { id })?;
        let mut file = self
            .files
            .remove(&id)
            .ok_or_else(|| fail(libc::EBADF, "Unknown file handle"))?;
        let result = (|| -> Result<()> {
            if !file.dirty {
                return Ok(());
            }
            self.writable()?;
            let mut entry = self.entry(file.inode)?;
            let request=serde_json::from_value(json!({"fd":file.native["fd"],"work":file.native["work"],"journal":file.native["journal"],"objects":self.objects,"staging":self.staging,"node":entry.node,"parent":self.entry(entry.parent)?.node,"baseline":file.baseline,"new_file":entry.local,"intent":file.intent,"error":file.failed,"remote_root":self.config.remote_root,"root_uid":self.entry(1)?.node["uid"]})).map_err(|_|fail(libc::EINVAL,"Invalid native writeback request"))?;
            let outcome = ondemand_write::run(request, host)?;
            if !outcome["after"].is_null() {
                entry.node = outcome["after"].clone();
                entry.local = false;
                file.baseline = entry.node.clone();
                self.replace_entry(entry.clone());
            }
            if outcome["errno"] == libc::EACCES
                && field(&outcome, "message").contains("Cloud account or mount root changed.")
            {
                self.blocked = true;
            }
            result(&outcome)?;
            self.save()?;
            handles::apply(H::Settled { id })?;
            file.dirty = false;
            file.failed.clear();
            self.observed
                .insert(field(&entry.node, "uid").into(), entry.node);
            Ok(())
        })();
        let result = if let Err(error) = result {
            file.dirty = true;
            file.failed = WRITE_FAILED.into();
            self.error = WRITE_FAILED.into();
            match self.journal(&file, "recovery") {
                Ok(()) => Err(error),
                Err(e) => Err(e),
            }
        } else {
            Ok(())
        };
        self.files.insert(id, file);
        result
    }
    fn release(&mut self, id: u64) -> Result<()> {
        let mut file = self
            .files
            .remove(&id)
            .ok_or_else(|| fail(libc::EBADF, "Unknown file handle"))?;
        let record = if file.dirty {
            if file.failed.is_empty() {
                file.failed = "Handle closed without confirmed writeback".into();
            }
            self.error = file.failed.clone();
            self.journal(&file, "recovery")
        } else {
            handles::apply(H::Settled { id }).map(|_| ())
        };
        let released = handles::apply(H::Release { id });
        record?;
        released?;
        Ok(())
    }
    fn namespace(&mut self, operation: Value, host: &mut impl Services) -> Result<Value> {
        self.writable()?;
        let request=serde_json::from_value(json!({"cache":self.config.cache,"objects":self.objects,"recovery":self.recovery,"staging":self.staging,"remote_root":self.config.remote_root,"tree":self.tree,"open_inodes":self.open_inodes(),"observed":self.observed,"operation":operation})).map_err(|_|fail(libc::EINVAL,"Invalid namespace request"))?;
        let outcome = ondemand_namespace::run(request, host)?;
        self.tree = serde_json::from_value(outcome["tree"].clone())
            .map_err(|_| fail(libc::EIO, "Invalid namespace tree"))?;
        for id in outcome["refreshed"].as_array().unwrap() {
            self.loaded.insert(id.as_u64().unwrap(), Instant::now());
        }
        if !outcome["committed"].is_null() {
            let node = outcome["committed"].clone();
            self.observed.insert(field(&node, "uid").into(), node);
        }
        result(&outcome)?;
        Ok(outcome["value"].clone())
    }
    fn evict(&self, inode: u64) -> Result<()> {
        // Validate the entire selection before removing any cached bytes.
        // Index entries/handles once and scan the object directory only once,
        // including all cached revisions of the selected files.
        let entries: BTreeMap<_, _> = self.tree.entries.iter().map(|e| (e.inode, e)).collect();
        let open = self.open_inodes();
        let mut pending = vec![inode];
        let mut visited = BTreeSet::new();
        let mut prefixes = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            let entry = entries
                .get(&id)
                .ok_or_else(|| fail(libc::ENOENT, "Entry no longer exists"))?;
            if self.tree.pinned.contains(&id) || open.contains(&id) {
                return Err(fail(
                    libc::EBUSY,
                    "Unpin and close files before freeing storage",
                ));
            }
            if entry.node["type"] == "folder" {
                if let Some(children) = self.tree.children.get(&id) {
                    pending.extend(children.values().copied());
                }
            } else {
                prefixes.insert(format!(
                    "{:x}-",
                    Sha256::digest(field(&entry.node, "uid").as_bytes())
                ));
            }
        }
        if prefixes.is_empty() {
            return Ok(());
        }
        for entry in std::fs::read_dir(&self.objects)? {
            let path = entry?.path();
            let name = path.file_name().unwrap().to_string_lossy();
            if !name
                .get(..65)
                .is_some_and(|prefix| prefixes.contains(prefix))
            {
                continue;
            }
            if name.len() != 129
                || !name.as_bytes()[65..]
                    .iter()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
                || path.is_symlink()
            {
                return Err(fail(libc::EACCES, "Unsafe cache entry"));
            }
            io::regular(&path, libc::O_RDONLY)?;
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
    fn close(&mut self) {
        if self.closed {
            return;
        }
        for id in self.files.keys().copied().collect::<Vec<_>>() {
            if self.release(id).is_err() {
                self.error =
                    "Local recovery metadata could not be updated; preserve the cache directory."
                        .into();
            }
        }
        if self.save().is_err() {
            self.error = "Mount metadata could not be saved; preserve the cache directory.".into();
        }
        self.closed = true;
    }
    fn apply(&mut self, action: Action, host: &mut impl Services) -> Result<Value> {
        if self.closed {
            return Err(fail(libc::EBADF, "Mount is closed"));
        }
        Ok(match action {
            Action::Snapshot => {
                json!({"tree":self.tree,"handles":self.files.keys().collect::<Vec<_>>()})
            }
            Action::Invalidate => {
                self.generation = self.generation.wrapping_add(1);
                self.loaded.clear();
                // Cloud events keep pinned content current even when no file
                // manager is browsing the directory.
                // Scan metadata once instead of searching it again for every pin.
                let mut roots: Vec<_> = if self.tree.pinned.is_empty() {
                    Vec::new()
                } else {
                    self.tree
                        .entries
                        .iter()
                        .filter(|entry| {
                            self.tree.pinned.contains(&entry.inode)
                                && (entry.inode == 1 || !self.tree.pinned.contains(&entry.parent))
                        })
                        .map(|entry| entry.inode)
                        .collect()
                };
                roots.sort_unstable();
                for inode in roots {
                    match self.sync_pin_tree(inode, host) {
                        Ok(()) => {}
                        Err(Error::Fs(libc::EBUSY, message)) => {
                            self.cloud_errors.insert(
                                (inode, "pin"),
                                format!("Pinned refresh is pending. {message}."),
                            );
                        }
                        Err(Error::Backend(error)) if !error.cancelled => {
                            // Invalidation must still succeed while disconnected;
                            // verified snapshots remain available for read-only opens.
                            self.cloud_failure(inode, "pin", "Cloud unavailable; pinned refresh is pending. Verified files remain readable.", &error);
                        }
                        Err(error) => return Err(error),
                    }
                }
                json!({"pending":self.cloud_errors.keys().any(|(_, operation)| *operation == "pin")})
            }
            Action::BlockAccount => {
                self.blocked = true;
                self.error = BLOCKED.into();
                Value::Null
            }
            Action::SetError { message } => {
                self.error = message;
                Value::Null
            }
            Action::SetWritable { writable } => {
                if writable && self.config.offline {
                    return Err(fail(libc::EROFS, "Offline mounts are read-only"));
                }
                self.config.writable = writable;
                Value::Null
            }
            Action::Refresh { inode } => self.refresh(inode, host)?,
            Action::Listdir { inode, refresh } => self.listdir(inode, refresh, host)?,
            Action::Lookup { parent, name } => json!(self.lookup(parent, &name, host)?),
            Action::Metadata { inode } => {
                let entry = self.entry(inode)?;
                let mut size = entry.node["size"].as_u64().unwrap_or(0);
                for f in self.files.values() {
                    if f.inode == inode && f.writable {
                        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
                        if unsafe {
                            libc::fstat(f.native["fd"].as_i64().unwrap() as i32, stat.as_mut_ptr())
                        } < 0
                        {
                            return Err(std::io::Error::last_os_error().into());
                        }
                        size = unsafe { stat.assume_init() }.st_size as u64;
                        break;
                    }
                }
                json!({"inode":inode,"folder":entry.node["type"]=="folder","size":size,"mtime_ns":timestamp(field(&entry.node,"modified")),"cached":self.objects.join(io::key(&entry.node)?).is_file(),"pinned":self.tree.pinned.contains(&inode),"writable":self.config.writable})
            }
            Action::Pin { inode, pinned } => {
                self.entry(inode)?;
                if pinned {
                    self.check_root(host)?;
                    self.sync_pin_tree(inode, host)?;
                } else {
                    let mut pending = vec![inode];
                    let mut visited = BTreeSet::new();
                    while let Some(id) = pending.pop() {
                        if !visited.insert(id) {
                            continue;
                        }
                        self.tree.pinned.remove(&id);
                        self.cloud_recovered(id, "pin");
                        if let Some(children) = self.tree.children.get(&id) {
                            pending.extend(children.values().copied());
                        }
                    }
                    self.save()?;
                }
                Value::Null
            }
            Action::Evict { inode } => {
                self.evict(inode)?;
                Value::Null
            }
            Action::Open { inode, flags } => json!(self.open(inode, flags, host)?),
            Action::Read {
                handle,
                offset,
                size,
            } => {
                self.file(handle)?;
                if offset < 0 || size < 0 {
                    return Err(fail(libc::EINVAL, "Invalid read range"));
                }
                handles::apply(H::Read {
                    id: handle,
                    offset,
                    size: size as u64,
                })?
            }
            Action::Write {
                handle,
                offset,
                content,
            } => self.write(handle, offset, content)?,
            Action::Truncate {
                inode,
                size,
                handle,
            } => {
                self.truncate(inode, size, handle, host)?;
                Value::Null
            }
            Action::Create {
                parent,
                name,
                flags,
            } => self.create(parent, &name, flags, host)?,
            Action::Flush { handle } => {
                self.flush(handle, host)?;
                Value::Null
            }
            Action::Release { handle } => {
                self.release(handle)?;
                Value::Null
            }
            Action::DrainDeletes => {
                if !self.blocked && !self.config.offline {
                    crate::ondemand_delete::drain(
                        &self.deletions,
                        &self.config.remote_root,
                        &self.objects,
                        host,
                    )?;
                }
                Value::Null
            }
            Action::Recovery => self.recovery()?,
            Action::Mkdir { parent, name } => {
                let id = self
                    .namespace(json!({"action":"mkdir","parent":parent,"name":name}), host)?
                    .as_u64()
                    .unwrap();
                json!(self.entry(id)?)
            }
            Action::Rename {
                parent,
                name,
                new_parent,
                new_name,
                no_replace,
            } => {
                self.writable()?;
                let source = self.lookup(parent, &name, host)?.inode;
                self.namespace(json!({"action":"rename","source":source,"parent":new_parent,"name":new_name,"no_replace":no_replace}),host)?
            }
            Action::Unlink {
                parent,
                name,
                directory,
            } => {
                self.writable()?;
                let source = self.lookup(parent, &name, host)?.inode;
                self.namespace(
                    json!({"action":"unlink","source":source,"directory":directory}),
                    host,
                )?
            }
        })
    }
}
impl Drop for Mount {
    fn drop(&mut self) {
        self.close();
    }
}
pub(crate) fn timestamp(value: &str) -> i64 {
    fn parse(value: &str) -> Option<i64> {
        let input = std::ffi::CString::new(value).ok()?;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_isdst = -1;
        let end = unsafe { libc::strptime(input.as_ptr(), c"%Y-%m-%dT%H:%M:%S".as_ptr(), &mut tm) };
        if end.is_null() {
            return None;
        }
        let mut tail = unsafe { std::ffi::CStr::from_ptr(end) }.to_str().ok()?;
        let mut nanos = 0i64;
        if let Some(after) = tail.strip_prefix('.') {
            let count = after.bytes().take_while(|b| b.is_ascii_digit()).count();
            if count == 0 {
                return None;
            }
            let digits = &after[..count.min(9)];
            nanos = digits.parse::<i64>().ok()? * 10i64.pow(9 - digits.len() as u32);
            tail = &after[count..];
        }
        let seconds = if tail.is_empty() {
            unsafe { libc::mktime(&mut tm) }
        } else {
            let offset = if tail == "Z" {
                0
            } else {
                let sign = match tail.as_bytes().first()? {
                    b'+' => 1,
                    b'-' => -1,
                    _ => return None,
                };
                let rest = &tail[1..];
                let (hours, minutes) = rest.split_once(':')?;
                if hours.len() != 2 || minutes.len() != 2 {
                    return None;
                }
                let h = hours.parse::<i64>().ok()?;
                let m = minutes.parse::<i64>().ok()?;
                if h > 23 || m > 59 {
                    return None;
                }
                sign * (h * 3600 + m * 60)
            };
            (unsafe { libc::timegm(&mut tm) }) - offset
        };
        seconds.checked_mul(1_000_000_000)?.checked_add(nanos)
    }
    parse(value).unwrap_or(0)
}
fn envelope(result: Result<Value>, mount: Option<&mut Mount>) -> Value {
    let mut value = json!({});
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
    if let Some(mount) = mount {
        if value["errno"] == libc::EACCES
            && field(&value, "message").contains("Cloud account or mount root changed.")
        {
            mount.blocked = true;
            mount.error = BLOCKED.into();
        }
        value["status"] = mount.status();
    }
    value
}
pub fn run(operation: Operation, host: &mut impl Services) -> std::result::Result<Value, Failure> {
    if let Operation::Call {
        id,
        operation:
            Action::Unlink {
                parent,
                name,
                directory,
            },
    } = &operation
    {
        return Ok(envelope(
            enqueue_delete(*id, *parent, name, *directory),
            None,
        ));
    }
    Ok(match operation {
        Operation::Create { config } => match Mount::new(config, host) {
            Ok(mount) => {
                mount.publish(true);
                let status = mount.status();
                let id = NEXT.fetch_add(1, Ordering::Relaxed);
                SNAPSHOTS.lock().unwrap().insert(id, mount.snapshot.clone());
                MUTATIONS.lock().unwrap().insert(id, mount.mutation.clone());
                MOUNTS
                    .lock()
                    .unwrap()
                    .insert(id, Arc::new(Mutex::new(mount)));
                json!({"value":id,"status":status})
            }
            Err(e) => envelope(Err(e), None),
        },
        Operation::Close { id } => {
            SNAPSHOTS.lock().unwrap().remove(&id);
            MUTATIONS.lock().unwrap().remove(&id);
            let mount = MOUNTS.lock().unwrap().remove(&id);
            match mount {
                Some(mount) => {
                    let mut mount = mount.lock().unwrap();
                    mount.close();
                    json!({"value":null,"status":mount.status()})
                }
                None => json!({"value":null}),
            }
        }
        Operation::TryRecovery { id } => {
            let mount = MOUNTS.lock().unwrap().get(&id).cloned();
            match mount {
                None => envelope(Err(fail(libc::EBADF, "Unknown mount")), None),
                Some(mount) => match mount.try_lock() {
                    Ok(mut mount) => {
                        let result = mount.recovery();
                        envelope(result, Some(&mut mount))
                    }
                    Err(std::sync::TryLockError::WouldBlock) => json!({"busy":true}),
                    Err(_) => return Err("Mount lock poisoned".into()),
                },
            }
        }
        Operation::Call { id, operation } => {
            let mount = MOUNTS.lock().unwrap().get(&id).cloned();
            match mount {
                None => envelope(Err(fail(libc::EBADF, "Unknown mount")), None),
                Some(mount) => {
                    let mut mount = mount.lock().unwrap();
                    let publish = !matches!(
                        operation,
                        Action::Metadata { .. }
                            | Action::Snapshot
                            | Action::Recovery
                            | Action::Read { .. }
                            | Action::Listdir { .. }
                            | Action::Lookup { .. }
                    );
                    let gate = mount.mutation.clone();
                    let _guard = if matches!(
                        operation,
                        Action::Open { .. }
                            | Action::Create { .. }
                            | Action::Rename { .. }
                            | Action::Mkdir { .. }
                            | Action::Write { .. }
                            | Action::Truncate { .. }
                            | Action::Flush { .. }
                            | Action::Release { .. }
                            | Action::BlockAccount
                            | Action::SetWritable { .. }
                    ) {
                        Some(gate.lock().unwrap())
                    } else {
                        None
                    };
                    let result = mount.apply(operation, host);
                    if publish {
                        mount.publish(true);
                    }
                    envelope(result, Some(&mut mount))
                }
            }
        }
    })
}

static MUTATIONS: LazyLock<Mutex<BTreeMap<u64, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
// RELEASE and other short local mutations can overlap a kernel unlink. Allow
// that handoff to finish, but never wait indefinitely for a cloud-backed write.
fn deletion_guard(gate: &Mutex<()>) -> Result<std::sync::MutexGuard<'_, ()>> {
    let started = Instant::now();
    loop {
        match gate.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(fail(libc::EIO, "File mutation lock unavailable"))
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                if started.elapsed() >= std::time::Duration::from_millis(250) {
                    return Err(fail(
                        libc::EBUSY,
                        "A file change is in progress; retry deletion shortly",
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }
}
pub(crate) fn enqueue_delete(id: u64, parent: u64, name: &str, directory: bool) -> Result<Value> {
    tree::name(name)?;
    let gate = MUTATIONS
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| fail(libc::EBADF, "Unknown mount"))?;
    let _guard = deletion_guard(&gate)?;
    let mirror = SNAPSHOTS
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| fail(libc::EBADF, "Unknown mount"))?;
    let s = mirror.read().unwrap();
    if s.blocked || !s.writable {
        return Err(fail(libc::EROFS, "Mount is read-only"));
    }
    let inode = s
        .tree
        .children
        .get(&parent)
        .and_then(|c| c.get(name))
        .ok_or_else(|| fail(libc::ENOENT, "No such file"))?;
    let entry = s
        .entries
        .get(inode)
        .ok_or_else(|| fail(libc::ENOENT, "No such file"))?;
    if entry.local || s.open.contains(inode) {
        return Err(fail(libc::EBUSY, "Close this file before deleting it"));
    }
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
    let queue = s
        .deletions
        .as_ref()
        .ok_or_else(|| fail(libc::EIO, "Deletion queue unavailable"))?;
    let mut q = queue.lock().unwrap();
    if directory
        && (!s.tree.listed.contains(inode)
            || s.tree.children.get(inode).is_some_and(|c| {
                c.values()
                    .any(|id| s.entries.get(id).is_none_or(|e| !q.hidden(&e.node)))
            }))
    {
        return Err(fail(
            libc::ENOTEMPTY,
            "Directory is not empty or has not been listed",
        ));
    }
    q.enqueue(
        entry.clone(),
        field(
            &s.entries
                .get(&parent)
                .ok_or_else(|| fail(libc::ENOENT, "Parent missing"))?
                .node,
            "path",
        )
        .into(),
    )?;
    drop(q);
    if let Ok(key) = io::key(&entry.node) {
        let _ = std::fs::remove_file(s.objects.join(key));
    }
    Ok(Value::Null)
}

// Readers never acquire the mount/SDK lock. A cloud timeout cannot block cached browsing.
type SharedSnapshot = Arc<RwLock<crate::ondemand_snapshot::Snapshot>>;
static SNAPSHOTS: LazyLock<Mutex<BTreeMap<u64, SharedSnapshot>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
pub(crate) fn cached(id: u64, action: &Action) -> Option<Value> {
    let snapshot = SNAPSHOTS.lock().ok()?.get(&id)?.clone();
    let value = snapshot.read().ok()?.answer(action);
    value
}
pub(crate) fn absent(id: u64, action: &Action) -> bool {
    let Action::Lookup { parent, name } = action else {
        return false;
    };
    if matches!(name.as_str(), "." | "..") {
        return false;
    }
    let Some(snapshot) = SNAPSHOTS.lock().unwrap().get(&id).cloned() else {
        return false;
    };
    let s = snapshot.read().unwrap();
    !s.blocked
        && (s
            .tree
            .children
            .get(parent)
            .and_then(|c| c.get(name))
            .and_then(|id| s.entries.get(id))
            .is_some_and(|e| s.hidden(&e.node))
            || (s.tree.listed.contains(parent)
                && s.tree
                    .children
                    .get(parent)
                    .is_none_or(|c| !c.contains_key(name))))
}
pub(crate) fn deletions_ready(id: u64) -> bool {
    let Some(mirror) = SNAPSHOTS.lock().unwrap().get(&id).cloned() else {
        return false;
    };
    let snapshot = mirror.read().unwrap();
    !snapshot.blocked
        && !snapshot.offline
        && snapshot
            .deletions
            .as_ref()
            .is_some_and(|q| q.lock().unwrap().ready())
}
pub(crate) fn activity(id: u64) -> Value {
    let Some(mirror) = SNAPSHOTS.lock().unwrap().get(&id).cloned() else {
        return json!({});
    };
    let snapshot = mirror.read().unwrap();
    json!({"deletions":snapshot.deletions.as_ref().map(|q|q.lock().unwrap().status()).unwrap_or(json!({})),"downloads":snapshot.downloading.iter().filter_map(|id|snapshot.entries.get(id).map(|e|e.node["name"].clone())).collect::<Vec<_>>()})
}
pub(crate) fn generation(id: u64) -> u64 {
    let Some(snapshot) = SNAPSHOTS.lock().unwrap().get(&id).cloned() else {
        return 0;
    };
    let value = snapshot.read().unwrap().generation;
    value
}
pub(crate) fn folders(id: u64) -> Vec<u64> {
    let Some(snapshot) = SNAPSHOTS.lock().unwrap().get(&id).cloned() else {
        return Vec::new();
    };
    let s = snapshot.read().unwrap();
    let mut folders: Vec<_> = s
        .entries
        .values()
        .filter(|e| e.node["type"] == "folder" && !s.hidden(&e.node))
        .map(|e| e.inode)
        .collect();
    folders.sort_by_key(|id| (s.tree.listed.contains(id), *id));
    folders
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nd-mount-{}-{}",
                std::process::id(),
                token().unwrap()
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
    struct Host {
        content: Vec<u8>,
        revision: u64,
        online: bool,
        uploads: usize,
        fail_download: bool,
    }
    impl Host {
        fn new() -> Self {
            Self {
                content: b"abc".to_vec(),
                revision: 1,
                online: true,
                uploads: 0,
                fail_download: false,
            }
        }
        fn node(&self) -> Value {
            use sha1::Sha1;
            json!({"uid":"data","name":"data","type":"file","safe":true,"size":self.content.len(),"sha1":format!("{:x}",Sha1::digest(&self.content)),"revision":format!("r{}",self.revision),"path":"/my-files/root/data"})
        }
    }
    impl Services for Host {
        fn poll(&mut self) -> std::result::Result<(), Failure> {
            Ok(())
        }
        fn request(&mut self, method: &str, args: Value) -> std::result::Result<Value, Failure> {
            if !self.online {
                return Err("synthetic network outage".into());
            }
            match method {
                "info" if args["path"] == "/my-files" => Ok(
                    json!({"uid":"root","name":"My files","type":"folder","safe":true,"path":"/my-files"}),
                ),
                "info" => Ok(self.node()),
                "list_nodes" => Ok(json!([self.node()])),
                "download" => {
                    if self.fail_download {
                        return Err("synthetic download interruption".into());
                    }
                    std::fs::write(
                        PathBuf::from(args["destination"].as_str().unwrap()).join("data"),
                        &self.content,
                    )
                    .unwrap();
                    Ok(Value::Null)
                }
                "upload" => {
                    self.content = std::fs::read(args["paths"][0].as_str().unwrap()).unwrap();
                    self.revision += 1;
                    self.uploads += 1;
                    Ok(Value::Null)
                }
                _ => panic!("unexpected service {method}"),
            }
        }
    }
    fn config(temp: &Temp, offline: bool) -> Config {
        Config {
            cache: temp.0.clone(),
            remote_root: "/my-files".into(),
            writable: !offline,
            offline,
            metadata_ttl: 15.0,
        }
    }
    #[test]
    fn deletion_waits_for_brief_close_handoff_but_bounds_busy_writes() {
        let gate = Arc::new(Mutex::new(()));
        let worker_gate = gate.clone();
        let (ready, started) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _guard = worker_gate.lock().unwrap();
            ready.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
        });
        started.recv().unwrap();
        drop(deletion_guard(&gate).unwrap());
        worker.join().unwrap();
        let _guard = gate.lock().unwrap();
        let start = Instant::now();
        assert!(matches!(
            deletion_guard(&gate),
            Err(Error::Fs(libc::EBUSY, _))
        ));
        assert!(start.elapsed().as_secs_f64() < 1.0);
    }
    #[test]
    fn local_delete_does_not_wait_for_mount_or_sdk_and_survives_restart() {
        let temp = Temp::new();
        let mut host = Host::new();
        let id = run(
            Operation::Create {
                config: config(&temp, false),
            },
            &mut host,
        )
        .unwrap()["value"]
            .as_u64()
            .unwrap();
        run(
            Operation::Call {
                id,
                operation: Action::Listdir {
                    inode: 1,
                    refresh: true,
                },
            },
            &mut host,
        )
        .unwrap();
        // A successful list must publish the latest mirror for the fast path.
        let mount = MOUNTS.lock().unwrap().get(&id).unwrap().clone();
        let guard = mount.lock().unwrap();
        guard.publish(false);
        let started = Instant::now();
        enqueue_delete(id, 1, "data", false).unwrap();
        assert!(started.elapsed().as_secs_f64() < 1.0);
        assert!(absent(
            id,
            &Action::Lookup {
                parent: 1,
                name: "data".into()
            }
        ));
        assert_eq!(
            cached(
                id,
                &Action::Listdir {
                    inode: 1,
                    refresh: false
                }
            )
            .unwrap(),
            json!([])
        );
        drop(guard);
        drop(mount);
        run(Operation::Close { id }, &mut host).unwrap();
        let id = run(
            Operation::Create {
                config: config(&temp, false),
            },
            &mut host,
        )
        .unwrap()["value"]
            .as_u64()
            .unwrap();
        assert!(absent(
            id,
            &Action::Lookup {
                parent: 1,
                name: "data".into()
            }
        ));
        run(Operation::Close { id }, &mut host).unwrap();
    }
    #[test]
    fn open_file_rejects_delete_without_creating_a_journal_entry() {
        let temp = Temp::new();
        let mut host = Host::new();
        let id = run(
            Operation::Create {
                config: config(&temp, false),
            },
            &mut host,
        )
        .unwrap()["value"]
            .as_u64()
            .unwrap();
        let entry = run(
            Operation::Call {
                id,
                operation: Action::Lookup {
                    parent: 1,
                    name: "data".into(),
                },
            },
            &mut host,
        )
        .unwrap()["value"]
            .clone();
        run(
            Operation::Call {
                id,
                operation: Action::Open {
                    inode: entry["inode"].as_u64().unwrap(),
                    flags: libc::O_RDONLY,
                },
            },
            &mut host,
        )
        .unwrap();
        assert!(matches!(
            enqueue_delete(id, 1, "data", false),
            Err(Error::Fs(libc::EBUSY, _))
        ));
        run(Operation::Close { id }, &mut host).unwrap();
    }
    #[test]
    fn folder_eviction_preserves_busy_files_and_unrelated_cache() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        let inode = mount.lookup(1, "data", &mut host).unwrap().inode;
        mount
            .apply(
                Action::Pin {
                    inode,
                    pinned: true,
                },
                &mut host,
            )
            .unwrap();
        let mut sibling = host.node();
        sibling["uid"] = json!("sibling");
        sibling["name"] = json!("z-sibling");
        sibling["path"] = json!("/my-files/z-sibling");
        mount.tree = tree::reconcile(
            mount.tree.clone(),
            1,
            vec![host.node(), sibling.clone()],
            BTreeSet::new(),
        )
        .unwrap();
        let sibling_path = mount.objects.join(io::key(&sibling).unwrap());
        let current = mount.objects.join(io::key(&host.node()).unwrap());
        let mut old = host.node();
        old["revision"] = json!("older");
        let old_path = mount.objects.join(io::key(&old).unwrap());
        let mut unrelated = host.node();
        unrelated["uid"] = json!("unrelated");
        let unrelated_path = mount.objects.join(io::key(&unrelated).unwrap());
        for path in [&old_path, &unrelated_path, &sibling_path] {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .unwrap();
        }
        assert!(matches!(
            mount.apply(Action::Evict { inode: 1 }, &mut host),
            Err(Error::Fs(libc::EBUSY, _))
        ));
        assert!(current.exists() && old_path.exists() && sibling_path.exists());
        mount
            .apply(
                Action::Pin {
                    inode,
                    pinned: false,
                },
                &mut host,
            )
            .unwrap();
        let handle = mount.open(inode, libc::O_RDONLY, &mut host).unwrap();
        assert!(matches!(
            mount.apply(Action::Evict { inode: 1 }, &mut host),
            Err(Error::Fs(libc::EBUSY, _))
        ));
        assert!(current.exists() && old_path.exists() && sibling_path.exists());
        mount.release(handle).unwrap();
        mount.apply(Action::Evict { inode: 1 }, &mut host).unwrap();
        assert!(!current.exists() && !old_path.exists() && !sibling_path.exists());
        assert!(unrelated_path.exists());
        assert!(mount.entry(inode).is_ok());
        assert_eq!(host.uploads, 0);
    }

    #[test]
    fn interrupted_first_file_pin_survives_reopen_and_retries() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        let inode = mount.lookup(1, "data", &mut host).unwrap().inode;
        host.fail_download = true;
        assert!(mount
            .apply(
                Action::Pin {
                    inode,
                    pinned: true
                },
                &mut host
            )
            .is_err());
        assert!(mount.tree.pinned.contains(&inode));
        assert!(!mount.objects.join(io::key(&host.node()).unwrap()).exists());
        drop(mount);
        host.fail_download = false;
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        assert!(mount.tree.pinned.contains(&inode));
        assert_eq!(
            mount.apply(Action::Invalidate, &mut host).unwrap()["pending"],
            false
        );
        assert_eq!(
            std::fs::read(mount.objects.join(io::key(&host.node()).unwrap())).unwrap(),
            b"abc"
        );
        assert_eq!(host.uploads, 0);
    }

    #[test]
    fn metadata_outage_recovers_without_hiding_pending_pin_or_write_errors() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        mount
            .apply(
                Action::Pin {
                    inode: 1,
                    pinned: true,
                },
                &mut host,
            )
            .unwrap();
        host.online = false;
        mount.apply(Action::Invalidate, &mut host).unwrap();
        mount.listdir(1, true, &mut host).unwrap();
        assert!(mount.status()["last_error"]
            .as_str()
            .unwrap()
            .contains("synthetic network outage"));
        host.online = true;
        mount.refresh_snapshot(1, &mut host).unwrap();
        // Listing metadata alone has not verified the pinned file revisions.
        assert!(mount.status()["last_error"]
            .as_str()
            .unwrap()
            .contains("pinned refresh is pending"));
        host.content = b"changed during outage".to_vec();
        host.revision = 2;
        mount.apply(Action::Invalidate, &mut host).unwrap();
        assert_eq!(mount.status()["last_error"], "");
        let inode = mount.lookup(1, "data", &mut host).unwrap().inode;
        assert_eq!(mount.entry(inode).unwrap().node["revision"], "r2");
        let file = io::verified(
            &mount.objects.join(io::key(&host.node()).unwrap()),
            &host.node(),
        )
        .unwrap();
        assert_eq!(file.metadata().unwrap().len(), host.content.len() as u64);
        mount.error = WRITE_FAILED.into();
        mount.apply(Action::Invalidate, &mut host).unwrap();
        assert_eq!(mount.status()["last_error"], WRITE_FAILED);
        assert_eq!(host.uploads, 0);
    }

    #[test]
    fn unpin_clears_only_the_cancelled_pin_refresh_warning() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        mount
            .apply(
                Action::Pin {
                    inode: 1,
                    pinned: true,
                },
                &mut host,
            )
            .unwrap();
        host.online = false;
        assert_eq!(
            mount.apply(Action::Invalidate, &mut host).unwrap()["pending"],
            true
        );
        mount.error = WRITE_FAILED.into();
        mount
            .apply(
                Action::Pin {
                    inode: 1,
                    pinned: false,
                },
                &mut host,
            )
            .unwrap();
        assert_eq!(
            mount.apply(Action::Invalidate, &mut host).unwrap()["pending"],
            false
        );
        assert_eq!(mount.status()["last_error"], WRITE_FAILED);
        assert!(!mount
            .cloud_errors
            .keys()
            .any(|(_, operation)| *operation == "pin"));
    }

    #[test]
    fn changed_open_pin_is_retried_after_close_without_replacing_open_bytes() {
        for flags in [libc::O_RDONLY, libc::O_RDWR] {
            let temp = Temp::new();
            let mut host = Host::new();
            let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
            mount
                .apply(
                    Action::Pin {
                        inode: 1,
                        pinned: true,
                    },
                    &mut host,
                )
                .unwrap();
            let inode = mount.lookup(1, "data", &mut host).unwrap().inode;
            let handle = mount.open(inode, flags, &mut host).unwrap();
            // An unchanged open snapshot must not cause a needless pending refresh.
            assert_eq!(
                mount.apply(Action::Invalidate, &mut host).unwrap()["pending"],
                false
            );
            host.content = b"new".to_vec();
            host.revision = 2;
            assert_eq!(
                mount.apply(Action::Invalidate, &mut host).unwrap()["pending"],
                true
            );
            assert_eq!(mount.entry(inode).unwrap().node["revision"], "r1");
            assert_eq!(
                mount
                    .apply(
                        Action::Read {
                            handle,
                            offset: 0,
                            size: 32
                        },
                        &mut host
                    )
                    .unwrap(),
                "616263"
            );
            mount.release(handle).unwrap();
            assert_eq!(
                mount.apply(Action::Invalidate, &mut host).unwrap()["pending"],
                false
            );
            assert_eq!(mount.status()["last_error"], "");
            assert_eq!(mount.entry(inode).unwrap().node["revision"], "r2");
            let handle = mount.open(inode, libc::O_RDONLY, &mut host).unwrap();
            assert_eq!(
                mount
                    .apply(
                        Action::Read {
                            handle,
                            offset: 0,
                            size: 32
                        },
                        &mut host
                    )
                    .unwrap(),
                "6e6577"
            );
            mount.release(handle).unwrap();
            assert_eq!(host.uploads, 0);
        }
    }

    #[test]
    fn parent_pin_recovery_clears_a_previously_failed_child_refresh() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        let inode = mount.lookup(1, "data", &mut host).unwrap().inode;
        mount
            .apply(
                Action::Pin {
                    inode,
                    pinned: true,
                },
                &mut host,
            )
            .unwrap();
        host.online = false;
        assert_eq!(
            mount.apply(Action::Invalidate, &mut host).unwrap()["pending"],
            true
        );
        host.online = true;
        mount
            .apply(
                Action::Pin {
                    inode: 1,
                    pinned: true,
                },
                &mut host,
            )
            .unwrap();
        assert_eq!(mount.status()["last_error"], "");
        assert_eq!(
            mount.apply(Action::Invalidate, &mut host).unwrap()["pending"],
            false
        );
        assert_eq!(host.uploads, 0);
    }

    #[test]
    fn successful_leaf_refresh_does_not_clear_pending_parent_refresh() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        mount
            .apply(
                Action::Pin {
                    inode: 1,
                    pinned: true,
                },
                &mut host,
            )
            .unwrap();
        let inode = mount.lookup(1, "data", &mut host).unwrap().inode;
        host.online = false;
        assert_eq!(
            mount.apply(Action::Invalidate, &mut host).unwrap()["pending"],
            true
        );
        host.online = true;
        mount
            .apply(
                Action::Pin {
                    inode,
                    pinned: true,
                },
                &mut host,
            )
            .unwrap();
        assert!(mount.status()["last_error"]
            .as_str()
            .unwrap()
            .contains("pinned refresh is pending"));
        assert_eq!(
            mount.apply(Action::Invalidate, &mut host).unwrap()["pending"],
            false
        );
        assert_eq!(mount.status()["last_error"], "");
    }

    #[test]
    fn cached_metadata_warning_clears_after_its_directory_recovers() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        mount.listdir(1, true, &mut host).unwrap();
        host.online = false;
        mount.loaded.clear();
        mount.listdir(1, true, &mut host).unwrap();
        assert!(!mount.status()["last_error"].as_str().unwrap().is_empty());
        host.online = true;
        mount.refresh_snapshot(1, &mut host).unwrap();
        assert_eq!(mount.status()["last_error"], "");
    }

    #[test]
    fn verified_cached_read_warning_recovers_after_online_read() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        mount
            .apply(
                Action::Pin {
                    inode: 1,
                    pinned: true,
                },
                &mut host,
            )
            .unwrap();
        let inode = mount.lookup(1, "data", &mut host).unwrap().inode;
        host.online = false;
        let file = mount.open(inode, libc::O_RDONLY, &mut host).unwrap();
        mount.release(file).unwrap();
        assert!(mount.status()["last_error"]
            .as_str()
            .unwrap()
            .contains("verified pinned snapshot"));
        host.online = true;
        let file = mount.open(inode, libc::O_RDONLY, &mut host).unwrap();
        mount.release(file).unwrap();
        assert_eq!(mount.status()["last_error"], "");
    }

    #[test]
    fn cached_browsing_does_not_wait_for_sdk_lock() {
        let temp = Temp::new();
        let mut host = Host::new();
        let created = run(
            Operation::Create {
                config: config(&temp, false),
            },
            &mut host,
        )
        .unwrap();
        let id = created["value"].as_u64().unwrap();
        run(
            Operation::Call {
                id,
                operation: Action::Pin {
                    inode: 1,
                    pinned: true,
                },
            },
            &mut host,
        )
        .unwrap();
        let shared = MOUNTS.lock().unwrap().get(&id).unwrap().clone();
        let guard = shared.lock().unwrap(); // Simulate an indefinitely blocked network operation.
        let started = Instant::now();
        assert_eq!(
            cached(
                id,
                &Action::Listdir {
                    inode: 1,
                    refresh: true
                }
            )
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
            1
        );
        let e = cached(
            id,
            &Action::Lookup {
                parent: 1,
                name: "data".into(),
            },
        )
        .unwrap();
        assert_eq!(
            cached(
                id,
                &Action::Metadata {
                    inode: e["inode"].as_u64().unwrap()
                }
            )
            .unwrap()["cached"],
            true
        );
        assert!(started.elapsed().as_millis() < 200);
        let status: Value =
            serde_json::from_slice(&std::fs::read(temp.0.join("availability.json")).unwrap())
                .unwrap();
        assert_eq!(status["available"]["data"], true);
        assert_eq!(status["available"][""], true);
        drop(guard);
        run(Operation::Close { id }, &mut host).unwrap();
        assert!(cached(id, &Action::Metadata { inode: 1 }).is_none());
    }
    #[test]
    fn pin_progress_is_published_before_download_and_cleared_on_success_or_failure() {
        struct ProgressHost {
            inner: Host,
            cache: PathBuf,
            fail: bool,
            observed: bool,
        }
        impl Services for ProgressHost {
            fn poll(&mut self) -> std::result::Result<(), Failure> {
                Ok(())
            }
            fn request(
                &mut self,
                method: &str,
                args: Value,
            ) -> std::result::Result<Value, Failure> {
                if method == "download" {
                    let status: Value = serde_json::from_slice(
                        &std::fs::read(self.cache.join("availability.json")).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(status["downloading"][""], true);
                    assert_eq!(status["downloading"]["data"], true);
                    assert!(status["available"].as_object().unwrap().is_empty());
                    self.observed = true;
                    if self.fail {
                        return Err(Failure {
                            message: "cancelled transfer".into(),
                            cancelled: true,
                        });
                    }
                }
                self.inner.request(method, args)
            }
        }
        for fail in [false, true] {
            let temp = Temp::new();
            let mut host = ProgressHost {
                inner: Host::new(),
                cache: temp.0.clone(),
                fail,
                observed: false,
            };
            let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
            let result = mount.sync_pin_tree(1, &mut host);
            assert_eq!(result.is_err(), fail);
            assert!(host.observed);
            assert!(mount.downloading.is_empty());
            let status: Value =
                serde_json::from_slice(&std::fs::read(temp.0.join("availability.json")).unwrap())
                    .unwrap();
            assert!(status["downloading"].as_object().unwrap().is_empty());
            assert_eq!(status["available"]["data"] == true, !fail);
            assert_eq!(status["available"][""] == true, !fail);
        }
    }

    #[test]
    fn browser_refresh_does_not_download_pinned_descendants() {
        struct NoDownload(Host);
        impl Services for NoDownload {
            fn poll(&mut self) -> std::result::Result<(), Failure> {
                Ok(())
            }
            fn request(
                &mut self,
                method: &str,
                args: Value,
            ) -> std::result::Result<Value, Failure> {
                assert_ne!(
                    method, "download",
                    "Browsing must not download file contents"
                );
                self.0.request(method, args)
            }
        }
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        mount
            .apply(
                Action::Pin {
                    inode: 1,
                    pinned: true,
                },
                &mut host,
            )
            .unwrap();
        host.content = b"new revision".to_vec();
        host.revision = 2;
        mount.loaded.clear();
        mount.listdir(1, true, &mut NoDownload(host)).unwrap();
    }
    #[test]
    fn folder_pin_survives_restart_and_refreshes_revisions_without_opening_files() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        mount
            .apply(
                Action::Pin {
                    inode: 1,
                    pinned: true,
                },
                &mut host,
            )
            .unwrap();
        let inode = mount.lookup(1, "data", &mut host).unwrap().inode;
        assert!(mount.tree.pinned.contains(&1));
        assert!(mount.tree.pinned.contains(&inode));
        host.content = b"new cloud content".to_vec();
        host.revision = 2;
        mount.apply(Action::Invalidate, &mut host).unwrap();
        assert_eq!(mount.entry(inode).unwrap().node["revision"], "r2");
        assert_eq!(host.uploads, 0);
        mount.close();
        drop(mount);
        host.online = false;
        let mut offline = Mount::new(config(&temp, true), &mut host).unwrap();
        assert!(offline.tree.pinned.contains(&1));
        let file = offline.open(inode, libc::O_RDONLY, &mut host).unwrap();
        assert!(file > 0);
    }
    #[test]
    fn new_cloud_files_inherit_the_existing_folder_pin() {
        struct Growing {
            base: Host,
            added: bool,
        }
        impl Growing {
            fn later(&self) -> Value {
                let mut n = self.base.node();
                n["uid"] = json!("later");
                n["name"] = json!("later");
                n["path"] = json!("/my-files/later");
                n
            }
        }
        impl Services for Growing {
            fn poll(&mut self) -> std::result::Result<(), Failure> {
                Ok(())
            }
            fn request(
                &mut self,
                method: &str,
                args: Value,
            ) -> std::result::Result<Value, Failure> {
                if method == "list_nodes" && self.added {
                    return Ok(json!([self.base.node(), self.later()]));
                }
                if method == "info" && args["path"] == "/my-files/later" {
                    return Ok(self.later());
                }
                if method == "download" && args["paths"][0] == "/my-files/later" {
                    std::fs::write(
                        PathBuf::from(args["destination"].as_str().unwrap()).join("later"),
                        &self.base.content,
                    )
                    .unwrap();
                    return Ok(Value::Null);
                }
                self.base.request(method, args)
            }
        }
        let temp = Temp::new();
        let mut host = Growing {
            base: Host::new(),
            added: false,
        };
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        mount
            .apply(
                Action::Pin {
                    inode: 1,
                    pinned: true,
                },
                &mut host,
            )
            .unwrap();
        host.added = true;
        mount.apply(Action::Invalidate, &mut host).unwrap();
        let later = mount.lookup(1, "later", &mut host).unwrap();
        assert!(mount.tree.pinned.contains(&later.inode));
        assert!(mount.objects.join(io::key(&later.node).unwrap()).is_file());
        assert_eq!(host.base.uploads, 0);
    }
    #[test]
    fn unpin_folder_removes_descendant_policy_without_uploading_or_deleting_cloud_data() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        mount
            .apply(
                Action::Pin {
                    inode: 1,
                    pinned: true,
                },
                &mut host,
            )
            .unwrap();
        mount
            .apply(
                Action::Pin {
                    inode: 1,
                    pinned: false,
                },
                &mut host,
            )
            .unwrap();
        assert!(mount.tree.pinned.is_empty());
        assert_eq!(host.content, b"abc");
        assert_eq!(host.uploads, 0);
    }
    #[test]
    fn owned_mount_writeback_offline_pin_and_lock_lifecycle() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        assert!(matches!(
            Mount::new(config(&temp, false), &mut host),
            Err(Error::Fs(libc::EBUSY, _))
        ));
        let inode = mount.lookup(1, "data", &mut host).unwrap().inode;
        let handle = mount
            .open(inode, libc::O_RDWR | libc::O_TRUNC, &mut host)
            .unwrap();
        mount.write(handle, 0, "6e6577".into()).unwrap();
        mount.flush(handle, &mut host).unwrap();
        mount.release(handle).unwrap();
        assert_eq!(host.content, b"new");
        assert_eq!(host.uploads, 1);
        assert_eq!(mount.recovery().unwrap(), json!([]));
        mount
            .apply(
                Action::Pin {
                    inode,
                    pinned: true,
                },
                &mut host,
            )
            .unwrap();
        mount.close();
        drop(mount);
        host.online = false;
        let mut mount = Mount::new(config(&temp, true), &mut host).unwrap();
        let inode = mount.lookup(1, "data", &mut host).unwrap().inode;
        let handle = mount.open(inode, libc::O_RDONLY, &mut host).unwrap();
        assert_eq!(
            mount
                .apply(
                    Action::Read {
                        handle,
                        offset: 0,
                        size: 32
                    },
                    &mut host
                )
                .unwrap(),
            "6e6577"
        );
        mount.release(handle).unwrap();
        assert!(matches!(
            mount.open(inode, libc::O_RDWR, &mut host),
            Err(Error::Fs(libc::EROFS, _))
        ));
    }
    #[test]
    fn disk_full_create_rolls_back_without_phantom_inode_or_handle() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        mount.journal_fault = Some("all".into());
        assert!(matches!(
            mount.create(1, "cannot-create", libc::O_RDWR, &mut host),
            Err(Error::Fs(libc::ENOSPC, _))
        ));
        assert!(!mount.tree.children[&1].contains_key("cannot-create"));
        assert!(mount.files.is_empty());
        assert_eq!(std::fs::read_dir(&mount.recovery).unwrap().count(), 0);
        assert_eq!(host.uploads, 0);
    }
    #[test]
    fn failed_truncating_open_closes_new_handle_without_mutating_content() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        let inode = mount.lookup(1, "data", &mut host).unwrap().inode;
        mount.journal_fault = Some("unclean".into());
        assert!(matches!(
            mount.open(inode, libc::O_RDWR | libc::O_TRUNC, &mut host),
            Err(Error::Fs(libc::ENOSPC, _))
        ));
        assert!(mount.files.is_empty());
        assert_eq!(host.content, b"abc");
        assert_eq!(std::fs::read_dir(&mount.recovery).unwrap().count(), 0);
        mount.journal_fault = None;
        let handle = mount.open(inode, libc::O_RDWR, &mut host).unwrap();
        assert_eq!(
            mount
                .apply(
                    Action::Read {
                        handle,
                        offset: 0,
                        size: 100
                    },
                    &mut host
                )
                .unwrap(),
            "616263"
        );
        mount.release(handle).unwrap();
    }
    #[test]
    fn close_preserves_unflushed_edit_and_recovery_baseline() {
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        let inode = mount.lookup(1, "data", &mut host).unwrap().inode;
        let handle = mount
            .open(inode, libc::O_RDWR | libc::O_TRUNC, &mut host)
            .unwrap();
        mount.write(handle, 0, "65646974".into()).unwrap();
        mount.close();
        let recovery = handles::pending(&mount.recovery, &[]).unwrap();
        assert_eq!(recovery[0]["baseline"]["revision"], "r1");
        assert_eq!(
            std::fs::read(recovery[0]["data_path"].as_str().unwrap()).unwrap(),
            b"edit"
        );
        assert_eq!(host.uploads, 0);
    }
    #[test]
    fn account_change_stays_blocked_even_when_recovery_journal_fails() {
        struct WrongAccount<'a>(&'a mut Host);
        impl Services for WrongAccount<'_> {
            fn poll(&mut self) -> std::result::Result<(), Failure> {
                Ok(())
            }
            fn request(
                &mut self,
                method: &str,
                args: Value,
            ) -> std::result::Result<Value, Failure> {
                if method == "info" && args["path"] == "/my-files" {
                    return Ok(json!({"uid":"other-account","type":"folder"}));
                }
                self.0.request(method, args)
            }
        }
        let temp = Temp::new();
        let mut host = Host::new();
        let mut mount = Mount::new(config(&temp, false), &mut host).unwrap();
        let inode = mount.lookup(1, "data", &mut host).unwrap().inode;
        let handle = mount.open(inode, libc::O_RDWR, &mut host).unwrap();
        mount.write(handle, 0, "65646974".into()).unwrap();
        mount.journal_fault = Some("recovery".into());
        assert!(matches!(
            mount.flush(handle, &mut WrongAccount(&mut host)),
            Err(Error::Fs(libc::ENOSPC, _))
        ));
        assert!(mount.blocked);
        assert_eq!(host.uploads, 0);
        assert!(matches!(
            mount.open(inode, libc::O_RDONLY, &mut host),
            Err(Error::Fs(libc::EACCES, _))
        ));
        mount.journal_fault = None;
        mount.release(handle).unwrap();
        assert_eq!(mount.recovery().unwrap().as_array().unwrap().len(), 1);
    }
    #[test]
    fn timestamp_preserves_sdk_fraction_and_offset() {
        assert_eq!(timestamp("1970-01-01T00:00:00Z"), 0);
        assert_eq!(
            timestamp("1970-01-01T01:00:01.123456789+01:00"),
            1_123_456_789
        );
        assert_eq!(timestamp("1970-01-01T00:00:00-01:00"), 3_600_000_000_000);
        assert_eq!(timestamp("invalid"), 0);
        assert_eq!(timestamp("1970-01-01T00:00:00+99:99"), 0);
    }
}
