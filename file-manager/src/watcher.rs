use crate::{availability, paths};
use std::{
    collections::BTreeSet,
    os::fd::AsRawFd,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub fn run(parent: u32) -> Result<(), String> {
    let state = paths::state_directory().ok_or("No state directory")?;
    std::fs::create_dir_all(&state).map_err(|e| e.to_string())?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(state.join("emblems.lock"))
        .map_err(|e| e.to_string())?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(());
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let history = state.join("emblems-applied.json");
    use std::io::Read;
    let mut previous = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&history)
        .ok()
        .and_then(|f| {
            if !f.metadata().ok()?.is_file() {
                return None;
            }
            let mut bytes = Vec::new();
            f.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes).ok()?;
            if bytes.len() > 16 * 1024 * 1024 {
                return None;
            }
            serde_json::from_slice::<BTreeSet<PathBuf>>(&bytes).ok()
        })
        .unwrap_or_default();
    let mut retry = Instant::now();
    while unsafe { libc::kill(parent as i32, 0) } == 0 {
        let now = availability::all_available();
        let reset = retry.elapsed() > Duration::from_secs(300);
        let changed: Vec<_> = if reset {
            now.union(&previous).cloned().collect()
        } else {
            now.symmetric_difference(&previous).cloned().collect()
        };
        if reset {
            retry = Instant::now();
        }
        for path in changed {
            if unsafe { libc::kill(parent as i32, 0) } != 0 {
                return Ok(());
            }
            let available = now.contains(&path);
            let mut child = Command::new(&executable)
                .arg("--apply-emblem")
                .arg(if available { "1" } else { "0" })
                .arg(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| e.to_string())?;
            let start = Instant::now();
            let success = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status.success(),
                    Ok(None) if start.elapsed() < Duration::from_secs(2) => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break false;
                    }
                }
            };
            if success {
                if available {
                    previous.insert(path);
                } else {
                    previous.remove(&path);
                }
            }
        }
        let temporary = state.join(format!(".emblems-applied-{}", std::process::id()));
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
        {
            use std::io::Write;
            if let Ok(bytes) = serde_json::to_vec(&previous) {
                if f.write_all(&bytes).is_ok() && f.sync_all().is_ok() {
                    let _ = std::fs::rename(&temporary, &history);
                }
            }
            let _ = std::fs::remove_file(&temporary);
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    Ok(())
}
