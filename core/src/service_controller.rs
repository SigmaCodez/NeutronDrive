//! Service process ownership and authenticated control, shared by native UI/CLI.
use crate::{control, process::Failure, storage};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{fs::MetadataExt, process::CommandExt},
    },
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Failure>;
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Status,
    Start {
        argv: Vec<String>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    Stop,
    StopIdle,
}
pub fn status(state: &Path) -> Result<Value> {
    if let Ok(value) = control::command(&state.join("control.sock"), "status", None, 5.0) {
        return Ok(value);
    }
    let mut value = storage::read(
        &state.join("status.json"),
        json!({"jobs":[],"event_source":{"state":"stopped"}}),
        8 * 1024 * 1024,
        true,
    )?;
    if !value.is_object() || !value["jobs"].is_array() {
        return Err("Gespeicherter Dienststatus ist beschädigt.".into());
    }
    value["running"] = json!(false);
    Ok(value)
}
fn log(state: &Path) -> Result<File> {
    let parent = storage::directory(state, false, true)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            c"daemon.log".as_ptr(),
            libc::O_WRONLY
                | libc::O_CREAT
                | libc::O_APPEND
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err("Privates Dienstprotokoll ist nicht sicher zugänglich.".into());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let info = file
        .metadata()
        .map_err(|_| "Dienstprotokoll ist nicht lesbar.")?;
    if !info.is_file()
        || info.uid() != unsafe { libc::getuid() }
        || info.nlink() != 1
        || info.mode() & 0o077 != 0
    {
        return Err("Ungültiges privates Dienstprotokoll.".into());
    }
    Ok(file)
}
fn terminate(child: &mut Child, wait: Duration) -> bool {
    match child.try_wait() {
        Ok(Some(_)) => return true,
        Ok(None) => {}
        Err(_) => return false,
    }
    // An unreaped child keeps this PID reserved; never signal a saved arbitrary PID.
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let deadline = Instant::now() + wait;
    loop {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn start(
    state: &Path,
    argv: Vec<String>,
    environment: BTreeMap<String, String>,
    timeout: Duration,
) -> Result<Value> {
    let existing = status(state)?;
    if existing["running"] == true {
        return Ok(existing);
    }
    if argv.is_empty()
        || argv[0].is_empty()
        || argv.iter().any(|s| s.contains('\0'))
        || environment
            .iter()
            .any(|(k, v)| k.is_empty() || k.contains(['=', '\0']) || v.contains('\0'))
    {
        return Err("Ungültiger Dienststart.".into());
    }
    let output = log(state)?;
    let error = output
        .try_clone()
        .map_err(|_| "Dienstprotokoll konnte nicht geöffnet werden.")?;
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(output)
        .stderr(error);
    let maximum = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
    // Only async-signal-safe syscalls in the forked child. Preserve the exec-error
    // pipe until exec, but ensure no host application's descriptors leak through.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 4u32) < 0 {
                if maximum < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                for fd in 3..(maximum as i64).min(i32::MAX as i64) as i32 {
                    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                }
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|_| Failure::from("Hintergrunddienst konnte nicht gestartet werden."))?;
    let deadline = Instant::now() + timeout;
    loop {
        match status(state) {
            Ok(value) if value["running"] == true => {
                // Reap on eventual exit without blocking the caller or owning the daemon lifetime.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(value);
            }
            Err(error) => {
                if !terminate(&mut child, Duration::from_secs(10)) {
                    return Err(
                        "Dienststart fehlgeschlagen; Beendigung noch nicht bestätigt.".into(),
                    );
                }
                return Err(error);
            }
            _ => {}
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                return Err(
                    "Hintergrunddienst konnte nicht starten; privates Dienstprotokoll prüfen."
                        .into(),
                )
            }
            Err(_) => {
                terminate(&mut child, Duration::from_secs(10));
                return Err("Dienstprozess konnte nicht geprüft werden.".into());
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if !terminate(&mut child, Duration::from_secs(10)) {
        return Err(
            "Dienststart hat das Zeitlimit überschritten; Beendigung noch nicht bestätigt.".into(),
        );
    }
    Err("Dienststart hat das Zeitlimit überschritten; der gestartete Prozess wurde beendet.".into())
}
pub fn run(state: &Path, operation: Operation) -> Result<Value> {
    storage::directory(state, true, true)?;
    match operation {
        Operation::Status => status(state),
        Operation::Start { argv, environment } => {
            start(state, argv, environment, Duration::from_secs(8))
        }
        Operation::Stop | Operation::StopIdle => {
            let existing = status(state)?;
            if existing["running"] != true {
                if matches!(operation, Operation::StopIdle) && state.join("control.sock").exists() {
                    return Err(
                        "Cannot confirm that the sync daemon is stopped; update was not installed."
                            .into(),
                    );
                }
                return Ok(existing);
            }
            let socket = state.join("control.sock");
            control::command(
                &socket,
                if matches!(operation, Operation::StopIdle) {
                    "stop_idle"
                } else {
                    "stop"
                },
                None,
                5.0,
            )?;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                match std::fs::symlink_metadata(&socket) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                    Err(_) => return Err("Dienstsocket ist nicht prüfbar.".into()),
                    Ok(_) => {}
                }
                if Instant::now() >= deadline {
                    return Err("Dienstbeendigung ist noch nicht bestätigt; laufende Übertragung wird abgebrochen.".into());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            status(state)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{symlink, PermissionsExt},
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Temp(std::path::PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "nd-controller-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn offline_status_preserves_jobs_but_clears_running() {
        let state = Temp::new();
        assert_eq!(run(&state.0, Operation::Status).unwrap()["running"], false);
        storage::write(
            &state.0.join("status.json"),
            json!({"jobs":[{"id":"saved"}],"running":true}),
        )
        .unwrap();
        let value = status(&state.0).unwrap();
        assert_eq!(value["jobs"][0]["id"], "saved");
        assert_eq!(value["running"], false);
        storage::write(&state.0.join("status.json"), json!([])).unwrap();
        assert!(status(&state.0).is_err());
    }
    #[test]
    fn private_log_rejects_symlinks_and_hardlinks() {
        let state = Temp::new();
        let target = state.0.join("target");
        fs::write(&target, b"preserve").unwrap();
        symlink(&target, state.0.join("daemon.log")).unwrap();
        assert!(log(&state.0).is_err());
        fs::remove_file(state.0.join("daemon.log")).unwrap();
        fs::hard_link(&target, state.0.join("daemon.log")).unwrap();
        assert!(log(&state.0).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"preserve");
    }
    #[test]
    fn acknowledged_start_and_stop_exchange_with_same_user_service() {
        use std::{
            io::{BufRead, BufReader, Write},
            os::unix::net::UnixListener,
        };
        let state = Temp::new();
        let path = state.0.clone();
        let ready = path.join("ready.pid");
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !ready.exists() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(10));
            }
            let pid: i32 = fs::read_to_string(&ready).unwrap().trim().parse().unwrap();
            let socket = path.join("control.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            for _ in 0..3 {
                let (mut peer, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&mut peer).read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                peer.write_all(b"{\"ok\":true,\"result\":{\"running\":true,\"jobs\":[]}}\n")
                    .unwrap();
                if request["action"] == "stop" {
                    unsafe {
                        libc::kill(pid, libc::SIGTERM);
                    }
                    fs::remove_file(&socket).unwrap();
                    return;
                }
            }
            panic!("stop was not received");
        });
        let result = start(
            &state.0,
            vec![
                "/bin/sh".into(),
                "-c".into(),
                "echo $$ > \"$1\"; exec sleep 60".into(),
                "fixture".into(),
                state.0.join("ready.pid").to_str().unwrap().into(),
            ],
            BTreeMap::new(),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(result["running"], true);
        assert_eq!(run(&state.0, Operation::Stop).unwrap()["running"], false);
        server.join().unwrap();
    }
    #[test]
    fn failed_and_unacknowledged_start_do_not_leave_processes() {
        let state = Temp::new();
        assert!(start(
            &state.0,
            vec!["/bin/false".into()],
            BTreeMap::new(),
            Duration::from_secs(1)
        )
        .unwrap_err()
        .message
        .contains("nicht starten"));
        let pidfile = state.0.join("child.pid");
        let error = start(
            &state.0,
            vec![
                "/bin/sh".into(),
                "-c".into(),
                "echo $$ > \"$1\"; exec sleep 60".into(),
                "test".into(),
                pidfile.to_str().unwrap().into(),
            ],
            BTreeMap::new(),
            Duration::from_millis(200),
        )
        .unwrap_err();
        assert!(error.message.contains("Prozess wurde beendet"));
        let pid: i32 = fs::read_to_string(pidfile).unwrap().trim().parse().unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
}
