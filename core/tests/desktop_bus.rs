//! Real isolated session-bus fixtures; no account, installed service or Python required.
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use zbus::{Connection, OwnedObjectPath, OwnedVariant};
const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
struct Background {
    spoof: bool,
    silent: bool,
}
#[zbus::interface(name = "org.freedesktop.portal.Background")]
impl Background {
    async fn request_background(
        &self,
        parent: &str,
        options: HashMap<String, OwnedVariant>,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        assert_eq!(parent, "");
        assert!(bool::try_from(options.get("autostart").unwrap()).unwrap());
        assert!(
            String::try_from(options.get("handle_token").unwrap().clone())
                .unwrap()
                .starts_with("neutrondrive_")
        );
        assert_eq!(
            Vec::<String>::try_from(options.get("commandline").unwrap().clone()).unwrap(),
            vec!["neutrondrive", "--background"]
        );
        let path = "/org/freedesktop/portal/desktop/request/fixture/changed_path";
        if self.silent {
            return Ok(OwnedObjectPath::try_from(path).unwrap());
        }
        let values = HashMap::from([
            ("background", OwnedVariant::from(true)),
            ("autostart", OwnedVariant::from(true)),
        ]);
        if self.spoof {
            let other = Connection::session().await.unwrap();
            other
                .emit_signal(
                    header.sender().map(|v| v.as_str()),
                    path,
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(0u32, &values),
                )
                .await
                .unwrap();
        }
        connection
            .emit_signal(
                None::<&str>,
                "/org/freedesktop/portal/desktop/request/fixture/unrelated",
                "org.freedesktop.portal.Request",
                "Response",
                &(1u32, &values),
            )
            .await
            .unwrap();
        connection
            .emit_signal(
                None::<&str>,
                path,
                "org.freedesktop.portal.Request",
                "Response",
                &(u32::from(self.spoof), &values),
            )
            .await
            .unwrap();
        Ok(OwnedObjectPath::try_from(path).unwrap())
    }
}
struct Settings {
    change_first: bool,
}
struct OpenUri {
    mode: String,
}
#[zbus::interface(name = "org.freedesktop.portal.OpenURI")]
impl OpenUri {
    #[zbus(name = "OpenURI")]
    async fn open_uri(
        &self,
        parent: &str,
        uri: &str,
        options: HashMap<String, OwnedVariant>,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        assert_eq!(parent, "");
        assert_eq!(
            uri,
            "https://account.proton.me/auth-desktop?test=fixture#fragment"
        );
        assert!(
            String::try_from(options.get("handle_token").unwrap().clone())
                .unwrap()
                .starts_with("neutrondrive_")
        );
        let path = "/org/freedesktop/portal/desktop/request/fixture/browser";
        if self.mode == "uri-invalid-path" {
            return Ok(OwnedObjectPath::try_from("/invalid").unwrap());
        }
        if self.mode != "uri-cancel" {
            let values: HashMap<String, OwnedVariant> = HashMap::new();
            if self.mode == "uri-spoof" {
                let other = Connection::session().await.unwrap();
                other
                    .emit_signal(
                        header.sender().map(|v| v.as_str()),
                        path,
                        "org.freedesktop.portal.Request",
                        "Response",
                        &(0u32, &values),
                    )
                    .await
                    .unwrap();
            }
            connection
                .emit_signal(
                    None::<&str>,
                    "/org/freedesktop/portal/desktop/request/fixture/unrelated",
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(1u32, &values),
                )
                .await
                .unwrap();
            connection
                .emit_signal(
                    None::<&str>,
                    path,
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(u32::from(self.mode != "uri"), &values),
                )
                .await
                .unwrap();
        }
        Ok(OwnedObjectPath::try_from(path).unwrap())
    }
}
#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Settings {
    async fn read(
        &self,
        namespace: &str,
        key: &str,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<OwnedVariant> {
        assert_eq!(namespace, "org.freedesktop.appearance");
        assert_eq!(key, "color-scheme");
        if self.change_first {
            connection
                .emit_signal(
                    None::<&str>,
                    PATH,
                    "org.freedesktop.portal.Settings",
                    "SettingChanged",
                    &(namespace, key, OwnedVariant::from(2u32)),
                )
                .await
                .unwrap();
        }
        Ok(
            OwnedVariant::try_from(zbus::Variant::Variant(Box::new(zbus::Variant::U32(1))))
                .unwrap(),
        )
    }
}
struct Drive {
    calls: Arc<AtomicUsize>,
}
#[zbus::interface(name = "io.github.SigmaCodez.NeutronDrive.Service")]
impl Drive {
    fn request(&self, payload: &str) -> zbus::fdo::Result<String> {
        let v: Value = serde_json::from_str(payload).unwrap();
        if v["action"] == "status" {
            return Ok(json!({"ok":true,"result":{"available":true,"mounted":false}}).to_string());
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(zbus::fdo::Error::NoReply(
            "synthetic-private-diagnostic".into(),
        ))
    }
}
fn runtime(operation: Value, callback: &mut impl FnMut(Value) -> i32) -> Value {
    neutrondrive_core::runtime_request(operation.to_string().as_bytes(), callback)
}
#[test]
fn isolated_native_desktop_bus_and_portals() {
    if std::env::var("NEUTRONDRIVE_DBUS_TEST").as_deref() != Ok("1") {
        return;
    }
    for case in [
        "uri",
        "uri-spoof",
        "uri-denied",
        "uri-invalid-path",
        "uri-cancel",
        "background",
        "background-spoof",
        "background-cancel",
        "appearance",
        "appearance-race",
        "client",
    ] {
        let result = Command::new("/usr/bin/dbus-run-session")
            .arg("--dbus-daemon=/usr/bin/dbus-daemon")
            .arg("--")
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", "isolated_child", "--nocapture"])
            .env("ND_DESKTOP_BUS_CASE", case)
            .env("PATH", "/nonexistent")
            .env_remove("SNAP")
            .env_remove("FLATPAK_ID")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{case}: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
#[test]
fn isolated_child() {
    let Ok(case) = std::env::var("ND_DESKTOP_BUS_CASE") else {
        return;
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let connection = zbus::block_on(async {
        if case == "client" {
            zbus::connection::Builder::session()
                .name("io.github.SigmaCodez.NeutronDrive.Service")
                .serve_at(
                    "/io/github/SigmaCodez/NeutronDrive/Service",
                    Drive {
                        calls: calls.clone(),
                    },
                )
                .build()
                .await
                .unwrap()
        } else {
            zbus::connection::Builder::session()
                .name(PORTAL)
                .serve_at(PATH, OpenUri { mode: case.clone() })
                .serve_at(
                    PATH,
                    Background {
                        spoof: case == "background-spoof",
                        silent: case == "background-cancel",
                    },
                )
                .serve_at(
                    PATH,
                    Settings {
                        change_first: case == "appearance-race",
                    },
                )
                .build()
                .await
                .unwrap()
        }
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    if case.starts_with("uri") {
        let deadline = if case == "uri-cancel" {
            std::time::Instant::now() + std::time::Duration::from_millis(200)
        } else {
            deadline
        };
        let result = runtime(
            json!({"version":1,"op":"portal","operation":{"action":"open_uri","uri":"https://account.proton.me/auth-desktop?test=fixture#fragment"}}),
            &mut |_| i32::from(std::time::Instant::now() >= deadline),
        );
        assert_eq!(result["ok"], case == "uri", "{result}");
        if case == "uri-cancel" {
            assert_eq!(result["cancelled"], true);
        }
    } else if case.starts_with("background") {
        let deadline = if case == "background-cancel" {
            std::time::Instant::now() + std::time::Duration::from_millis(200)
        } else {
            deadline
        };
        let result = runtime(
            json!({"version":1,"op":"portal","operation":{"action":"background","enabled":true}}),
            &mut |_| i32::from(std::time::Instant::now() >= deadline),
        );
        if case == "background-cancel" {
            assert_eq!(result["cancelled"], true);
        } else {
            assert_eq!(result, json!({"ok":true,"result":case=="background"}));
        }
    } else if case.starts_with("appearance") {
        let mut values = Vec::new();
        let result = runtime(
            json!({"version":1,"op":"portal","operation":{"action":"appearance"}}),
            &mut |v| {
                if v["kind"] == "appearance_changed" {
                    values.push(v["scheme"].as_u64().unwrap());
                    1
                } else {
                    i32::from(std::time::Instant::now() >= deadline)
                }
            },
        );
        assert_eq!(result["cancelled"], true);
        assert_eq!(values, vec![if case == "appearance-race" { 2 } else { 1 }]);
    } else {
        let operation = |action| json!({"version":1,"op":"desktop_bus","operation":{"action":"request","request":{"action":action},"launcher":{"argv":["/nonexistent-marker"],"source":"/tmp"}}});
        assert_eq!(
            runtime(operation("status"), &mut |_| 0)["result"],
            json!({"available":true,"mounted":false})
        );
        let result = runtime(operation("mutate"), &mut |_| 0);
        assert_eq!(result["ok"], false);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let message = result["error"].as_str().unwrap();
        assert!(!message.contains("synthetic-private"));
        assert!(message.contains("unterbrochenen Aktion"));
    }
    zbus::block_on(connection.close()).unwrap();
}
