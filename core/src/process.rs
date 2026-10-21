//! Linux subprocess ownership. Pipes, locks and process groups never escape Rust.
//! The synchronous callback reports login URLs and polls cancellation; no worker
//! thread may call it after `run` returns. Child output is never logged.
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const LIMIT: usize = 64 * 1024 * 1024;
const CANCELLED: &str = "Vorgang abgebrochen; bereits übertragene Dateien bleiben erhalten.";
use libc::{
    fcntl, flock, getuid, kill, setsid, F_GETFL, F_SETFL, LOCK_EX, LOCK_NB, O_DIRECTORY,
    O_NOFOLLOW, O_NONBLOCK, SIGKILL, SIGTERM,
};

#[derive(Debug)]
pub struct Failure {
    pub message: String,
    pub cancelled: bool,
}
impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self {
            message: message.into(),
            cancelled: false,
        }
    }
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            message,
            cancelled: false,
        }
    }
}
type Result<T> = std::result::Result<T, Failure>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub executable: String,
    pub args: Vec<String>,
    pub timeout: f64,
    pub parse: bool,
    pub login: bool,
    pub max_output: usize,
}

fn notify(callback: &mut impl FnMut(Value) -> i32, event: Value) -> Result<()> {
    if callback(event) != 0 {
        return Err(Failure {
            message: CANCELLED.into(),
            cancelled: true,
        });
    }
    Ok(())
}

pub fn data_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state")))
        .ok_or("Das NeutronDrive-Zustandsverzeichnis ist nicht zugänglich.")?;
    let path = base.join("neutrondrive");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&path)
        .map_err(|_| "Das NeutronDrive-Zustandsverzeichnis ist nicht zugänglich.")?;
    // Open the final directory without following a link, and chmod the opened
    // inode rather than a path that can be replaced between stat and chmod.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW | O_DIRECTORY)
        .open(&path)
        .map_err(|_| "Unsicheres NeutronDrive-Zustandsverzeichnis.")?;
    let metadata = file
        .metadata()
        .map_err(|_| "Unsicheres NeutronDrive-Zustandsverzeichnis.")?;
    // SAFETY: getuid has no preconditions.
    if metadata.uid() != unsafe { getuid() } {
        return Err("Unsicheres NeutronDrive-Zustandsverzeichnis.".into());
    }
    file.set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|_| "Das NeutronDrive-Zustandsverzeichnis ist nicht zugänglich.")?;
    Ok(path)
}

struct OwnedChild {
    child: Child,
    reaped: bool,
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            // The leader has not been reaped, so its PID cannot be reused.
            // Kill the entire private process group, including pipe holders.
            unsafe {
                kill(-(self.child.id() as i32), SIGTERM);
            }
            // Allow the SDK its previous two-second termination grace without
            // reaping the leader (and releasing its PID) before group cleanup.
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                let result = unsafe {
                    libc::waitid(
                        libc::P_PID,
                        self.child.id(),
                        &mut info,
                        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                    )
                };
                if result == 0 && unsafe { info.si_pid() } != 0 {
                    break;
                }
                if result < 0
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD)
                {
                    // An external SIGCHLD handler reaped this process. Its PID
                    // is no longer ours, so never signal that numeric ID again.
                    self.reaped = true;
                    return;
                }
                if Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            unsafe {
                kill(-(self.child.id() as i32), SIGKILL);
            }
            let _ = self.child.wait();
        }
    }
}

fn nonblocking(fd: RawFd) -> Result<()> {
    // SAFETY: fd is a live owned pipe; both fcntl commands take integer flags.
    let flags = unsafe { fcntl(fd, F_GETFL) };
    if flags < 0 || unsafe { fcntl(fd, F_SETFL, flags | O_NONBLOCK) } < 0 {
        return Err("Die Backend-Ausgabe konnte nicht überwacht werden.".into());
    }
    Ok(())
}

fn read_pipe(
    pipe: &mut impl Read,
    output: &mut Vec<u8>,
    eof: &mut bool,
    limit: usize,
) -> Result<()> {
    if *eof {
        return Ok(());
    }
    let mut bytes = [0u8; 65536];
    match pipe.read(&mut bytes) {
        Ok(0) => *eof = true,
        Ok(count) => {
            if output.len().saturating_add(count) > limit {
                return Err("Die Backend-Antwort überschreitet die Größenbegrenzung. Bitte kleinere Ordner verwenden.".into());
            }
            output.extend_from_slice(&bytes[..count]);
        }
        Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => (),
        Err(_) => {
            return Err("Die Backend-Ausgabe konnte nicht vollständig gelesen werden.".into())
        }
    }
    Ok(())
}

