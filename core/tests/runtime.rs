//! Run the compiled Rust entry point, without importing the Python application.
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
fn request(value: Value) -> (bool, Value) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let state = std::env::temp_dir().join(format!(
        "nd-runtime-state-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&state).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_neutrondrive-runtime"))
        .env("XDG_STATE_HOME", &state)
        .env("XDG_CONFIG_HOME", state.join("config"))
        .env("XDG_CACHE_HOME", state.join("cache"))
        .env("XDG_DATA_HOME", state.join("data"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(value.to_string().as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    std::fs::remove_dir_all(&state).unwrap();
    assert!(result.stderr.is_empty());
    (
        result.status.success(),
        serde_json::from_slice(&result.stdout).unwrap(),
    )
}
#[test]
fn native_entry_point_validates_requests() {
    let (ok, reply) = request(json!({"version":1,"op":"validate_remote","path":"/my-files"}));
    assert!(ok);
    assert_eq!(reply["result"], "/my-files");
    for value in [
        json!({}),
        json!({"version":2,"op":"validate_remote","path":"/my-files"}),
        json!({"version":1,"op":"drive","executable":"/bin/echo","operation":{"action":"trash","paths":[]}}),
    ] {
        let (ok, reply) = request(value);
        assert!(!ok);
        assert_eq!(reply["ok"], false);
    }
}
#[test]
fn version_requires_no_sdk_or_python() {
    let result = Command::new(env!("CARGO_BIN_EXE_neutrondrive-runtime"))
        .arg("--version")
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8(result.stdout)
        .unwrap()
        .starts_with("NeutronDrive runtime "));
}

#[test]
fn sigterm_cancels_and_reaps_the_sdk_process() {
    let root = std::env::temp_dir().join(format!("nd-native-cancel-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let marker = root.join("sdk-pid");
    let mut child = Command::new(env!("CARGO_BIN_EXE_neutrondrive-runtime"))
        .env("XDG_STATE_HOME", root.join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let value = json!({"version":1,"executable":"/bin/sh","args":["-c","printf '%s' \"$$\" > \"$1\"; exec sleep 30","sdk",marker],"timeout":60.,"parse":false,"login":false,"max_output":4096});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(value.to_string().as_bytes())
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !marker.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if !marker.exists() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("SDK fixture did not start");
    }
    let sdk: i32 = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
    // SAFETY: the child is alive, owned and unreaped; SIGTERM has no pointer inputs.
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["cancelled"], true);
    assert_eq!(unsafe { libc::kill(sdk, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn remote_scan_runs_with_native_sdk_transport_without_python() {
    use std::os::unix::fs::PermissionsExt;
    let folder = std::env::temp_dir().join(format!("nd-native-remote-{}", std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    let sdk = folder.join("sdk");
    std::fs::write(&sdk,"#!/bin/sh\nprintf '%s\\n' '[{\"uid\":\"file-1\",\"name\":{\"ok\":true,\"value\":\"test.txt\"},\"type\":\"file\",\"activeRevision\":{\"uid\":\"rev-1\",\"claimedSize\":3,\"claimedDigests\":{\"sha1\":\"0000000000000000000000000000000000000000\",\"sha1Verified\":true}}}]'\n").unwrap();
    std::fs::set_permissions(&sdk, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (ok, reply) = request(
        json!({"version":1,"op":"sync_remote","executable":sdk,"operation":{"action":"scan","root":{"uid":"root","path":"/my-files"},"fresh":true,"allow_trashed":false}}),
    );
    std::fs::remove_dir_all(folder).unwrap();
    assert!(ok, "{reply}");
    assert_eq!(reply["result"]["entries"]["test.txt"]["uid"], "file-1");
    assert_eq!(reply["result"]["blocked"], json!({}));
}

#[test]
fn sync_preview_uses_native_scans_and_baseline_without_python() {
    use std::os::unix::fs::PermissionsExt;
    let folder = std::env::temp_dir().join(format!("nd-native-plan-{}", std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    let local = folder.join("local");
    std::fs::create_dir(&local).unwrap();
    let sdk = folder.join("sdk");
    std::fs::write(&sdk,r#"#!/bin/sh
if [ "$2" = "info" ]; then
  printf '%s\n' '{"uid":"root","name":{"ok":true,"value":"root"},"type":"folder"}'
else
  printf '%s\n' '[{"uid":"file-1","name":{"ok":true,"value":"test.txt"},"type":"file","activeRevision":{"uid":"rev-1","claimedSize":3,"claimedDigests":{"sha1":"0000000000000000000000000000000000000000","sha1Verified":true}}}]'
fi
"#).unwrap();
    std::fs::set_permissions(&sdk, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (ok, reply) = request(
        json!({"version":1,"op":"sync_plan","executable":sdk,"state":folder.join("state"),"root":local,"remote_path":"/my-files"}),
    );
    assert!(ok, "{reply}");
    assert_eq!(reply["result"]["actions"][0]["kind"], "download");
    assert_eq!(reply["result"]["actions"][0]["relative_path"], "test.txt");
    assert_eq!(std::fs::read_dir(local).unwrap().count(), 0);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn sync_download_execution_and_rescan_need_no_python() {
    use std::os::unix::fs::PermissionsExt;
    let folder = std::env::temp_dir().join(format!("nd-native-execute-{}", std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    let local = folder.join("local");
    std::fs::create_dir(&local).unwrap();
    let sdk = folder.join("sdk");
    std::fs::write(&sdk,r#"#!/bin/sh
case "$2" in
info) printf '%s\n' '{"uid":"root","name":{"ok":true,"value":"root"},"type":"folder"}' ;;
download) for destination do :; done; printf abc > "$destination/test.txt"; printf '%s\n' '{}' ;;
*) printf '%s\n' '[{"uid":"file-1","name":{"ok":true,"value":"test.txt"},"type":"file","activeRevision":{"uid":"rev-1","claimedSize":3,"claimedDigests":{"sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","sha1Verified":true}}}]' ;;
esac
"#).unwrap();
    std::fs::set_permissions(&sdk, std::fs::Permissions::from_mode(0o700)).unwrap();
    let preview = json!({"version":1,"op":"sync_plan","executable":sdk,"state":folder.join("state"),"root":local,"remote_path":"/my-files"});
    let (ok, plan) = request(preview.clone());
    assert!(ok, "{plan}");
    let (ok, result) = request(
        json!({"version":1,"op":"sync_execute","executable":sdk,"state":folder.join("state"),"plan":plan["result"],"trash_state":folder.join("trash-state"),"trash_data":folder.join("xdg"),"trash_desktop":true}),
    );
    assert!(ok, "{result}");
    assert_eq!(result["result"]["errors"], json!([]));
    assert_eq!(result["result"]["conflicts"], json!([]));
    assert_eq!(std::fs::read(local.join("test.txt")).unwrap(), b"abc");
    let (ok, plan) = request(preview);
    assert!(ok, "{plan}");
    assert_eq!(plan["result"]["actions"], json!([]));
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn native_daemon_download_pause_resume_and_stop_need_no_python() {
    use std::{
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root =
        Cleanup(std::env::temp_dir().join(format!("nd-native-daemon-{}", std::process::id())));
    std::fs::create_dir(&root.0).unwrap();
    let local = root.0.join("local");
    std::fs::create_dir(&local).unwrap();
    let state = root.0.join("daemon");
    let sdk = root.0.join("sdk");
    std::fs::write(&sdk,r#"#!/bin/sh
if [ "$1" = "events" ]; then
  printf '%s\n' '{"protocol":1,"type":"batch","roots":[{"path":"/my-files","uid":"root","scopeId":"scope"}],"events":[],"cursors":{"scope":"next"},"resets":[]}'
  exit 0
fi
case "$2" in
info) printf '%s\n' '{"uid":"root","name":{"ok":true,"value":"root"},"type":"folder"}' ;;
download) for destination do :; done; printf abc > "$destination/test.txt"; printf '%s\n' '{}' ;;
upload) exit 1 ;;
*) printf '%s\n' '[{"uid":"file-1","name":{"ok":true,"value":"test.txt"},"type":"file","activeRevision":{"uid":"rev-1","claimedSize":3,"claimedDigests":{"sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","sha1Verified":true}}}]' ;;
esac
"#).unwrap();
    std::fs::set_permissions(&sdk, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (ok, job) = request(
        json!({"version":1,"op":"jobs","state":state,"planned":[],"operation":{"action":"add","local_root":local,"remote_root":"/my-files","name":"native","paused":false}}),
    );
    assert!(ok, "{job}");
    let mut child = Child(
        Command::new(env!("CARGO_BIN_EXE_neutrondrive-runtime"))
            .env("PATH", "/nonexistent")
            .env("XDG_STATE_HOME", root.0.join("state"))
            .env("XDG_DATA_HOME", root.0.join("data"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    child
        .0
        .stdin
        .take()
        .unwrap()
        .write_all(
            json!({"version":1,"op":"daemon","config":{"state":state,"executable":sdk}})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let control = |action: &str| {
        request(
            json!({"version":1,"op":"control_command","path":state.join("control.sock"),"action":action,"job_id":null,"timeout":1.0}),
        )
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (ok, status) = control("status");
        if ok
            && status["result"]["jobs"][0]["state"] == "idle"
            && status["result"]["jobs"][0]["pending"] == false
            && status["result"]["event_source"]["state"] == "connected"
            && std::fs::read_dir(state.join("events"))
                .ok()
                .is_some_and(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .any(|entry| entry.file_name().to_string_lossy().ends_with(".json"))
                })
        {
            break;
        }
        assert!(Instant::now() < deadline, "daemon did not sync: {status}");
        assert!(child.0.try_wait().unwrap().is_none(), "daemon exited early");
        std::thread::sleep(Duration::from_millis(30));
    }
    assert_eq!(std::fs::read(local.join("test.txt")).unwrap(), b"abc");
    let event_file = std::fs::read_dir(state.join("events"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let cursor: Value = serde_json::from_slice(&std::fs::read(event_file).unwrap()).unwrap();
    assert_eq!(cursor["cursors"]["scope"], "next");
    std::fs::write(local.join("test.txt"), b"local edit").unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let (ok, status) = control("status");
        if ok && status["result"]["jobs"][0]["state"] == "offline" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "inotify did not schedule edited file: {status}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        std::fs::read(local.join("test.txt")).unwrap(),
        b"local edit"
    );
    let (ok, paused) = control("pause");
    assert!(ok, "{paused}");
    assert_eq!(paused["result"]["jobs"][0]["state"], "paused");
    let (ok, resumed) = control("resume");
    assert!(ok, "{resumed}");
    assert_eq!(resumed["result"]["jobs"][0]["account_root_uid"], "root");
    let (ok, stopped) = control("stop");
    assert!(ok, "{stopped}");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "daemon failed to stop");
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(!state.join("control.sock").exists());
    assert!(!state.join("daemon.pid").exists());
}

#[test]
fn native_daemon_sigterm_cancels_an_inflight_event_sdk() {
    use std::{
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };
    let root = std::env::temp_dir().join(format!("nd-daemon-signal-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let sdk = root.join("sdk");
    let pidfile = root.join("sdk.pid");
    std::fs::write(
        &sdk,
        "#!/bin/sh\necho $$ > \"$SDK_PID_FILE\"\nexec /bin/sleep 60\n",
    )
    .unwrap();
    std::fs::set_permissions(&sdk, std::fs::Permissions::from_mode(0o700)).unwrap();
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(
        Command::new(env!("CARGO_BIN_EXE_neutrondrive-runtime"))
            .env("PATH", "/nonexistent")
            .env("SDK_PID_FILE", &pidfile)
            .env("XDG_STATE_HOME", root.join("private"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    child.0.stdin.take().unwrap().write_all(json!({"version":1,"op":"daemon","config":{"state":root.join("daemon"),"executable":sdk}}).to_string().as_bytes()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !pidfile.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let sdk_pid: i32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    unsafe {
        libc::kill(child.0.id() as i32, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "event SDK did not cancel");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(unsafe { libc::kill(sdk_pid, 0) }, -1);
    assert!(!root.join("daemon/control.sock").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cache_hydration_uses_native_sdk_and_verified_install_without_python() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("nd-native-hydrate-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let objects = root.join("objects");
    let staging = root.join("staging");
    for path in [&objects, &staging] {
        std::fs::create_dir(path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let sdk = root.join("sdk");
    std::fs::write(&sdk,r#"#!/bin/sh
for last do :; done
if [ "$2" = "download" ]; then printf abc > "$last/data"; printf '%s\n' '{}'; exit 0; fi
if [ "$last" = "/my-files" ]; then printf '%s\n' '{"uid":"root","name":{"ok":true,"value":"root"},"type":"folder"}'; exit 0; fi
printf '%s\n' '{"uid":"file","name":{"ok":true,"value":"data"},"type":"file","activeRevision":{"uid":"r1","claimedSize":3,"claimedDigests":{"sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","sha1Verified":true}}}'
"#).unwrap();
    std::fs::set_permissions(&sdk, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (ok, result) = request(
        json!({"version":1,"op":"ondemand_io","executable":sdk,"operation":{"action":"hydrate","objects":objects,"staging":staging,"node":{"uid":"file","name":"data","path":"/my-files/file","revision":"r1","sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","size":3},"offline":false,"remote_root":"/my-files","root_uid":"root"}}),
    );
    assert!(ok, "{result}");
    assert!(result["result"]["value"].is_number(), "{result}");
    let files = std::fs::read_dir(&objects)
        .unwrap()
        .collect::<std::io::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(std::fs::read(files[0].path()).unwrap(), b"abc");
    assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_snapshot_roundtrip_through_native_protocol() {
    let dir = std::env::temp_dir().join(format!("nd-native-tree-{}", std::process::id()));
    let (ok, reply) = request(
        json!({"version":1,"op":"ondemand_io","operation":{"action":"private_directory","path":dir}}),
    );
    assert!(ok, "{reply}");
    let path = dir.join("metadata.json");
    let root = json!({"inode":1,"parent":1,"node":{"uid":"root","name":"My files","type":"folder","path":"/my-files/root"},"local":false});
    let (ok, reply) = request(
        json!({"version":1,"op":"ondemand_tree","operation":{"action":"save","path":path,"remote_root":"/my-files","tree":{"entries":[root],"children":{"1":{}},"pinned":[],"listed":[1],"next_inode":2}}}),
    );
    assert!(ok, "{reply}");
    assert_eq!(reply["result"], json!({"value":null}));
    let (ok, reply) = request(
        json!({"version":1,"op":"ondemand_tree","operation":{"action":"load","path":path,"remote_root":"/my-files"}}),
    );
    assert!(ok, "{reply}");
    assert_eq!(reply["result"]["value"]["listed"], json!([1]));
    assert_eq!(reply["result"]["value"]["children"], json!({"1":{}}));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn owned_mount_lifecycle_uses_native_sdk_without_python() {
    use std::os::unix::fs::PermissionsExt;
    if let Some(root) = std::env::var_os("ND_MOUNT_NATIVE_WORKER") {
        let root = std::path::PathBuf::from(root);
        let call = |operation: Value, offline: bool| {
            let response=neutrondrive_core::runtime_request(json!({"version":1,"op":"ondemand_fs","executable":if offline{root.join("absent-sdk")}else{root.join("sdk")},"operation":operation}).to_string().as_bytes(),&mut |_|0);
            assert_eq!(response["ok"], true, "{response}");
            let value = &response["result"];
            assert!(
                value.get("errno").is_none() && value.get("backend_error").is_none(),
                "{response}"
            );
            value["value"].clone()
        };
        let create = |offline: bool| json!({"action":"create","config":{"cache":root.join("cache"),"remote_root":"/my-files","writable":false,"offline":offline,"metadata_ttl":15.0}});
        let mount = call(create(false), false);
        let files = call(
            json!({"action":"call","id":mount,"operation":{"action":"listdir","inode":1,"refresh":true}}),
            false,
        );
        let inode = files[0]["inode"].clone();
        call(
            json!({"action":"call","id":mount,"operation":{"action":"pin","inode":inode,"pinned":true}}),
            false,
        );
        let handle = call(
            json!({"action":"call","id":mount,"operation":{"action":"open","inode":inode,"flags":0}}),
            false,
        );
        assert_eq!(
            call(
                json!({"action":"call","id":mount,"operation":{"action":"read","handle":handle,"offset":0,"size":64}}),
                false
            ),
            "616263"
        );
        call(
            json!({"action":"call","id":mount,"operation":{"action":"release","handle":handle}}),
            false,
        );
        call(json!({"action":"close","id":mount}), false);
        let mount = call(create(true), true);
        let handle = call(
            json!({"action":"call","id":mount,"operation":{"action":"open","inode":inode,"flags":0}}),
            true,
        );
        assert_eq!(
            call(
                json!({"action":"call","id":mount,"operation":{"action":"read","handle":handle,"offset":0,"size":64}}),
                true
            ),
            "616263"
        );
        call(json!({"action":"close","id":mount}), true);
        if std::env::var("NEUTRONDRIVE_FUSE_TEST").as_deref() == Ok("1") {
            let request = |operation: Value| {
                let response=neutrondrive_core::runtime_request(json!({"version":1,"op":"mount_manager","operation":operation,"executable":root.join("sdk")}).to_string().as_bytes(),&mut |event| {
                    assert_ne!(event["kind"],"host_request","Native mount requested Python services");0
                });
                assert_eq!(response["ok"], true, "{response}");
                let v = &response["result"];
                assert!(
                    v.get("errno").is_none() && v.get("backend_error").is_none(),
                    "{response}"
                );
                v["value"].clone()
            };
            let id = request(json!({"action":"create","state_dir":root.join("manager-state")}));
            request(
                json!({"action":"prepare","id":id,"mountpoint":root.join("kernel"),"remote_root":"/my-files","writable":false,"offline":false}),
            );
            let worker_id = id.clone();
            let sdk = root.join("sdk");
            let worker = std::thread::spawn(move || {
                let response=neutrondrive_core::runtime_request(json!({"version":1,"op":"mount_manager","operation":{"action":"run","id":worker_id},"executable":sdk}).to_string().as_bytes(),&mut |event|{assert_ne!(event["kind"],"host_request");0});
                assert_eq!(response["ok"], true, "{response}");
                assert!(response["result"].get("errno").is_none(), "{response}");
            });
            request(json!({"action":"wait","id":id}));
            let read = std::fs::read(root.join("kernel/data"));
            // Unmount before asserting so a failed check cannot orphan a test mount.
            request(json!({"action":"unmount","id":id}));
            worker.join().unwrap();
            assert_eq!(read.unwrap(), b"abc");
            request(json!({"action":"forget","id":id}));
            let bookmarks = std::fs::read_to_string(root.join("config/gtk-3.0/bookmarks")).unwrap();
            assert_eq!(
                bookmarks,
                format!("file://{} Proton Drive\n", root.join("kernel").display())
            );
        }
        return;
    }
    let root = std::env::temp_dir().join(format!("nd-native-owned-mount-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let sdk = root.join("sdk");
    std::fs::write(&sdk,r#"#!/bin/sh
for last do :; done
file='{"uid":"data","name":{"ok":true,"value":"data"},"type":"file","activeRevision":{"uid":"r1","claimedSize":3,"claimedDigests":{"sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","sha1Verified":true}}}'
if [ "$2" = "download" ]; then printf abc > "$last/data"; printf '%s\n' '{}'; exit 0; fi
if [ "$2" = "list" ]; then printf '[%s]\n' "$file"; exit 0; fi
if [ "$last" = "/my-files" ]; then printf '%s\n' '{"uid":"root","name":{"ok":true,"value":"root"},"type":"folder"}'; exit 0; fi
printf '%s\n' "$file"
"#).unwrap();
    std::fs::set_permissions(&sdk, std::fs::Permissions::from_mode(0o700)).unwrap();
    let helpers = root.join("helpers");
    std::fs::create_dir(&helpers).unwrap();
    if std::env::var("NEUTRONDRIVE_FUSE_TEST").as_deref() == Ok("1") {
        std::os::unix::fs::symlink("/usr/bin/fusermount3", helpers.join("fusermount3")).unwrap();
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "owned_mount_lifecycle_uses_native_sdk_without_python",
            "--nocapture",
        ])
        .env("ND_MOUNT_NATIVE_WORKER", &root)
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("PATH", &helpers)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn desktop_event_source_owns_sdk_cursor_and_wait_without_python() {
    use std::os::unix::fs::PermissionsExt;
    if let Some(root) = std::env::var_os("ND_EVENT_SOURCE_WORKER") {
        let root = std::path::PathBuf::from(root);
        let request = |operation: Value| {
            let r=neutrondrive_core::runtime_request(json!({"version":1,"op":"event_source","operation":operation,"executable":root.join("sdk")}).to_string().as_bytes(),&mut |_|0);
            assert_eq!(r["ok"], true, "{r}");
            r["result"].clone()
        };
        let id = request(
            json!({"action":"create","state_dir":root.join("events"),"roots":["/my-files"],"interval":30.0,"dynamic_roots":false}),
        );
        let initial = request(json!({"action":"snapshot","id":id}));
        let file = std::path::PathBuf::from(initial["file"].as_str().unwrap());
        request(json!({"action":"start","id":id}));
        let mut events = 0;
        let r=neutrondrive_core::runtime_request(json!({"version":1,"op":"event_source","operation":{"action":"run","id":id},"executable":root.join("sdk")}).to_string().as_bytes(),&mut |event| {
            assert_ne!(event["kind"],"host_request","Native events unexpectedly requested a Python service");
            if event["kind"]=="remote_event"{events+=1;assert!(!file.exists());}
            let ready=request(json!({"action":"snapshot","id":id}))["status"]["state"]=="ready";
            i32::from(event["kind"]=="poll"&&ready)
        });
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(events, 2);
        let stored: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        assert_eq!(stored["cursors"], json!({"scope":"next"}));
        assert_eq!(
            request(json!({"action":"snapshot","id":id}))["running"],
            false
        );
        request(json!({"action":"forget","id":id}));
        return;
    }
    let root = std::env::temp_dir().join(format!("nd-native-event-source-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let sdk = root.join("sdk");
    std::fs::write(&sdk,"#!/bin/sh\nprintf '%s\\n' '{\"protocol\":1,\"type\":\"batch\",\"roots\":[{\"path\":\"/my-files\",\"uid\":\"root\",\"scopeId\":\"scope\"}],\"events\":[{\"type\":\"node_updated\",\"treeEventScopeId\":\"scope\",\"eventId\":\"next\",\"nodeUid\":\"file\"}],\"cursors\":{\"scope\":\"next\"},\"resets\":[]}'\n").unwrap();
    std::fs::set_permissions(&sdk, std::fs::Permissions::from_mode(0o700)).unwrap();
    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "desktop_event_source_owns_sdk_cursor_and_wait_without_python",
            "--nocapture",
        ])
        .env("ND_EVENT_SOURCE_WORKER", &root)
        .env("XDG_STATE_HOME", root.join("state"))
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn update_verification_runs_without_python_or_external_hash_commands() {
    use std::os::unix::fs::PermissionsExt;
    let folder = std::env::temp_dir().join(format!("nd-native-update-{}", std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = folder.join("package.deb");
    std::fs::write(&path, b"package").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_neutrondrive-runtime"))
        .env("PATH", "/nonexistent")
        .env("XDG_STATE_HOME", folder.join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use sha2::{Digest, Sha256};
    let value = json!({"version":1,"op":"update_runtime","operation":{"action":"verify","path":path,"release":{
        "tag":"v1.0.0","url":"https://github.com/SigmaCodez/NeutronDrive/releases/tag/v1.0.0",
        "asset_url":"https://github.com/SigmaCodez/NeutronDrive/releases/download/v1.0.0/package.deb",
        "digest":format!("{:x}",Sha256::digest(b"package")),"size":7}}});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(value.to_string().as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    let reply: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(reply["result"], json!({"value":null}));
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn native_autostart_respects_package_boundaries_without_python() {
    use std::path::Path;
    let folder = std::env::temp_dir().join(format!("nd-native-autostart-{}", std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    let invoke = |operation: Value, extra: &[(&str, &str)]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_neutrondrive-runtime"));
        command
            .env_clear()
            .env("PATH", "/nonexistent")
            .env("XDG_CONFIG_HOME", &folder)
            .env("XDG_STATE_HOME", folder.join("state"));
        for (key, value) in extra {
            command.env(key, value);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                json!({"version":1,"op":"autostart","operation":operation})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.stderr.is_empty());
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let path = invoke(json!({"action":"path"}), &[])["result"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(Path::new(&path).starts_with(&folder));
    assert_eq!(
        invoke(
            json!({"action":"set","path":path,"enabled":true,"command":["/usr/bin/neutrondrive","--background"]}),
            &[]
        )["ok"],
        true
    );
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("X-NeutronDrive-Managed=true"));
    assert_eq!(
        invoke(
            json!({"action":"set","path":path,"enabled":false,"command":[]}),
            &[]
        )["ok"],
        true
    );
    assert!(!Path::new(&path).exists());
    let operation = json!({"action":"command","arguments":["--background"],"source_launcher":"/absent","interpreter":"/absent"});
    assert_eq!(
        invoke(operation.clone(), &[("SNAP", "/snap/app")])["result"],
        json!(["neutrondrive", "--background"])
    );
    assert_eq!(
        invoke(
            operation,
            &[("FLATPAK_ID", "io.github.SigmaCodez.NeutronDrive")]
        )["result"],
        json!([
            "flatpak",
            "run",
            "io.github.SigmaCodez.NeutronDrive",
            "--background"
        ])
    );
    assert_eq!(
        invoke(
            json!({"action":"set","path":path,"enabled":true,"command":["anything"]}),
            &[("FLATPAK_ID", "app")]
        )["ok"],
        false
    );
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn link_settings_use_private_stdin_and_never_process_arguments() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("nd-private-link-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let sdk = root.join("sdk");
    std::fs::write(&sdk, "#!/bin/sh\n[ \"$#\" = 5 ] && [ \"$1\" = sharing ] && [ \"$2\" = configure ] && [ \"$3\" = --json ] && [ \"$4\" = -- ] && [ \"$5\" = /my-files/test ] || exit 7\nexec /bin/cat\n").unwrap();
    std::fs::set_permissions(&sdk, std::fs::Permissions::from_mode(0o700)).unwrap();
    let settings = json!({"password":"not-in-argv-ä","role":"viewer"});
    let (ok, reply) = request(
        json!({"version":1,"op":"drive","executable":sdk,"operation":{"action":"set_link","path":"/my-files/test","settings":settings}}),
    );
    assert!(ok, "{reply}");
    assert_eq!(reply["result"], settings);
    let (ok, _) = request(
        json!({"version":1,"op":"drive","executable":"/nonexistent","operation":{"action":"set_link","path":"/my-files/test","settings":{"password":"x".repeat(4096)}}}),
    );
    assert!(!ok);
    std::fs::remove_dir_all(root).unwrap();
}
