//! Native sync execution. Every mutation follows fresh proofs and durable intent.
use crate::{
    filesystem,
    host::Services,
    process::Failure,
    storage,
    sync_proof::{self, Plan},
    sync_remote::{self, Operation as Remote},
    sync_state::{self, Operation as State},
    trash_move, trash_restore,
};
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeSet,
    ffi::CString,
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
    path::{Path, PathBuf},
};
struct Error {
    failure: Failure,
    operational: bool,
}
type Result<T> = std::result::Result<T, Error>;
impl From<Failure> for Error {
    fn from(failure: Failure) -> Self {
        Self {
            failure,
            operational: false,
        }
    }
}
impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Failure::from(s).into()
    }
}
fn operational(failure: Failure) -> Error {
    Error {
        failure,
        operational: true,
    }
}
fn io(e: std::io::Error) -> Error {
    operational(e.to_string().into())
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .ok_or_else(|| "Invalid sync execution input".into())
}
fn tree(v: &Value) -> Result<&Map<String, Value>> {
    v.as_object()
        .ok_or_else(|| "Invalid sync execution tree".into())
}
fn under(p: &str, blocked: &BTreeSet<String>) -> bool {
    blocked
        .iter()
        .any(|b| p == b || p.strip_prefix(b).is_some_and(|s| s.starts_with('/')))
}
fn set(v: &Value) -> Result<BTreeSet<String>> {
    v.as_array()
        .ok_or("Invalid sync path set")?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "Invalid sync path".into())
        })
        .collect()
}
fn same(a: &Value, b: &Value) -> bool {
    if a.is_null() || b.is_null() {
        a == b
    } else {
        ["kind", "uid", "revision", "sha1", "size"]
            .iter()
            .all(|k| a[k] == b[k])
    }
}
fn local_same(a: &Value, b: &Value) -> bool {
    if a.is_null() || b.is_null() {
        return a == b;
    }
    [
        "kind",
        "sha1",
        "size",
        "uid",
        "revision",
        "remote_path",
        "mtime_ns",
        "inode",
        "device",
    ]
    .iter()
    .all(|k| {
        let empty = if matches!(*k, "size" | "mtime_ns" | "inode" | "device") {
            json!(0)
        } else {
            json!("")
        };
        a.get(k).unwrap_or(&empty) == b.get(k).unwrap_or(&empty)
    })
}
fn poll(host: &mut impl Services) -> i32 {
    if host.poll().is_ok() {
        0
    } else {
        1
    }
}
fn scan(fd: &File, host: &mut impl Services) -> Result<Value> {
    Ok(filesystem::scan(fd.as_raw_fd(), &mut |_| poll(host))?)
}
fn lookup(root: &Value, path: &str, host: &mut impl Services) -> Result<Value> {
    Ok(sync_remote::run(
        Remote::Lookup {
            root: root.clone(),
            relative: path.into(),
        },
        host,
    )?)
}
fn parent(root: &File, relative: &str) -> Result<(File, String)> {
    let parts: Vec<_> = relative.split('/').collect();
    if parts.iter().any(|p| !sync_remote::name_error(p).is_empty()) {
        return Err("Unsafe relative path".into());
    }
    let mut fd = filesystem::duplicate(root.as_raw_fd()).map_err(operational)?;
    for part in &parts[..parts.len() - 1] {
        fd = filesystem::open(fd.as_raw_fd(), part, libc::O_DIRECTORY).map_err(operational)?;
    }
    Ok((fd, parts[parts.len() - 1].into()))
}
struct Lock {
    path: PathBuf,
    directory: File,
    file: File,
}
impl Lock {
    fn acquire(state: &str) -> Result<Self> {
        let directory = storage::directory(Path::new(state), true, true)?;
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                c"sync.lock".as_ptr(),
                libc::O_WRONLY
                    | libc::O_CREAT
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io(std::io::Error::last_os_error()));
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let m = file.metadata().map_err(io)?;
        if !m.is_file()
            || m.uid() != unsafe { libc::getuid() }
            || m.nlink() != 1
            || m.mode() & 0o077 != 0
        {
            return Err("Unsafe sync process lock".into());
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Another synchronization is already running".into());
        }
        let lock = Self {
            path: state.into(),
            directory,
            file,
        };
        lock.validate()?;
        Ok(lock)
    }
    fn validate(&self) -> Result<()> {
        let current = storage::directory(&self.path, false, true)?;
        let old = self.directory.metadata().map_err(io)?;
        let new = current.metadata().map_err(io)?;
        if (old.dev(), old.ino()) != (new.dev(), new.ino()) {
            return Err("Sync state directory changed during execution".into());
        }
        let current = filesystem::open(
            self.directory.as_raw_fd(),
            "sync.lock",
            libc::O_WRONLY | libc::O_NONBLOCK,
        )?;
        let old = self.file.metadata().map_err(io)?;
        let new = current.metadata().map_err(io)?;
        if (old.dev(), old.ino()) != (new.dev(), new.ino()) || old.nlink() != 1 {
            return Err("Sync process lock changed during execution".into());
        }
        Ok(())
    }
}
struct Stage(PathBuf);
impl Stage {
    fn new(state: &str) -> Result<Self> {
        let path = Path::new(state).join(format!("stage-{}", trash_move::identifier(None)?));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(io)?;
        Ok(Self(path))
    }
}
use std::os::unix::fs::DirBuilderExt;
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Execution<'a> {
    state: &'a str,
    plan: &'a Value,
    proof: Plan,
    lock: Lock,
    root: File,
    trash_state: &'a str,
    trash_data: &'a str,
    trash_desktop: bool,
    result: Value,
    failed: BTreeSet<String>,
    baseline: Map<String, Value>,
    pending: Map<String, Value>,
    expected: Map<String, Value>,
}
impl Execution<'_> {
    fn push(&mut self, key: &str, value: Value) {
        self.result[key].as_array_mut().unwrap().push(value)
    }
    fn save(&self, hook: &mut impl FnMut(&str) -> std::result::Result<(), Failure>) -> Result<()> {
        self.lock.validate()?;
        hook("save").map_err(operational)?;
        sync_state::run(State::Save{state:self.state.into(),key:text(self.plan,"_state_key")?.into(),value:json!({"version":2,"local_root":self.proof.local_root,"remote_root_uid":self.proof.remote_root_uid,"remote_root":self.proof.remote_root,"local_root_identity":self.proof.root_identity,"entries":self.baseline,"pending_deletions":self.pending})}).map_err(operational)?;
        Ok(())
    }
    fn preflight(&self, relative: &str, kind: &str, host: &mut impl Services) -> Result<Value> {
        Ok(sync_proof::preflight(
            &self.proof,
            self.root.as_raw_fd(),
            relative,
            kind,
            host,
            &mut || {},
        )?)
    }
    fn transfer(
        &mut self,
        action: &Value,
        root: &Value,
        host: &mut impl Services,
        hook: &mut impl FnMut(&str) -> std::result::Result<(), Failure>,
    ) -> Result<()> {
        self.lock.validate()?;
        let relative = text(action, "relative_path")?;
        let kind = text(action, "kind")?;
        if matches!(kind, "trash_local" | "trash_remote") {
            let target = self.preflight(relative, kind, host)?;
            let id = trash_move::identifier(None)?;
            self.pending.insert(relative.into(),json!({"direction":if kind=="trash_local"{"local"}else{"remote"},"operation_id":id.replace('-',"")}));
            self.save(hook)?;
            if kind == "trash_remote" {
                let target = self.preflight(relative, kind, host)?;
                host.request("trash", json!({"paths":[target[relative]["remote_path"]]}))
                    .map_err(operational)?;
            } else {
                storage::directory(Path::new(self.trash_state), true, true)?;
                storage::directory(&Path::new(self.trash_state).join("entries"), true, true)?;
                let mut proof_error = None;
                let moved = trash_move::run(
                    trash_move::Request {
                        state: self.trash_state.into(),
                        data_home: self.trash_data.into(),
                        desktop: self.trash_desktop,
                        root_fd: self.root.as_raw_fd(),
                        root: self.proof.local_root.clone(),
                        relative: relative.into(),
                        expected: target[relative].clone(),
                        expected_tree: None,
                        identifier: Some(id),
                    },
                    &mut |event| {
                        if event["kind"] == "trash_validate" {
                            match self.preflight(relative, kind, host) {
                                Ok(_) => 0,
                                Err(e) => {
                                    proof_error = Some(e);
                                    1
                                }
                            }
                        } else if event["kind"] == "poll" {
                            poll(host)
                        } else {
                            0
                        }
                    },
                )
                .map_err(operational)?;
                if let Some(error) = proof_error {
                    return Err(error);
                }
                if moved["ok"] != true {
                    return Err(operational(Failure {
                        message: text(&moved, "error")?.into(),
                        cancelled: moved["cancelled"] == true,
                    }));
                }
                if moved["record"]["status"] != "trashed"
                    || moved["record"]["original_path"]
                        != json!(Path::new(&self.proof.local_root).join(relative))
                {
                    return Err("Local trash did not return a verified receipt".into());
                }
                hook("after_local_trash").map_err(operational)?;
            }
            let current = sync_proof::roots(&self.proof, self.root.as_raw_fd(), host)?;
            let local = scan(&self.root, host)?;
            if tree(&local["entries"])?.contains_key(relative)
                || tree(&local["blocked"])?
                    .keys()
                    .any(|p| relative == p || relative.starts_with(&format!("{p}/")))
            {
                return Err(
                    "Local path reappeared or became unreadable while deletion was propagated"
                        .into(),
                );
            }
            sync_proof::remote(&self.proof, &current, relative, host)?;
            self.push("trashed", json!(relative));
            return Ok(());
        }
        let expected_local = self.plan["_local"]
            .get(relative)
            .cloned()
            .unwrap_or(Value::Null);
        let (parent, name) = parent(&self.root, relative)?;
        let local = filesystem::entry(parent.as_raw_fd(), &name, &mut |_| poll(host))?;
        if !local_same(&local, &expected_local) {
            return Err("Local path changed after preview".into());
        }
        let before = self.expected.get(relative).cloned().unwrap_or(Value::Null);
        if !same(&before, &lookup(root, relative, host)?) {
            return Err("Remote path changed after preview".into());
        }
        let remote_parent = if let Some((parent, _)) = relative.rsplit_once('/') {
            let entry = lookup(root, parent, host)?;
            if entry["kind"] != "folder"
                || !same(self.expected.get(parent).unwrap_or(&Value::Null), &entry)
            {
                return Err("Remote parent changed after preview".into());
            }
            text(&entry, "remote_path")?.to_owned()
        } else {
            text(root, "path")?.to_owned()
        };
        match kind {
            "mkdir_local" => {
                let name = CString::new(name).map_err(|_| Error::from("Invalid folder name"))?;
                if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                    return Err(io(std::io::Error::last_os_error()));
                }
            }
            "mkdir_remote" => {
                let node = host
                    .request("mkdir", json!({"parent":remote_parent,"name":name}))
                    .map_err(operational)?;
                let entry = sync_remote::run(
                    Remote::NodeEntry {
                        node,
                        allow_trashed: false,
                    },
                    host,
                )?;
                if entry["kind"] != "folder" {
                    return Err("Remote mkdir did not return a folder".into());
                }
                self.expected.insert(relative.into(), entry);
            }
            "upload" => {
                if expected_local["kind"] != "file" {
                    return Err("Invalid upload action".into());
                }
                {
                    let stage = Stage::new(self.state)?;
                    let source = filesystem::stage_upload(
                        self.root.as_raw_fd(),
                        relative,
                        expected_local.clone(),
                        &stage.0,
                        &mut |_| poll(host),
                    )?;
                    if !same(&before, &lookup(root, relative, host)?) {
                        return Err("Remote destination changed while preparing upload".into());
                    }
                    host.request("upload",json!({"paths":[source],"parent":remote_parent,"file_strategy":if before.is_null(){"skip"}else{"create-new-revision"},"folder_strategy":"skip"})).map_err(operational)?;
                }
                let after = lookup(root, relative, host)?;
                if after["kind"] != "file"
                    || after["sha1"] != expected_local["sha1"]
                    || (!before.is_null() && after["uid"] != before["uid"])
                {
                    return Err("Upload could not be verified; remote content was retained".into());
                }
                self.expected.insert(relative.into(), after);
            }
            "download" => {
                if before["kind"] != "file" {
                    return Err("Invalid download action".into());
                }
                let stage = Stage::new(self.state)?;
                host.request("download",json!({"paths":[before["remote_path"]],"destination":stage.0,"file_strategy":"skip","folder_strategy":"skip"})).map_err(operational)?;
                let stage_fd = storage::directory(&stage.0, false, false)?;
                let downloaded =
                    filesystem::entry(stage_fd.as_raw_fd(), &name, &mut |_| poll(host))?;
                if downloaded["kind"] != "file" || downloaded["sha1"] != before["sha1"] {
                    return Err("Downloaded content is missing or failed SHA-1 verification".into());
                }
                if !same(&before, &lookup(root, relative, host)?) {
                    return Err("Remote source changed while downloading".into());
                }
                filesystem::install_download(
                    self.root.as_raw_fd(),
                    relative,
                    &stage.0.join(name),
                    expected_local,
                    Path::new(&self.proof.local_root),
                    &mut |event| {
                        if event["kind"] == "local_backup" {
                            self.result["backups"]
                                .as_array_mut()
                                .unwrap()
                                .push(event["path"].clone());
                            0
                        } else {
                            poll(host)
                        }
                    },
                )?;
            }
            _ => return Err(format!("Unknown sync action: {kind}").as_str().into()),
        }
        Ok(())
    }
    fn settle(
        &mut self,
        host: &mut impl Services,
        hook: &mut impl FnMut(&str) -> std::result::Result<(), Failure>,
    ) -> Result<()> {
        let root = sync_proof::roots(&self.proof, self.root.as_raw_fd(), host)?;
        let local = scan(&self.root, host)?;
        let remote = sync_remote::run(
            Remote::Scan {
                root: root.clone(),
                fresh: false,
                allow_trashed: false,
            },
            host,
        )?;
        self.result["_local_snapshot"] = local["entries"].clone();
        self.result["_remote_snapshot"] = remote["entries"].clone();
        let mut blocked = self.failed.clone();
        let mut problems = tree(&local["blocked"])?.clone();
        problems.extend(tree(&remote["blocked"])?.clone());
        for (path, reason) in &problems {
            blocked.insert(path.clone());
            let message = json!(format!(
                "{path}: {}",
                reason.as_str().unwrap_or("Unverified path")
            ));
            if !self.result["skipped"]
                .as_array()
                .unwrap()
                .contains(&message)
            {
                self.push("skipped", message)
            }
        }
        let convergence = sync_proof::evaluate(
            json!({"op":"convergence","local":local["entries"],"remote":remote["entries"],"actions":self.plan["actions"],"failed":self.failed,"blocked":blocked}),
        )?;
        self.result["conflicts"].as_array_mut().unwrap().extend(
            convergence["conflicts"]
                .as_array()
                .ok_or("Invalid convergence result")?
                .clone(),
        );
        blocked = set(&convergence["blocked"])?;
        for (relative, operation) in self.pending.clone() {
            if under(&relative, &blocked)
                || tree(&local["entries"])?.contains_key(&relative)
                || tree(&remote["entries"])?.contains_key(&relative)
            {
                continue;
            }
            let confirm = (|| -> Result<()> {
                sync_proof::remote(&self.proof, &root, &relative, host)?;
                if operation["direction"] == "local" {
                    let id = text(&operation, "operation_id")?;
                    let id = if id.len() == 32 && id.bytes().all(|c| c.is_ascii_hexdigit()) {
                        format!(
                            "{}-{}-{}-{}-{}",
                            &id[..8],
                            &id[8..12],
                            &id[12..16],
                            &id[16..20],
                            &id[20..]
                        )
                    } else {
                        return Err("Invalid pending operation identity".into());
                    };
                    let receipts = trash_restore::list(self.trash_state, &mut |_| poll(host))?;
                    let receipt = receipts
                        .as_array()
                        .ok_or("Invalid trash listing")?
                        .iter()
                        .find(|r| r["id"] == id)
                        .ok_or("Uncertain local trash receipt requires recovery review")?;
                    if receipt["status"] != "trashed"
                        || receipt["original_path"]
                            != json!(Path::new(&self.proof.local_root).join(&relative))
                    {
                        return Err("Uncertain local trash receipt requires recovery review".into());
                    }
                }
                Ok(())
            })();
            match confirm {
                Ok(()) => {
                    self.pending.remove(&relative);
                    self.baseline
                        .retain(|p, _| p != &relative && !p.starts_with(&format!("{relative}/")));
                    if !self.result["trashed"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(relative))
                    {
                        self.push("trashed", json!(relative))
                    }
                }
                Err(e) if e.failure.cancelled => return Err(e),
                Err(e) => {
                    blocked.insert(relative.clone());
                    self.push(
                        "conflicts",
                        json!(format!("{relative}: {}", e.failure.message)),
                    )
                }
            }
        }
        let mut observation = json!({"op":"baseline","local":local["entries"],"remote":remote["entries"],"baseline":self.baseline,"blocked":blocked,"pending":self.pending.keys().collect::<Vec<_>>(),"absent":[]});
        let mut update = sync_proof::evaluate(observation.clone())?;
        for relative in update["checks"]
            .as_array()
            .ok_or("Invalid baseline checks")?
        {
            let relative = relative.as_str().ok_or("Invalid baseline check path")?;
            if lookup(&root, relative, host)?.is_null() {
                observation["absent"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(relative));
            }
        }
        if !update["checks"].as_array().unwrap().is_empty() {
            update = sync_proof::evaluate(observation)?
        }
        self.baseline = tree(&update["baseline"])?.clone();
        self.save(hook)
    }
}
pub struct Request {
    pub state: String,
    pub plan: Value,
    pub trash_state: String,
    pub trash_data: String,
    pub trash_desktop: bool,
}
pub fn run(request: Request, host: &mut impl Services) -> std::result::Result<Value, Failure> {
    run_with_hook(request, host, &mut |_| Ok(())).map_err(|e| e.failure)
}
fn run_with_hook(
    request: Request,
    host: &mut impl Services,
    hook: &mut impl FnMut(&str) -> std::result::Result<(), Failure>,
) -> Result<Value> {
    host.poll()?;
    let lock = Lock::acquire(&request.state)?;
    host.execution_started()?;
    let plan = &request.plan;
    let proof:Plan=serde_json::from_value(json!({"local_root":plan["local_root"],"remote_root":plan["remote_root"],"remote_root_uid":plan["remote_root_uid"],"root_identity":plan["_root_identity"],"baseline":plan["_baseline"]})).map_err(|_|Error::from("Invalid execution plan"))?;
    if Path::new(&request.state).starts_with(Path::new(&proof.local_root)) {
        return Err(
            "The private sync state directory must be outside the synchronized tree".into(),
        );
    }
    if !plan["skipped"]
        .as_array()
        .is_some_and(|v| v.iter().all(Value::is_string))
    {
        return Err("Invalid skipped paths".into());
    }
    let root = sync_remote::run(
        Remote::Root {
            path: proof.remote_root.clone(),
        },
        host,
    )?;
    if root["uid"] != proof.remote_root_uid {
        return Err("Remote root/account changed; create a new preview".into());
    }
    let current = sync_state::run(State::Load {
        state: request.state.clone(),
        key: text(plan, "_state_key")?.into(),
        root: proof.local_root.clone(),
        uid: proof.remote_root_uid.clone(),
    })?;
    if current["digest"] != plan["_state_digest"] {
        return Err("Another sync changed the baseline; create a new preview".into());
    }
    let fd = storage::directory(Path::new(&proof.local_root), false, false)?;
    let m = fd.metadata().map_err(io)?;
    if [m.dev(), m.ino()] != proof.root_identity {
        return Err("Local sync root was replaced after preview".into());
    }
    let conflicts: Vec<_> = plan["conflicts"]
        .as_array()
        .ok_or("Invalid plan conflicts")?
        .iter()
        .map(|v| {
            json!(format!(
                "{}: {}",
                v["relative_path"].as_str().unwrap_or(""),
                v["reason"].as_str().unwrap_or("")
            ))
        })
        .collect();
    let result = json!({"completed":[],"conflicts":conflicts,"errors":[],"skipped":plan["skipped"],"backups":[],"trashed":[],"_local_snapshot":{},"_remote_snapshot":{}});
    let mut execution = Execution {
        state: &request.state,
        plan,
        proof,
        lock,
        root: fd,
        trash_state: &request.trash_state,
        trash_data: &request.trash_data,
        trash_desktop: request.trash_desktop,
        result,
        failed: set(&plan["_blocked"])?,
        baseline: tree(&plan["_baseline"])?.clone(),
        pending: tree(&plan["_pending_deletions"])?.clone(),
        expected: tree(&plan["_remote"])?.clone(),
    };
    let actions = plan["actions"]
        .as_array()
        .ok_or("Invalid execution actions")?;
    for (index, action) in actions.iter().enumerate() {
        host.progress(json!({"kind":"progress","unit":"items","completed":index,"total":actions.len(),"path":action["relative_path"],"operation":action["kind"]}));
        host.poll()?;
        let relative = text(action, "relative_path")?;
        if under(relative, &execution.failed) {
            execution.push(
                "conflicts",
                json!(format!("{relative}: parent operation was blocked")),
            );
            continue;
        }
        match execution.transfer(action, &root, host, hook) {
            Ok(()) => {
                let detail = action["detail"].as_str().unwrap_or("");
                execution.push(
                    "completed",
                    json!(format!(
                        "{}: {relative}{}",
                        text(action, "kind")?,
                        if detail.is_empty() {
                            String::new()
                        } else {
                            format!(" — {detail}")
                        }
                    )),
                );
            }
            Err(e) if e.failure.cancelled => return Err(e),
            Err(e) => {
                execution.failed.insert(relative.into());
                execution.push(
                    if e.operational { "errors" } else { "conflicts" },
                    json!(format!("{relative}: {}", e.failure.message)),
                );
            }
        }
    }
    host.progress(json!({"kind":"progress","unit":"items","completed":actions.len(),"total":actions.len(),"path":"","operation":"finalizing"}));
    match execution.settle(host, hook) {
        Ok(()) => {}
        Err(e) if e.failure.cancelled => return Err(e),
        Err(e) => execution.push(
            "errors",
            json!(format!("Baseline was not updated: {}", e.failure.message)),
        ),
    }
    Ok(execution.result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Host {
        file: Value,
        trashed: bool,
        trash_calls: usize,
    }
    impl Services for Host {
        fn poll(&mut self) -> std::result::Result<(), Failure> {
            Ok(())
        }
        fn request(&mut self, method: &str, args: Value) -> std::result::Result<Value, Failure> {
            if method == "trash" {
                self.trashed = true;
                self.trash_calls += 1;
                return Ok(Value::Null);
            }
            if method.starts_with("list_nodes") {
                return Ok(if self.trashed {
                    json!([])
                } else {
                    json!([self.file])
                });
            }
            let node = if args["path"] == "/my-files" {
                json!({"uid":"root","type":"folder","path":"/my-files","name":"root"})
            } else {
                let mut v = self.file.clone();
                v["raw"]["isTrashed"] = json!(self.trashed);
                v
            };
            Ok(if method == "try_info" {
                json!({"node":node})
            } else {
                node
            })
        }
    }
    struct Tree {
        base: PathBuf,
        root: PathBuf,
        state: PathBuf,
        host: Host,
    }
    impl Tree {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!(
                "nd-execution-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let root = base.join("local");
            let state = base.join("state");
            let fd = storage::directory(&root, true, true).unwrap();
            std::fs::write(root.join("data"), b"valuable").unwrap();
            let entry = filesystem::entry(fd.as_raw_fd(), "data", &mut |_| 0).unwrap();
            let host = Host {
                file: json!({"uid":"file","name":"data","type":"file","path":"/my-files/file","sha1":entry["sha1"],"size":entry["size"],"revision":"rev1","raw":{"parentUid":"root","isTrashed":false}}),
                trashed: false,
                trash_calls: 0,
            };
            let mut tree = Self {
                base,
                root,
                state,
                host,
            };
            let request = tree.request();
            assert!(run(request, &mut tree.host).unwrap()["errors"]
                .as_array()
                .unwrap()
                .is_empty());
            tree
        }
        fn request(&mut self) -> Request {
            let state = self.state.to_str().unwrap();
            let root = self.root.to_str().unwrap();
            let plan = crate::sync_plan::run(state, root, "/my-files", &mut self.host).unwrap();
            Request {
                state: state.into(),
                plan,
                trash_state: self.base.join("journal").to_str().unwrap().into(),
                trash_data: self.base.join("xdg").to_str().unwrap().into(),
                trash_desktop: true,
            }
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }
    #[test]
    fn failed_intent_persistence_prevents_remote_mutation() {
        let mut tree = Tree::new();
        std::fs::remove_file(tree.root.join("data")).unwrap();
        let request = tree.request();
        let result = run_with_hook(request, &mut tree.host, &mut |phase| {
            if phase == "save" {
                Err("synthetic full disk".into())
            } else {
                Ok(())
            }
        })
        .map_err(|e| e.failure)
        .unwrap();
        assert!(!result["errors"].as_array().unwrap().is_empty());
        assert_eq!(tree.host.trash_calls, 0);
        assert!(!tree.host.trashed);
    }
    #[test]
    fn lost_local_ack_reconciles_only_matching_durable_receipt() {
        let mut tree = Tree::new();
        tree.host.trashed = true;
        let request = tree.request();
        let result = run_with_hook(request, &mut tree.host, &mut |phase| {
            if phase == "after_local_trash" {
                Err("synthetic lost acknowledgement".into())
            } else {
                Ok(())
            }
        })
        .map_err(|e| e.failure)
        .unwrap();
        assert!(!result["errors"].as_array().unwrap().is_empty());
        assert!(!tree.root.join("data").exists());
        let request = tree.request();
        assert!(!request.plan["_pending_deletions"]
            .as_object()
            .unwrap()
            .is_empty());
        let result = run(request, &mut tree.host).unwrap();
        assert_eq!(result["trashed"], json!(["data"]));
        assert_eq!(result["errors"], json!([]));
        assert_eq!(result["conflicts"], json!([]));
        let next = tree.request();
        assert_eq!(next.plan["_pending_deletions"], json!({}));
        assert_eq!(
            trash_restore::list(tree.base.join("journal").to_str().unwrap(), &mut |_| 0)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn process_lock_rejects_replacement_and_releases_on_drop() {
        let tree = Tree::new();
        let state = tree.state.to_str().unwrap();
        let lock = Lock::acquire(state).map_err(|e| e.failure).unwrap();
        assert!(Lock::acquire(state).is_err());
        std::fs::rename(tree.state.join("sync.lock"), tree.state.join("old-lock")).unwrap();
        std::fs::write(tree.state.join("sync.lock"), b"").unwrap();
        assert!(lock.validate().is_err());
        drop(lock);
        std::fs::remove_file(tree.state.join("sync.lock")).unwrap();
        assert!(Lock::acquire(state).is_ok());
    }
}
