//! Real service replacement on a private D-Bus and private state, without apt,
//! credentials, a mounted cloud filesystem or changes to the user's services.
use serde_json::{json, Value};
use std::{path::PathBuf, process::Command};
fn call(mut request: Value) -> Value {
    request["version"] = json!(1);
    let response = neutrondrive_core::runtime_request(request.to_string().as_bytes(), &mut |_| 0);
    assert_eq!(response["ok"], true, "{response}");
    response["result"].clone()
}
fn owner() -> String {
    zbus::block_on(async {
        let connection = zbus::Connection::session().await.unwrap();
        connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetNameOwner",
                &("io.github.SigmaCodez.NeutronDrive.Service",),
            )
            .await
            .unwrap()
            .body()
            .deserialize::<String>()
            .unwrap()
    })
}
#[test]
fn services_use_new_processes_after_update() {
    let Ok(binary) = std::env::var("NEUTRONDRIVE_UPDATE_TEST_BINARY") else {
        return;
    };
    let root = std::env::temp_dir().join(format!("nd-update-handoff-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let result = Command::new("/usr/bin/dbus-run-session")
        .arg(format!(
            "--config-file={}/../desktop/tests/session.conf",
            env!("CARGO_MANIFEST_DIR")
        ))
        .args([
            "--",
            std::env::current_exe().unwrap().to_str().unwrap(),
            "--exact",
            "handoff_child",
            "--nocapture",
        ])
        .env("ND_UPDATE_HANDOFF_CHILD", &binary)
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env_remove("SNAP")
        .env_remove("FLATPAK_ID")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env("QT_QPA_PLATFORM", "invalid-platform")
        .output()
        .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
fn handoff_child() {
    let Ok(binary) = std::env::var("ND_UPDATE_HANDOFF_CHILD") else {
        return;
    };
    let data = call(json!({"op":"data_dir"}));
    let state = PathBuf::from(data.as_str().unwrap()).join("background");
    let service = |action: &str| json!({"op":"service_controller","state":state,"operation":{"action":action}});
    let stop_drive = || call(json!({"op":"desktop_bus","operation":{"action":"stop_for_update"}}));
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for request in [
                json!({"version":1,"op":"service_controller","state":self.0,"operation":{"action":"stop"}}),
                json!({"version":1,"op":"desktop_bus","operation":{"action":"stop_for_update"}}),
            ] {
                let _ =
                    neutrondrive_core::runtime_request(request.to_string().as_bytes(), &mut |_| 0);
            }
        }
    }
    let _cleanup = Cleanup(state.clone());
    // Stopping an absent service must never activate it.
    assert_eq!(stop_drive()["running"], false);
    let plan = json!({"app":binary,"cli":"/missing-synthetic-sdk","sync":true,"drive":true});
    let restore = || {
        assert!(Command::new(&binary)
            .args(["--restore-update-services", &plan.to_string()])
            .status()
            .unwrap()
            .success())
    };
    restore();
    let first = call(service("status"));
    assert_eq!(first["running"], true);
    let first_owner = owner();
    call(service("stop_idle"));
    assert_eq!(call(service("status"))["running"], false);
    assert_eq!(stop_drive()["running"], true);
    let missing = call(
        json!({"op":"desktop_bus","operation":{"action":"request","request":{"action":"status"}}}),
    );
    assert_eq!(missing["available"], false);
    restore();
    let second = call(service("status"));
    assert_eq!(second["running"], true);
    assert_ne!(first["pid"], second["pid"]);
    assert_ne!(first_owner, owner());
    assert_eq!(second["jobs"], first["jobs"]);
    println!("NATIVE_UPDATE_SERVICE_REPLACEMENT_OK");
}
