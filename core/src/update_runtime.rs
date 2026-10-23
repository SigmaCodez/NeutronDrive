//! Bounded HTTPS updates with owned private staging and stable descriptor verification.
use crate::{process::Failure, storage, update_policy};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{File, Metadata},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{FileExt, MetadataExt},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const REPOSITORY: &str = "https://github.com/SigmaCodez/NeutronDrive";
const API: &str = "https://api.github.com/repos/SigmaCodez/NeutronDrive/releases?per_page=100";
const MAX_PACKAGE: u64 = 512 * 1024 * 1024;
const MAX_RELEASES: usize = 4 * 1024 * 1024;
#[derive(Debug)]
enum Error {
    Invalid(&'static str),
    Io,
    Timeout,
    Cancelled,
}
type Result<T> = std::result::Result<T, Error>;
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::TimedOut {
            Self::Timeout
        } else {
            Self::Io
        }
    }
}
impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        if matches!(e, ureq::Error::Timeout(_)) {
            Self::Timeout
        } else {
            Self::Io
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    tag: String,
    url: String,
    asset_url: String,
    digest: String,
    size: u64,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    InstallFormat {
        payload: PathBuf,
    },
    Check {
        current: String,
        preview: bool,
        format: String,
    },
    Download {
        release: Release,
        destination: PathBuf,
    },
    Verify {
        release: Release,
        path: PathBuf,
    },
}
#[derive(Clone, Copy)]
enum Mode {
    Check,
    Download,
}
trait Transport {
    fn open(&mut self, url: &str, mode: Mode) -> Result<Box<dyn Read>>;
}
struct Https;
impl Transport for Https {
    fn open(&mut self, url: &str, mode: Mode) -> Result<Box<dyn Read>> {
        request(agent_config(mode).build().new_agent(), url, mode)
    }
}
fn agent_config(mode: Mode) -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(10)
        .timeout_global(Some(Duration::from_secs(match mode {
            Mode::Check => 15,
            Mode::Download => 600,
        })))
        .timeout_resolve(Some(Duration::from_secs(15)))
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(20)))
        .timeout_per_read(Some(Duration::from_secs(20)))
        .timeout_per_write(Some(Duration::from_secs(20)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
}
fn request(agent: ureq::Agent, url: &str, mode: Mode) -> Result<Box<dyn Read>> {
    let mut request = agent.get(url).header(
        "User-Agent",
        match mode {
            Mode::Check => "NeutronDrive-update-check",
            Mode::Download => "NeutronDrive-update-download",
        },
    );
    if matches!(mode, Mode::Check) {
        request = request.header("Accept", "application/vnd.github+json");
    }
    let response = request.call()?;
    Ok(Box::new(response.into_body().into_reader()))
}
fn poll(callback: &mut impl FnMut(Value) -> i32) -> Result<()> {
    if callback(json!({"kind":"poll"})) != 0 {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
fn release_valid(release: &Release) -> Result<()> {
    if release.digest.len() != 64
        || !release
            .digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || release.size == 0
        || release.size > MAX_PACKAGE
    {
        return Err(Error::Invalid("Missing verified asset"));
    }
    if !release
        .asset_url
        .starts_with(&format!("{REPOSITORY}/releases/download/"))
        || release.asset_url.parse::<ureq::http::Uri>().is_err()
    {
        return Err(Error::Invalid("Invalid download source"));
    }
    // These fields remain available to the UI, but the digest and exact selected
    // asset URL are the transport authority. Release selection validates tags.
    let _ = (&release.tag, &release.url);
    Ok(())
}
fn installation(payload: &Path) -> Value {
    let set = |name| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    json!(if set("FLATPAK_ID") {
        "flatpak"
    } else if set("SNAP") {
        "snap"
    } else if set("APPIMAGE") || set("APPDIR") {
        "appimage"
    } else if payload.canonicalize().ok().as_deref()
        == Some(Path::new("/usr/lib/neutrondrive/neutrondrive"))
        && Path::new("/var/lib/dpkg/info/neutrondrive.list").is_file()
    {
        "deb"
    } else {
        "other"
    })
}
fn check(
    current: &str,
    preview: bool,
    format: &str,
    transport: &mut impl Transport,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    poll(callback)?;
    let mut response = transport.open(API, Mode::Check)?;
    let mut bytes = Vec::new();
    let mut chunk = [0; 32768];
    loop {
        poll(callback)?;
        let count = response.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > MAX_RELEASES {
            return Err(Error::Invalid("Release response too large"));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let releases =
        serde_json::from_slice(&bytes).map_err(|_| Error::Invalid("Invalid release response"))?;
    update_policy::choose(releases, current, preview, format, std::env::consts::ARCH)
        .map_err(|_| Error::Invalid("Invalid release response"))
}
fn same(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino()
}
fn stable(a: &Metadata, b: &Metadata) -> bool {
    same(a, b)
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
        && b.nlink() == 1
}
fn regular(meta: &Metadata, size: u64) -> Result<()> {
    if !meta.is_file()
        || meta.uid() != unsafe { libc::getuid() }
        || meta.nlink() != 1
        || meta.len() != size
    {
        return Err(Error::Invalid("Invalid downloaded package"));
    }
    Ok(())
}
fn digest(file: &File, release: &Release, callback: &mut impl FnMut(Value) -> i32) -> Result<()> {
    let before = file.metadata()?;
    regular(&before, release.size)?;
    let mut sha = Sha256::new();
    let mut offset = 0;
    let mut chunk = vec![0; 256 * 1024];
    loop {
        poll(callback)?;
        let count = file.read_at(&mut chunk, offset)?;
        if count == 0 {
            break;
        }
        offset += count as u64;
        if offset > release.size {
            return Err(Error::Invalid("Downloaded package changed"));
        }
        sha.update(&chunk[..count]);
    }
    let after = file.metadata()?;
    if offset != release.size
        || !stable(&before, &after)
        || format!("{:x}", sha.finalize()) != release.digest
    {
        return Err(Error::Invalid("Package checksum mismatch"));
    }
    Ok(())
}
fn directory(path: &Path, create: bool, private: bool) -> Result<File> {
    storage::directory(path, create, private)
        .map_err(|_| Error::Invalid("Update directory is not safely accessible"))
}
fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}
fn open_at(parent: &File, name: &std::ffi::CStr, flags: i32) -> Result<File> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(Error::Io);
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
struct Staging {
    parent: File,
    directory: File,
    file: File,
    name: CString,
    path: PathBuf,
    keep: bool,
}
impl Staging {
    fn new(destination: &Path) -> Result<Self> {
        let destination = absolute(destination)?;
        let parent = directory(&destination, true, false)?;
        let mut bytes = [0; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        let name = format!(
            "release-{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let path = destination.join(&name);
        let name = CString::new(name).unwrap();
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } < 0 {
            return Err(Error::Io);
        }
        // If the name was replaced, do not remove an entry we cannot identify.
        let directory = open_at(&parent, &name, libc::O_RDONLY | libc::O_DIRECTORY)?;
        let meta = directory.metadata()?;
        if meta.uid() != unsafe { libc::getuid() } || meta.mode() & 0o077 != 0 {
            return Err(Error::Invalid("Update directory is not private"));
        }
        let file = match open_at(
            &directory,
            c"neutrondrive.deb",
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
        ) {
            Ok(file) => file,
            Err(e) => {
                if let Ok(current) = open_at(&parent, &name, libc::O_RDONLY | libc::O_DIRECTORY) {
                    if current.metadata().is_ok_and(|m| same(&m, &meta)) {
                        unsafe {
                            libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR);
                        }
                    }
                }
                return Err(e);
            }
        };
        Ok(Self {
            parent,
            directory,
            file,
            name,
            path,
            keep: false,
        })
    }
    fn validate_path(&self) -> Result<()> {
        let current = directory(&self.path, false, true)?;
        if !same(&current.metadata()?, &self.directory.metadata()?) {
            return Err(Error::Invalid("Update directory changed"));
        }
        let file = open_at(&current, c"neutrondrive.deb", libc::O_RDONLY)?;
        if !same(&file.metadata()?, &self.file.metadata()?) {
            return Err(Error::Invalid("Downloaded package changed"));
        }
        Ok(())
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        if let (Ok(current), Ok(original)) = (
            open_at(&self.directory, c"neutrondrive.deb", libc::O_RDONLY),
            self.file.metadata(),
        ) {
            if current.metadata().is_ok_and(|m| same(&m, &original)) {
                unsafe {
                    libc::unlinkat(self.directory.as_raw_fd(), c"neutrondrive.deb".as_ptr(), 0);
                }
            }
        }
        if let (Ok(current), Ok(original)) = (
            open_at(&self.parent, &self.name, libc::O_RDONLY | libc::O_DIRECTORY),
            self.directory.metadata(),
        ) {
            if current.metadata().is_ok_and(|m| same(&m, &original)) {
                unsafe {
                    libc::unlinkat(
                        self.parent.as_raw_fd(),
                        self.name.as_ptr(),
                        libc::AT_REMOVEDIR,
                    );
                }
            }
        }
    }
}
fn download(
    release: &Release,
    destination: &Path,
    transport: &mut impl Transport,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    release_valid(release)?;
    poll(callback)?;
    let mut staging = Staging::new(destination)?;
    let mut response = transport.open(&release.asset_url, Mode::Download)?;
    let deadline = Instant::now() + Duration::from_secs(600);
    let mut sha = Sha256::new();
    let mut count = 0u64;
    let mut chunk = vec![0; 256 * 1024];
    loop {
        poll(callback)?;
        if Instant::now() >= deadline {
            return Err(Error::Timeout);
        }
        let read = response.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > release.size || count > MAX_PACKAGE {
            return Err(Error::Invalid("Unexpected package size"));
        }
        sha.update(&chunk[..read]);
        staging.file.write_all(&chunk[..read])?;
    }
    if count != release.size || format!("{:x}", sha.finalize()) != release.digest {
        return Err(Error::Invalid("Package checksum mismatch"));
    }
    staging.file.sync_all()?;
    digest(&staging.file, release, callback)?;
    staging.directory.sync_all()?;
    staging.parent.sync_all()?;
    staging.validate_path()?;
    let path = staging.path.join("neutrondrive.deb");
    staging.keep = true;
    Ok(json!(path))
}
fn verify(
    path: &Path,
    release: &Release,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    release_valid(release)?;
    poll(callback)?;
    let path = absolute(path)?;
    let parent = path
        .parent()
        .ok_or(Error::Invalid("Invalid package path"))?;
    let parent_fd = directory(parent, false, true)?;
    use std::os::unix::ffi::OsStrExt;
    let name = CString::new(
        path.file_name()
            .ok_or(Error::Invalid("Invalid package path"))?
            .as_bytes(),
    )
    .map_err(|_| Error::Invalid("Invalid package path"))?;
    let file = open_at(&parent_fd, &name, libc::O_RDONLY)?;
    digest(&file, release, callback)?;
    let current_parent = directory(parent, false, true)?;
    if !same(&parent_fd.metadata()?, &current_parent.metadata()?) {
        return Err(Error::Invalid("Update directory changed"));
    }
    let current = open_at(&current_parent, &name, libc::O_RDONLY)?;
    if !same(&file.metadata()?, &current.metadata()?) {
        return Err(Error::Invalid("Downloaded package changed"));
    }
    Ok(Value::Null)
}
pub fn run(
    operation: Operation,
    callback: &mut impl FnMut(Value) -> i32,
) -> std::result::Result<Value, Failure> {
    let result = match operation {
        Operation::InstallFormat { payload } => Ok(installation(&payload)),
        Operation::Check {
            current,
            preview,
            format,
        } => check(&current, preview, &format, &mut Https, callback),
        Operation::Download {
            release,
            destination,
        } => download(&release, &destination, &mut Https, callback),
        Operation::Verify { release, path } => verify(&path, &release, callback),
    };
    Ok(match result {
        Ok(v) => json!({"value":v}),
        Err(e) => {
            let (kind, message) = match e {
                Error::Invalid(m) => ("validation", m),
                Error::Io => ("io", "Update network or file operation failed"),
                Error::Timeout => ("timeout", "Update request timed out"),
                Error::Cancelled => ("cancelled", "Download cancelled"),
            };
            json!({"error_kind":kind,"message":message})
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Cursor,
        os::unix::fs::PermissionsExt,
        sync::atomic::{AtomicU64, Ordering},
    };
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "nd-update-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    struct Fixture(Option<Vec<u8>>);
    impl Transport for Fixture {
        fn open(&mut self, _: &str, _: Mode) -> Result<Box<dyn Read>> {
            self.0
                .take()
                .map(|v| Box::new(Cursor::new(v)) as Box<dyn Read>)
                .ok_or(Error::Io)
        }
    }
    fn release() -> Release {
        Release {
            tag: "v1.0.0".into(),
            url: format!("{REPOSITORY}/releases/tag/v1.0.0"),
            asset_url: format!("{REPOSITORY}/releases/download/v1.0.0/package.deb"),
            digest: format!("{:x}", Sha256::digest(b"package")),
            size: 7,
        }
    }
    #[test]
    fn download_is_private_and_reverification_detects_modification() {
        let tmp = Temp::new();
        let r = release();
        let value = download(
            &r,
            &tmp.0,
            &mut Fixture(Some(b"package".to_vec())),
            &mut |_| 0,
        )
        .unwrap();
        let path = Path::new(value.as_str().unwrap());
        assert_eq!(std::fs::metadata(path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            std::fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
            0o700
        );
        verify(path, &r, &mut |_| 0).unwrap();
        std::fs::write(path, b"changed").unwrap();
        assert!(matches!(
            verify(path, &r, &mut |_| 0),
            Err(Error::Invalid(_))
        ));
    }
    #[test]
    fn failed_corrupt_short_long_and_cancelled_downloads_remove_staging() {
        for data in [
            None,
            Some(vec![]),
            Some(b"pack".to_vec()),
            Some(b"package-more".to_vec()),
            Some(b"changed".to_vec()),
        ] {
            let tmp = Temp::new();
            assert!(download(&release(), &tmp.0, &mut Fixture(data), &mut |_| 0).is_err());
            assert_eq!(std::fs::read_dir(&tmp.0).unwrap().count(), 0);
        }
        let tmp = Temp::new();
        let mut polls = 0;
        assert!(matches!(
            download(
                &release(),
                &tmp.0,
                &mut Fixture(Some(b"package".to_vec())),
                &mut |_| {
                    polls += 1;
                    i32::from(polls == 3)
                }
            ),
            Err(Error::Cancelled)
        ));
        assert_eq!(std::fs::read_dir(&tmp.0).unwrap().count(), 0);
    }
    #[test]
    fn rejects_unsafe_source_and_size_before_transport_or_staging() {
        let tmp = Temp::new();
        for r in [
            Release {
                size: MAX_PACKAGE + 1,
                ..release()
            },
            Release {
                asset_url: "http://github.com/file".into(),
                ..release()
            },
            Release {
                digest: "bad".into(),
                ..release()
            },
        ] {
            assert!(matches!(
                download(&r, &tmp.0, &mut Fixture(None), &mut |_| 0),
                Err(Error::Invalid(_))
            ));
        }
        assert_eq!(std::fs::read_dir(&tmp.0).unwrap().count(), 0);
    }
    #[test]
    fn verification_rejects_links_and_parent_replacement_during_hash() {
        let tmp = Temp::new();
        let folder = tmp.0.join("private");
        std::fs::create_dir(&folder).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = folder.join("package.deb");
        std::fs::write(&path, b"package").unwrap();
        let alias = folder.join("alias");
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(verify(&path, &release(), &mut |_| 0).is_err());
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(verify(&alias, &release(), &mut |_| 0).is_err());
        let mut n = 0;
        let result = verify(&path, &release(), &mut |_| {
            n += 1;
            if n == 2 {
                std::fs::rename(&folder, tmp.0.join("moved")).unwrap();
                std::fs::create_dir(&folder).unwrap();
                std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
                std::fs::write(&path, b"package").unwrap();
            }
            0
        });
        assert!(matches!(result, Err(Error::Invalid(_))));
    }
    #[test]
    fn cleanup_preserves_replacement_file() {
        let tmp = Temp::new();
        let staging = Staging::new(&tmp.0).unwrap();
        let path = staging.path.join("neutrondrive.deb");
        std::fs::rename(&path, staging.path.join("original")).unwrap();
        std::fs::write(&path, b"replacement").unwrap();
        drop(staging);
        assert_eq!(std::fs::read(path).unwrap(), b"replacement");
    }
    #[test]
    fn release_response_is_bounded_and_must_be_json() {
        for data in [b"no JSON".to_vec(), vec![b' '; MAX_RELEASES + 1]] {
            assert!(matches!(
                check("0.6.0a6", true, "deb", &mut Fixture(Some(data)), &mut |_| 0),
                Err(Error::Invalid(_))
            ));
        }
        assert_eq!(
            check(
                "0.6.0a6",
                true,
                "deb",
                &mut Fixture(Some(b"[]".to_vec())),
                &mut |_| 0
            )
            .unwrap(),
            Value::Null
        );
    }
    fn tls_server(response: &'static [u8]) -> (String, std::thread::JoinHandle<()>) {
        use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
        let cert = CertificateDer::from_pem_slice(include_bytes!(
            "../tests/fixtures/update_tls/server.pem"
        ))
        .unwrap();
        let key = PrivateKeyDer::from_pem_slice(include_bytes!(
            "../tests/fixtures/update_tls/server-key.pem"
        ))
        .unwrap();
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "https://localhost:{}/test",
            listener.local_addr().unwrap().port()
        );
        let handle = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let conn = rustls::ServerConnection::new(std::sync::Arc::new(config)).unwrap();
            let mut stream = rustls::StreamOwned::new(conn, socket);
            let mut buf = [0u8; 4096];
            let mut request = Vec::new();
            while !request.windows(4).any(|v| v == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            let _ = stream.write_all(response);
            let _ = stream.flush();
        });
        (url, handle)
    }
    fn test_agent(trusted: bool) -> ureq::Agent {
        let cert =
            ureq::tls::Certificate::from_pem(include_bytes!("../tests/fixtures/update_tls/ca.pem"))
                .unwrap();
        let roots = if trusted {
            ureq::tls::RootCerts::new_with_certs(&[cert])
        } else {
            ureq::tls::RootCerts::PlatformVerifier
        };
        agent_config(Mode::Check)
            .proxy(None)
            .tls_config(ureq::tls::TlsConfig::builder().root_certs(roots).build())
            .build()
            .new_agent()
    }
    #[test]
    fn https_checks_certificate_and_refuses_downgrade_redirect() {
        let (url, server) =
            tls_server(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\npackage");
        let mut body = request(test_agent(true), &url, Mode::Check).unwrap();
        let mut bytes = Vec::new();
        body.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"package");
        server.join().unwrap();
        let (url, server) = tls_server(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        assert!(request(test_agent(false), &url, Mode::Check).is_err());
        server.join().unwrap();
        let (url,server)=tls_server(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/insecure\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        assert!(matches!(
            test_agent(true).get(&url).call(),
            Err(ureq::Error::RequireHttpsOnly(_))
        ));
        server.join().unwrap();
    }
}