pub fn login_url(bytes: &[u8]) -> Result<String> {
    let error = "Die Proton-CLI lieferte keine vertrauenswürdige Anmelde-URL.";
    let value: Value = serde_json::from_slice(bytes).map_err(|_| error)?;
    let url = value
        .get("signInUrl")
        .and_then(Value::as_str)
        .ok_or(error)?;
    // Authority must be exact. Reject userinfo, backslashes, controls, odd ports
    // and whitespace instead of asking a browser to normalize untrusted input.
    if url
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return Err(error.into());
    }
    let tail = url.strip_prefix("https://").ok_or(error)?;
    let authority = tail.split(['/', '?', '#']).next().unwrap_or("");
    if !matches!(authority, "account.proton.me" | "account.proton.me:443") {
        return Err(error.into());
    }
    Ok(url.into())
}

pub fn checked_json(output: &str) -> Result<Value> {
    if output.trim().is_empty() {
        return Ok(Value::Null);
    }
    let value: Value = serde_json::from_str(output).map_err(|_| {
        "Die Proton-CLI lieferte keine vollständige JSON-Antwort. Es wurde kein Erfolg angenommen."
    })?;
    if let Some(items) = value.as_array() {
        let count = items
            .iter()
            .filter(|x| x.get("ok") == Some(&Value::Bool(false)))
            .count();
        if count > 0 {
            return Err(format!("Proton meldet {count} fehlgeschlagene Dateioperation(en). Bitte Ansicht aktualisieren.").into());
        }
    }
    if let Some(failed) = value.get("failedItems") {
        if failed.as_u64() != Some(0) {
            // Never interpolate untyped SDK values into a user-visible error.
            return Err("Übertragung(en) fehlgeschlagen oder ungültiger Ergebnisstatus. Erfolgreiche Teilübertragungen bleiben erhalten.".into());
        }
    }
    Ok(value)
}

pub fn friendly_error(stderr: &str) -> &'static str {
    let lower = stderr.to_lowercase();
    for (needles, message) in [
        (&["not authenticated", "not logged in", "authentication required", "auth login", "log in first", "login first"][..], "Bitte zuerst über den Browser bei Proton anmelden."),
        (&["libsecret", "secret service", "keyring", "credentials store", "dbus"][..], "Der Linux-Schlüsselbund ist nicht erreichbar. GNOME Keyring oder KWallet entsperren und Secret-Service-Zugriff prüfen."),
        (&["429", "rate limit", "too many requests"][..], "Proton begrenzt gerade die Anfragen. Bitte später erneut versuchen."),
        (&["quota", "insufficient storage"][..], "Der verfügbare Cloud-Speicher reicht für diese Übertragung nicht aus."),
        (&["not found"][..], "Die Datei oder der Ordner wurde nicht gefunden. Bitte aktualisieren."),
        (&["network", "fetch failed", "connection", "enotfound", "timeout", "timed out"][..], "Verbindung zu Proton fehlgeschlagen. Bitte Netzwerk prüfen und erneut versuchen."),
    ] { if needles.iter().any(|s| lower.contains(s)) { return message; } }
    "Die Proton-CLI hat die Aktion abgebrochen. Bitte Anmeldung, Zugriffsrechte und Netzwerk prüfen. Rohdaten werden zum Schutz von Kontodaten nicht angezeigt."
}

