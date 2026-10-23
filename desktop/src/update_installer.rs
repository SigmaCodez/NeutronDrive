//! Explicit installation only. Never terminate an in-progress privileged package transaction.
use crate::{runtime::Failure, updates::failure};
use serde_json::{json, Value};
use std::{
    io::Read,
    os::fd::AsRawFd,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub fn install(path: &Path, callback: &mut impl FnMut(Value) -> i32) -> Result<(), Failure> {
    let package = package_name(path, callback)?;
    checked_install(&package, || {
        if callback(json!({"kind":"poll"})) != 0 {
            return Err(Failure {
                message: "Installation cancelled".into(),
                cancelled: true,
            });
        }
        let status = Command::new("/usr/bin/pkexec")
            .args(["/usr/bin/apt-get", "install", "--no-remove", "-y"])
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| failure("Could not start update installer"))?;
        if !status.success() {
            return Err(failure("Update installation failed or was cancelled"));
        }
        Ok(())
    })
}
fn checked_install(
    package: &str,
    install: impl FnOnce() -> Result<(), Failure>,
) -> Result<(), Failure> {
    if package.trim() != "neutrondrive" {
        return Err(failure("Unexpected update package"));
    }
    install()
}
fn package_name(path: &Path, callback: &mut impl FnMut(Value) -> i32) -> Result<String, Failure> {
    let mut child = Command::new("/usr/bin/dpkg-deb")
        .arg("-f")
        .arg(path)
        .arg("Package")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| failure("Could not inspect update package"))?;
    let result = (|| {
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| failure("Package output unavailable"))?;
        let fd = stdout.as_raw_fd();
        // SAFETY: stdout owns this descriptor throughout the bounded read loop.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(failure("Could not inspect package output"));
        }
        let started = Instant::now();
        let mut output = Vec::new();
        let mut done = false;
        loop {
            if callback(json!({"kind":"poll"})) != 0 {
                return Err(Failure {
                    message: "Package inspection cancelled".into(),
                    cancelled: true,
                });
            }
            if started.elapsed() > Duration::from_secs(10) {
                return Err(failure("Package inspection timed out"));
            }
            let mut buffer = [0u8; 1024];
            match stdout.read(&mut buffer) {
                Ok(0) => done = true,
                Ok(n) => {
                    output.extend_from_slice(&buffer[..n]);
                    if output.len() > 4096 {
                        return Err(failure("Unexpected package output"));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => (),
                Err(_) => return Err(failure("Could not read package output")),
            }
            if let Some(status) = child
                .try_wait()
                .map_err(|_| failure("Could not inspect package status"))?
            {
                if !status.success() {
                    return Err(failure("Invalid update package"));
                }
                if done {
                    return String::from_utf8(output).map_err(|_| failure("Invalid package name"));
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    // The inspection process is unprivileged and always reaped. The installer above
    // is deliberately not subject to this cancellation path.
    let _ = child.kill();
    let _ = child.wait();
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_synthetic_deb_metadata_and_cancels_inspection() {
        let root =
            std::env::temp_dir().join(format!("nd-update-deb-inspection-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let control = root.join("payload/DEBIAN");
        std::fs::create_dir_all(&control).unwrap();
        std::fs::write(control.join("control"),"Package: neutrondrive\nVersion: 0.0.0\nArchitecture: all\nMaintainer: Synthetic Test <test@example.invalid>\nDescription: Synthetic metadata inspection only\n").unwrap();
        let package = root.join("fixture.deb");
        let status = Command::new("/usr/bin/dpkg-deb")
            .arg("--build")
            .arg(root.join("payload"))
            .arg(&package)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            package_name(&package, &mut |_| 0).unwrap().trim(),
            "neutrondrive"
        );
        assert!(package_name(&package, &mut |_| 1).unwrap_err().cancelled);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_wrong_package_before_privilege_prompt() {
        assert!(checked_install("other", || panic!("must not install")).is_err());
        assert!(checked_install("neutrondrive\n", || Ok(())).is_ok());
        assert!(checked_install("neutrondrive", || Err(failure("denied"))).is_err());
    }
    #[test]
    fn invalid_package_is_reaped_without_installing() {
        assert!(package_name(
            Path::new("/nonexistent/neutrondrive-test-package"),
            &mut |_| 0
        )
        .is_err());
    }
}
