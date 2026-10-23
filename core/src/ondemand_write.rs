//! Revision-checked on-demand writeback with durable local recovery.
use crate::{
    host::Services,
    ondemand_io::{self as io, fail, field, revision, Error, Result},
    process::Failure,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs::File,
    os::{
        fd::{FromRawFd, RawFd},
        unix::fs::{FileExt, MetadataExt},
    },
    path::PathBuf,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    fd: RawFd,
    work: PathBuf,
    journal: PathBuf,
    objects: PathBuf,
    staging: PathBuf,
    node: Value,
    parent: Value,
    baseline: Value,
    new_file: bool,
    intent: Value,
    error: String,
    remote_root: String,
    root_uid: String,
}
fn record(request: &Request, state: &str, after: Option<&Value>, error: &str) -> Result<()> {
    io::write_json(
        &request.journal,
        json!({"version":1,"state":state,"data":request.work.file_name().and_then(|n|n.to_str()).ok_or_else(||fail(libc::EINVAL,"Invalid recovery filename"))?,"name":request.node["name"],"parent":request.parent["path"],"baseline":after.unwrap_or(&request.baseline).as_object().ok_or_else(||fail(libc::EINVAL,"Invalid recovery baseline"))?.iter().filter(|(key,_)|key.as_str()!="raw").map(|(key,value)|(key.clone(),value.clone())).collect::<serde_json::Map<_,_>>(),"new_file":request.new_file&&after.is_none(),"error":error,"intent":request.intent}),
    )
}
fn copy(source: &File, target: &File) -> Result<()> {
    let mut offset = 0;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = source.read_at(&mut buffer, offset)?;
        if count == 0 {
            break;
        }
        target.write_all_at(&buffer[..count], offset)?;
        offset += count as u64;
    }
    target.sync_all()?;
    Ok(())
}
fn matches(host: &mut impl Services, parent: &Value, name: &str) -> Result<Vec<Value>> {
    let nodes = host.request("list_nodes", json!({"path":parent["path"]}))?;
    Ok(nodes
        .as_array()
        .ok_or_else(|| fail(libc::EIO, "Invalid cloud directory response"))?
        .iter()
        .filter(|n| n["name"] == name)
        .cloned()
        .collect())
}
fn safe_node(node: &Value) -> Result<()> {
    if node["safe"] != true || node["type"] != "file" || field(node, "uid").is_empty() {
        return Err(fail(libc::EIO, "Unsafe or unsupported remote node"));
    }
    crate::backend::name(field(node, "name"))?;
    crate::backend::remote(field(node, "path"))?;
    let sha = field(node, "sha1");
    if field(node, "revision").is_empty()
        || node["size"].as_u64().is_none()
        || sha.len() != 40
        || !sha.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(fail(
            libc::ENOTSUP,
            "File has no verifiable size, SHA1 or revision (native documents are unsupported)",
        ));
    }
    Ok(())
}
fn execute(request: &Request, after: &mut Option<Value>, host: &mut impl Services) -> Result<()> {
    let fd = unsafe { libc::fcntl(request.fd, libc::F_DUPFD_CLOEXEC, 3) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let source = unsafe { File::from_raw_fd(fd) };
    let named = io::regular(&request.work, libc::O_RDONLY)?.metadata()?;
    let opened = source.metadata()?;
    if (named.dev(), named.ino()) != (opened.dev(), opened.ino()) {
        return Err(fail(
            libc::EBUSY,
            "Recovery data path was replaced before upload",
        ));
    }
    source.sync_all()?;
    let root = host.request("info", json!({"path":request.remote_root}))?;
    if root["uid"] != request.root_uid || root["type"] != "folder" {
        return Err(fail(libc::EACCES,"Cloud account or mount root changed. Unmount before reconnecting; unsaved content remains in recovery."));
    }
    let (digest, size, _) = io::digest(&source)?;
    let name = field(&request.node, "name");
    crate::backend::name(name)?;
    if name
        .chars()
        .any(|c| matches!(c, '*' | '?' | '[' | '{' | '\u{7f}'))
        || name.len() > 255
    {
        return Err(fail(libc::EINVAL, "Invalid cache upload name"));
    }
    let candidates = matches(host, &request.parent, name)?;
    let strategy = if request.new_file {
        if !candidates.is_empty() {
            return Err(fail(
                libc::EEXIST,
                "A remote file now uses this name; local data is in recovery",
            ));
        }
        "rename"
    } else {
        if candidates.len() != 1 || revision(&candidates[0]) != revision(&request.baseline) {
            return Err(fail(
                libc::ESTALE,
                "Remote revision changed; local edit retained in recovery",
            ));
        }
        let current = host.request("info", json!({"path":request.baseline["path"]}))?;
        if revision(&current) != revision(&request.baseline) {
            return Err(fail(
                libc::ESTALE,
                "Remote revision changed before upload; local edit retained",
            ));
        }
        "create-new-revision"
    };
    record(request, "uploading", None, &request.error)?;
    {
        let stage = io::Stage::new(&request.staging)?;
        let path = stage.path.join(name);
        let file = stage.create_file(name)?;
        copy(&source, &file)?;
        let (sha, bytes, _) = io::digest(&file)?;
        if sha != digest || bytes != size {
            return Err(fail(libc::EBUSY, "Local edit changed while staging upload"));
        }
        host.request(
            "upload",
            json!({"paths":[path],"parent":request.parent["path"],"file_strategy":strategy}),
        )?;
    }
    let candidates = matches(host, &request.parent, name)?;
    if candidates.len() != 1 {
        return Err(fail(
            libc::EIO,
            "Cannot identify the uploaded revision; local recovery retained",
        ));
    }
    let current = host.request("info", json!({"path":candidates[0]["path"]}))?;
    safe_node(&current)?;
    if field(&current, "sha1").to_lowercase() != digest
        || current["size"].as_u64() != Some(size)
        || (!request.new_file && current["uid"] != request.baseline["uid"])
    {
        return Err(fail(
            libc::ESTALE,
            "Remote content changed after upload; local recovery retained",
        ));
    }
    *after = Some(current.clone());
    // A separate file ensures later edits never modify an already verified object.
    let stage = io::Stage::new(&request.objects)?;
    let file = stage.create_file("verified")?;
    copy(&source, &file)?;
    let (sha, bytes, _) = io::digest(&file)?;
    if sha != digest || bytes != size {
        return Err(fail(
            libc::EBUSY,
            "Local edit changed after upload; recovery retained",
        ));
    }
    stage.install("verified", &request.objects, &io::key(&current)?)?;
    record(request, "clean", after.as_ref(), &request.error)?;
    Ok(())
}
pub fn run(request: Request, host: &mut impl Services) -> std::result::Result<Value, Failure> {
    let mut after = None;
    let result = execute(&request, &mut after, host);
    match result {
        Ok(()) => Ok(json!({"after":after})),
        Err(error) => {
            let message="Writeback failed or conflicted; local content retained. Inspect recovery before retrying.";
            let error = match record(&request, "recovery", after.as_ref(), message) {
                Ok(()) => error,
                Err(error) => error,
            };
            Ok(match error {
                Error::Fs(errno, message) => json!({"after":after,"errno":errno,"message":message}),
                Error::Backend(error) => {
                    json!({"after":after,"backend_error":error.message,"cancelled":error.cancelled})
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};
    use std::{
        os::fd::AsRawFd,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "nd-writeback-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
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
    fn node(updated: bool) -> Value {
        json!({"uid":"file","name":"data","type":"file","safe":true,"path":"/my-files/file","revision":if updated{"r2"}else{"r1"},"sha1":if updated{"a9993e364706816aba3e25717850c26c9cd0d89d"}else{"0000000000000000000000000000000000000000"},"size":3,"raw":{}})
    }
    struct Host {
        uploaded: bool,
        lost_ack: bool,
        break_cache: bool,
        objects: PathBuf,
    }
    impl Services for Host {
        fn poll(&mut self) -> std::result::Result<(), Failure> {
            Ok(())
        }
        fn request(&mut self, method: &str, args: Value) -> std::result::Result<Value, Failure> {
            match method {
                "info" if args["path"] == "/my-files" => Ok(json!({"uid":"root","type":"folder"})),
                "info" => {
                    if self.uploaded && self.break_cache {
                        std::fs::rename(&self.objects, self.objects.with_extension("old")).unwrap();
                        std::fs::write(&self.objects, b"obstruction").unwrap();
                    }
                    Ok(node(self.uploaded))
                }
                "list_nodes" => Ok(json!([node(self.uploaded)])),
                "upload" => {
                    assert_eq!(
                        std::fs::read(args["paths"][0].as_str().unwrap()).unwrap(),
                        b"abc"
                    );
                    self.uploaded = true;
                    if self.lost_ack {
                        Err("lost acknowledgement".into())
                    } else {
                        Ok(Value::Null)
                    }
                }
                _ => panic!("unexpected SDK method"),
            }
        }
    }
    fn request(temp: &Temp, source: &File) -> Request {
        Request {
            fd: source.as_raw_fd(),
            work: temp.0.join("edit.data"),
            journal: temp.0.join("edit.json"),
            objects: temp.0.join("objects"),
            staging: temp.0.join("staging"),
            node: node(false),
            parent: json!({"path":"/my-files/root"}),
            baseline: node(false),
            new_file: false,
            intent: Value::Null,
            error: String::new(),
            remote_root: "/my-files".into(),
            root_uid: "root".into(),
        }
    }
    #[test]
    fn writeback_keeps_recovery_for_lost_ack_and_post_commit_cache_failure() {
        for (lost_ack, break_cache) in [(false, false), (true, false), (false, true)] {
            let temp = Temp::new();
            let source = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(temp.0.join("edit.data"))
                .unwrap();
            source.write_all_at(b"abc", 0).unwrap();
            io::private_directory(&temp.0.join("objects")).unwrap();
            io::private_directory(&temp.0.join("staging")).unwrap();
            let result = run(
                request(&temp, &source),
                &mut Host {
                    uploaded: false,
                    lost_ack,
                    break_cache,
                    objects: temp.0.join("objects"),
                },
            )
            .unwrap();
            let journal = io::read_json(&temp.0.join("edit.json")).unwrap();
            assert!(source.metadata().is_ok());
            assert_eq!(std::fs::read(temp.0.join("edit.data")).unwrap(), b"abc");
            if lost_ack {
                assert!(result["backend_error"].is_string());
                assert_eq!(journal["state"], "recovery");
                assert_eq!(journal["baseline"]["revision"], "r1");
            } else if break_cache {
                assert!(result["errno"].is_number());
                assert_eq!(result["after"]["revision"], "r2");
                assert_eq!(journal["baseline"]["revision"], "r2");
                assert_eq!(journal["state"], "recovery");
            } else {
                assert_eq!(journal["state"], "clean");
                let target = temp.0.join("objects").join(io::key(&node(true)).unwrap());
                source.write_all_at(b"xyz", 0).unwrap();
                assert_eq!(std::fs::read(target).unwrap(), b"abc");
            }
            assert_eq!(
                std::fs::read_dir(temp.0.join("staging")).unwrap().count(),
                0
            );
        }
    }
}
