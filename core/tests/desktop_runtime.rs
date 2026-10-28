//! Production server, SDK workers and shutdown exercised with no Python on PATH.
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
const PATH: &str = "/io/github/SigmaCodez/NeutronDrive/Service";
fn invoke(value: Value, callback: &mut impl FnMut(Value) -> i32) -> Value {
    neutrondrive_core::runtime_request(value.to_string().as_bytes(), callback)
}
#[test]
fn isolated_native_service() {
    if std::env::var("NEUTRONDRIVE_DBUS_TEST").as_deref() != Ok("1") {
        return;
    }
    // Deliberately give children an outer profile: the actual service fixture
    // must select its own profile, leaving this sentinel byte-for-byte intact.
    let profile =
        Temp(std::env::temp_dir().join(format!("nd-native-outer-profile-{}", std::process::id())));
    let bookmarks = profile.0.join("config/gtk-3.0/bookmarks");
    std::fs::create_dir_all(bookmarks.parent().unwrap()).unwrap();
    let sentinel =
        "file:///fixture/Documents Documents\nfile:///fixture/ProtonDrive Proton Drive\n";
    std::fs::write(&bookmarks, sentinel).unwrap();
    for case in ["native", "snap", "shutdown", "mount", "process"] {
        if case == "mount" && std::env::var("NEUTRONDRIVE_FUSE_TEST").as_deref() != Ok("1") {
            continue;
        }
        let mut child = Command::new("/usr/bin/dbus-run-session")
            .args(["--dbus-daemon=/usr/bin/dbus-daemon", "--"])
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", "native_service_child", "--nocapture"])
            .env("ND_NATIVE_SERVICE_CASE", case)
            .env("XDG_CONFIG_HOME", profile.0.join("config"))
            .env("XDG_CACHE_HOME", profile.0.join("cache"))
            .env("XDG_DATA_HOME", profile.0.join("data"))
            .env("PATH", "/nonexistent")
            .env_remove("FLATPAK_ID")
            .env_remove("SNAP")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(75);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() > deadline {
                let _ = child.kill();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "{case} timed out: {} {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            std::fs::read_to_string(&bookmarks).unwrap(),
            sentinel,
            "Service test modified its inherited profile"
        );
        assert!(
            output.status.success(),
            "{case}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Server {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<Value>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            assert_eq!(thread.join().unwrap()["ok"], true);
        }
    }
}
#[test]
fn native_service_child() {
    use std::os::unix::fs::PermissionsExt;
    let Ok(case) = std::env::var("ND_NATIVE_SERVICE_CASE") else {
        return;
    };
    if case == "process" {
        native_process();
        return;
    }
    let root = Temp(std::env::temp_dir().join(format!("nd-native-server-{}", std::process::id())));
    std::fs::create_dir(&root.0).unwrap();
    std::env::set_var("XDG_STATE_HOME", root.0.join("state"));
    std::env::set_var("XDG_CONFIG_HOME", root.0.join("config"));
    std::env::set_var("XDG_CACHE_HOME", root.0.join("cache"));
    std::env::set_var("XDG_DATA_HOME", root.0.join("data"));
    if case == "snap" {
        std::env::set_var("SNAP", "/snap/neutrondrive/test");
    }
    let sdk = root.0.join("sdk");
    std::fs::write(&sdk,r#"#!/bin/sh
for last do :; done
if [ "$1" = "auth" ]; then echo $$ > "$SDK_PID_FILE"; exec /bin/sleep 60; fi
if [ "$1" = "events" ]; then printf '%s\n' '{"protocol":1,"type":"batch","roots":[{"path":"/my-files","uid":"root","scopeId":"scope"}],"events":[],"cursors":{"scope":"next"},"resets":[]}'; exit 0; fi
if [ "$2" = "info" ] && [ "$last" != "/my-files" ] && [ -f "$SDK_FAIL_FILE" ]; then printf 'network synthetic outage\n' >&2; exit 1; fi
file='{"uid":"data","name":{"ok":true,"value":"data"},"type":"file","activeRevision":{"uid":"r1","claimedSize":3,"claimedDigests":{"sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","sha1Verified":true}}}'
if [ "$2" = "download" ]; then printf abc > "$last/data"; printf '%s\n' '{}'; exit 0; fi
if [ "$2" = "list" ]; then printf '[%s]\n' "$file"; exit 0; fi
if [ "$last" = "/my-files" ]; then printf '%s\n' '{"uid":"root","name":{"ok":true,"value":"root"},"type":"folder"}'; exit 0; fi
printf '%s\n' "$file"
"#).unwrap();
    std::fs::set_permissions(&sdk, std::fs::Permissions::from_mode(0o700)).unwrap();
    let pidfile = root.0.join("sdk.pid");
    std::env::set_var("SDK_PID_FILE", &pidfile);
    let failfile = root.0.join("sdk-offline");
    std::env::set_var("SDK_FAIL_FILE", &failfile);
    if case == "mount" {
        let helpers = root.0.join("helpers");
        std::fs::create_dir(&helpers).unwrap();
        std::os::unix::fs::symlink("/usr/bin/fusermount3", helpers.join("fusermount3")).unwrap();
        std::env::set_var("PATH", helpers);
    }
    let stop = Arc::new(AtomicBool::new(false));
    let cancel = stop.clone();
    let config = json!({"state_dir":root.0.join("service"),"executable":sdk});
    let owned = config.clone();
    let mut server = Server {
        stop,
        thread: Some(std::thread::spawn(move || {
            invoke(
                json!({"version":1,"op":"desktop_runtime","config":owned}),
                &mut |event| {
                    assert_eq!(event["kind"], "poll");
                    i32::from(cancel.load(Ordering::Acquire))
                },
            )
        })),
    };
    let name = if case == "snap" {
        "io.github.SigmaCodez.NeutronDrive.SnapService"
    } else {
        "io.github.SigmaCodez.NeutronDrive.Service"
    };
    let connection = zbus::block_on(zbus::Connection::session()).unwrap();
    let request = |value: Value| -> Value {
        zbus::block_on(async {
            let response = connection
                .call_method(
                    Some(name),
                    PATH,
                    Some(name),
                    "Request",
                    &(value.to_string(),),
                )
                .await
                .unwrap();
            let raw: String = response.body().deserialize().unwrap();
            serde_json::from_str(&raw).unwrap()
        })
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let owned = zbus::block_on(connection.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "NameHasOwner",
            &(name,),
        ))
        .unwrap()
        .body()
        .deserialize::<bool>()
        .unwrap();
        if owned {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let status = request(json!({"action":"status"}));
    assert_eq!(status["result"]["available"], true);
    assert_eq!(status["result"]["mounted"], false);
    // A second server must not replace the active owner's bus name.
    let duplicate = invoke(
        json!({"version":1,"op":"desktop_runtime","config":{"state_dir":root.0.join("duplicate"),"executable":sdk}}),
        &mut |_| 0,
    );
    assert_eq!(duplicate["ok"], false);
    assert_eq!(request(json!({"action":"status"}))["ok"], true);
    assert_eq!(
        request(json!({"action":"shell","command":"anything"}))["ok"],
        false
    );
    let oversized = "x".repeat(16385);
    assert!(zbus::block_on(connection.call_method(
        Some(name),
        PATH,
        Some(name),
        "Request",
        &(oversized,)
    ))
    .is_err());
    if case == "mount" {
        let point = root.0.join("mount");
        let result = request(json!({"action":"mount","mountpoint":point}));
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(std::fs::read(point.join("data")).unwrap(), b"abc");
        assert_eq!(request(json!({"action":"login"}))["ok"], false);
        let deadline = Instant::now() + Duration::from_secs(5);
        while request(json!({"action":"status"}))["result"]["events"]["state"] != "ready" {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        let held = std::fs::File::open(point.join("data")).unwrap();
        assert_eq!(request(json!({"action":"unmount"}))["ok"], false);
        let still_live = request(json!({"action":"status"}));
        assert_eq!(still_live["result"]["mounted"], true);
        assert_eq!(still_live["result"]["events"]["state"], "ready");
        drop(held);
        assert_eq!(request(json!({"action":"pin","path":"data"}))["ok"], true);
        assert_eq!(request(json!({"action":"unmount"}))["ok"], true);
        // The persisted pin is refreshed by the real event->SDK path on remount.
        // A failed download/info must retain the event for automatic retry.
        std::fs::write(&failfile, b"offline").unwrap();
        assert_eq!(
            request(json!({"action":"mount","mountpoint":point}))["ok"],
            true
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = request(json!({"action":"status"}));
            if status["result"]["events"]["state"] == "error" {
                assert!(status["result"]["error"]
                    .as_str()
                    .unwrap()
                    .contains("pinned refresh is pending"));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Failed pin was incorrectly acknowledged: {status}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::remove_file(&failfile).unwrap();
        let deadline = Instant::now() + Duration::from_secs(50);
        loop {
            let status = request(json!({"action":"status"}));
            if status["result"]["events"]["state"] == "ready" {
                assert_eq!(status["result"]["error"], "", "{status}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Cloud refresh did not recover: {status}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(std::fs::read(point.join("data")).unwrap(), b"abc");
        assert_eq!(request(json!({"action":"unmount"}))["ok"], true);
        assert_eq!(
            request(json!({"action":"status"}))["result"]["events"]["state"],
            "stopped"
        );
        assert_eq!(
            request(json!({"action":"mount","mountpoint":point,"offline":true}))["ok"],
            true
        );
        assert_eq!(std::fs::read(point.join("data")).unwrap(), b"abc");
        // Leave the mounted drive for native shutdown cleanup to exercise.
    } else {
        let login = std::thread::spawn(move || {
            zbus::block_on(async {
                let connection = zbus::Connection::session().await.unwrap();
                connection
                    .call_method(
                        Some(name),
                        PATH,
                        Some(name),
                        "Request",
                        &("{\"action\":\"login\"}",),
                    )
                    .await
            })
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while !pidfile.exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            request(json!({"action":"status"}))["result"]["operation"],
            "login"
        );
        if case == "shutdown" {
            server.stop.store(true, Ordering::Release);
        } else {
            assert_eq!(request(json!({"action":"cancel"}))["ok"], true);
        }
        let reply = login.join().unwrap();
        if case != "shutdown" {
            let raw: String = reply.unwrap().body().deserialize().unwrap();
            assert_eq!(serde_json::from_str::<Value>(&raw).unwrap()["ok"], false);
            assert_eq!(request(json!({"action":"status"}))["result"]["busy"], false);
        } else {
            // The bus disconnects before the worker has reaped its SDK child.
            assert_eq!(server.thread.take().unwrap().join().unwrap()["ok"], true);
        }
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }
    server.stop.store(true, Ordering::Release);
    if let Some(thread) = server.thread.take() {
        assert_eq!(thread.join().unwrap()["ok"], true);
    }
    if case == "mount" {
        let text = std::fs::read_to_string(root.0.join("config/gtk-3.0/bookmarks")).unwrap();
        assert_eq!(
            text.lines().count(),
            1,
            "Repeated mounts must not duplicate the favorite"
        );
        assert!(text.contains(&format!(
            "file://{} Proton Drive",
            root.0.join("mount").display()
        )));
    }
    assert!(!root.0.join("mount/data").exists());
    assert!(!std::fs::read_to_string("/proc/self/mountinfo")
        .unwrap()
        .contains(&format!(" {} ", root.0.join("mount").display())));
    zbus::block_on(connection.close()).unwrap();
}

fn native_process() {
    use std::io::Write;
    let root =
        Temp(std::env::temp_dir().join(format!("nd-desktop-process-{}", std::process::id())));
    std::fs::create_dir(&root.0).unwrap();
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(
        Command::new(env!("CARGO_BIN_EXE_neutrondrive-runtime"))
            .env("XDG_STATE_HOME", root.0.join("state"))
            .env("XDG_CONFIG_HOME", root.0.join("config"))
            .env("XDG_CACHE_HOME", root.0.join("cache"))
            .env("XDG_DATA_HOME", root.0.join("data"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    child.0.stdin.take().unwrap().write_all(json!({"version":1,"op":"desktop_runtime","config":{"state_dir":root.0.join("service"),"executable":"/nonexistent-sdk"}}).to_string().as_bytes()).unwrap();
    let connection = zbus::block_on(zbus::Connection::session()).unwrap();
    let name = "io.github.SigmaCodez.NeutronDrive.Service";
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let owned = zbus::block_on(connection.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "NameHasOwner",
            &(name,),
        ))
        .unwrap()
        .body()
        .deserialize::<bool>()
        .unwrap();
        if owned {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let status = zbus::block_on(connection.call_method(
        Some(name),
        PATH,
        Some(name),
        "Request",
        &("{\"action\":\"status\"}",),
    ))
    .unwrap();
    let raw: String = status.body().deserialize().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&raw).unwrap()["result"]["available"],
        true
    );
    assert_eq!(unsafe { libc::kill(child.0.id() as i32, libc::SIGTERM) }, 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    use std::io::Read;
    let mut out = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["ok"], true);
    let mut err = String::new();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    assert!(err.is_empty(), "{err}");
    zbus::block_on(connection.close()).unwrap();
}