pub fn run(request: Request, callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    if request.version != 1
        || !request.timeout.is_finite()
        || request.timeout <= 0.0
        || request.timeout > 86400.0
        || request.max_output == 0
        || request.max_output > LIMIT
    {
        return Err("Ungültiger Backend-Auftrag.".into());
    }
    notify(callback, json!({"kind":"poll"}))?;
    let environment: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
    if environment
        .get(std::ffi::OsStr::new("PROTON_DRIVE_CREDENTIALS_STORE"))
        .is_some_and(|v| v != "keychain" && v != "pass")
    {
        return Err(
            "NeutronDrive erlaubt nur einen verschlüsselten Schlüsselbund (keychain oder pass)."
                .into(),
        );
    }
    if environment
        .get(std::ffi::OsStr::new("PROTON_DRIVE_BASE_URL"))
        .is_some_and(|v| v != "drive-api.proton.me")
    {
        return Err(
            "NeutronDrive verwendet ausschließlich den offiziellen Proton-API-Endpunkt.".into(),
        );
    }
    let lock_path = data_dir()?.join("backend.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(O_NOFOLLOW | O_NONBLOCK)
        .open(lock_path)
        .map_err(|_| "Die Backend-Sperre ist nicht zugänglich.")?;
    let metadata = lock
        .metadata()
        .map_err(|_| "Die Backend-Sperre ist nicht zugänglich.")?;
    if !metadata.is_file() || metadata.uid() != unsafe { getuid() } || metadata.nlink() != 1 {
        return Err("Unsichere Backend-Sperre.".into());
    }
    let started = Instant::now();
    loop {
        notify(callback, json!({"kind":"poll"}))?;
        // SAFETY: fd remains owned throughout this scope; flock is advisory.
        if unsafe { flock(lock.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0 {
            break;
        }
        let error = std::io::Error::last_os_error();
        if !matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) {
            return Err("Die Backend-Sperre konnte nicht erworben werden.".into());
        }
        if started.elapsed().as_secs_f64() >= request.timeout.min(120.0) {
            return Err("Das Backend ist noch beschäftigt. Bitte erneut versuchen.".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    notify(callback, json!({"kind":"poll"}))?;
    let mut command = Command::new(&request.executable);
    command
        .args(&request.args)
        .env_clear()
        .envs(environment)
        .env("NO_COLOR", "1")
        .env("PROTON_DRIVE_LOG_LEVEL", "WARNING")
        .env("PROTON_DRIVE_UNSAFE_CACHE", "false")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: after fork this closure only invokes async-signal-safe setsid
    // and reads errno. It allocates nothing and never locks or calls Python.
    unsafe {
        command.pre_exec(|| {
            if setsid() < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let child = command.spawn().map_err(|_| {
        "Die Proton-CLI konnte nicht gestartet werden. Architektur und Bibliotheken prüfen."
    })?;
    let mut owner = OwnedChild {
        child,
        reaped: false,
    };
    let mut stdout = owner
        .child
        .stdout
        .take()
        .ok_or("Die Backend-Ausgabe fehlt.")?;
    let mut stderr = owner
        .child
        .stderr
        .take()
        .ok_or("Die Backend-Ausgabe fehlt.")?;
    nonblocking(stdout.as_raw_fd())?;
    nonblocking(stderr.as_raw_fd())?;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let (mut out_eof, mut err_eof, mut login_reported) = (false, false, false);
    let started = Instant::now();
    let status = loop {
        notify(callback, json!({"kind":"poll"}))?;
        if started.elapsed().as_secs_f64() > request.timeout {
            return Err("Zeitlimit erreicht. Der Vorgang wurde beendet; bitte Cloud-Status vor einem erneuten Versuch prüfen.".into());
        }
        let before = out.len() + err.len();
        read_pipe(&mut stdout, &mut out, &mut out_eof, request.max_output)?;
        read_pipe(&mut stderr, &mut err, &mut err_eof, request.max_output)?;
        if request.login && !login_reported {
            if let Some(end) = out.iter().position(|b| *b == b'\n') {
                let url = login_url(&out[..end])?;
                notify(callback, json!({"kind":"login","url":url}))?;
                login_reported = true;
            }
        }
        // Do not reap a leader while descendants still hold inherited pipes:
        // cleanup must retain ownership of the group ID until pipes close.
        if out_eof && err_eof {
            if let Some(status) = owner
                .child
                .try_wait()
                .map_err(|_| "Der Backend-Prozess konnte nicht beendet werden.")?
            {
                owner.reaped = true;
                break status;
            }
        }
        if before == out.len() + err.len() {
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    if !status.success() {
        return Err(friendly_error(&String::from_utf8_lossy(&err)).into());
    }
    if request.login && !login_reported {
        return Err("Die Proton-CLI hat keine Anmeldung bestätigt. Bitte erneut anmelden.".into());
    }
    let output = String::from_utf8_lossy(&out);
    if request.parse {
        checked_json(&output)
    } else {
        Ok(json!(output.trim()))
    }
}
