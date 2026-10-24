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
        Arc, LazyLock, Mutex,
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
    config: Config,
    objects: PathBuf,
    recovery: PathBuf,
    staging: PathBuf,
    _lock: File,
    tree: Tree,
    loaded: BTreeMap<u64, Instant>,
    files: BTreeMap<u64, OpenFile>,
    observed: BTreeMap<String, Value>,
    error: String,
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
        Ok(Self {
            config,
            objects,
            recovery,
            staging,
            _lock: lock,
            tree,
            loaded: BTreeMap::new(),
            files: BTreeMap::new(),
            observed: BTreeMap::new(),
            error: String::new(),
            blocked: false,
            closed: false,
            #[cfg(test)]
            journal_fault: None,
        })
    }
    fn status(&self) -> Value {
        json!({"last_error":self.error,"account_blocked":self.blocked,"writable":self.config.writable,"offline":self.config.offline,"root_uid":self.tree.entries.iter().find(|e|e.inode==1).map(|e|field(&e.node,"uid")).unwrap_or("")})
    }
    fn entry(&self, inode: u64) -> Result<Entry> {
        self.tree
            .entries
            .iter()
            .find(|e| e.inode == inode)
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
    fn save(&self) -> Result<()> {
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
        let mut pending = vec![root];
        let mut visited = BTreeSet::new();
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
                if self.open_inodes().contains(&inode) {
                    if !self.tree.pinned.contains(&inode) {
                        return Err(fail(
                            libc::EBUSY,
                            "Close the file before keeping it on this device",
                        ));
                    }
                    continue;
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
                self.hydrate(&current, host)?;
                if let Some(saved) = self.tree.entries.iter_mut().find(|e| e.inode == inode) {
                    saved.node = current;
                }
                self.tree.pinned.insert(inode);
                self.save()?;
            }
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
            match self.refresh(inode, host) {
                Ok(v) => return Ok(v),
                Err(Error::Backend(_)) if self.tree.listed.contains(&inode) => {
                    self.error =
                        "Cloud unavailable; showing saved metadata. Pinned files remain readable."
                            .into();
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
        if !self.config.offline && !entry.local {
            let current = (|| -> Result<Value> {
                self.check_root(host)?;
                let n = host.request("info", json!({"path":entry.node["path"]}))?;
                tree::node(&n)?;
                Ok(n)
            })();
            match current {
                Err(Error::Backend(_)) if !writable && self.tree.pinned.contains(&inode) => {
                    self.error = "Cloud unavailable; opening the verified pinned snapshot.".into();
                }
                Err(e) => return Err(e),
                Ok(current) => {
                    if revision(&current) != revision(&entry.node) {
                        if self.tree.pinned.contains(&inode) {
                            match self.hydrate(&current,host){Ok(_)=>entry.node=current,Err(Error::Backend(_)) if !writable=>self.error="New cloud revision unavailable; using the verified pinned snapshot.".into(),Err(e)=>return Err(e)}
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
                self.loaded.clear();
                // Cloud events keep pinned content current even when no file
                // manager is browsing the directory.
                let roots: Vec<_> = self
                    .tree
                    .pinned
                    .iter()
                    .copied()
                    .filter(|id| {
                        self.tree
                            .entries
                            .iter()
                            .find(|e| e.inode == *id)
                            .is_some_and(|e| e.inode == 1 || !self.tree.pinned.contains(&e.parent))
                    })
                    .collect();
                for inode in roots {
                    self.sync_pin_tree(inode, host)?;
                }
                Value::Null
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
                        if let Some(children) = self.tree.children.get(&id) {
                            pending.extend(children.values().copied());
                        }
                    }
                    self.save()?;
                }
                Value::Null
            }
            Action::Evict { inode } => {
                if self.entry(inode)?.node["type"] == "folder" {
                    let mut pending = vec![inode];
                    let mut visited = BTreeSet::new();
                    let mut files = Vec::new();
                    while let Some(id) = pending.pop() {
                        if !visited.insert(id) {
                            continue;
                        }
                        if self.tree.pinned.contains(&id)
                            || self.files.values().any(|f| f.inode == id)
                        {
                            return Err(fail(
                                libc::EBUSY,
                                "Unpin and close files before freeing folder storage",
                            ));
                        }
                        if self.entry(id)?.node["type"] == "folder" {
                            if let Some(children) = self.tree.children.get(&id) {
                                pending.extend(children.values().copied());
                            }
                        } else {
                            files.push(id);
                        }
                    }
                    for id in files {
                        self.apply(Action::Evict { inode: id }, host)?;
                    }
                    return Ok(Value::Null);
                }
                if self.tree.pinned.contains(&inode)
                    || self.files.values().any(|f| f.inode == inode)
                {
                    return Err(fail(
                        libc::EBUSY,
                        "Unpin and close the file before freeing its cache",
                    ));
                }
                let prefix = format!(
                    "{:x}-",
                    Sha256::digest(field(&self.entry(inode)?.node, "uid").as_bytes())
                );
                for entry in std::fs::read_dir(&self.objects)? {
                    let path = entry?.path();
                    let name = path.file_name().unwrap().to_string_lossy();
                    if !name.starts_with(&prefix) {
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
fn timestamp(value: &str) -> i64 {
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
    Ok(match operation {
        Operation::Create { config } => match Mount::new(config, host) {
            Ok(mount) => {
                let status = mount.status();
                let id = NEXT.fetch_add(1, Ordering::Relaxed);
                MOUNTS
                    .lock()
                    .unwrap()
                    .insert(id, Arc::new(Mutex::new(mount)));
                json!({"value":id,"status":status})
            }
            Err(e) => envelope(Err(e), None),
        },
        Operation::Close { id } => {
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
                    let result = mount.apply(operation, host);
                    envelope(result, Some(&mut mount))
                }
            }
        }
    })
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
    }
    impl Host {
        fn new() -> Self {
            Self {
                content: b"abc".to_vec(),
                revision: 1,
                online: true,
                uploads: 0,
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
