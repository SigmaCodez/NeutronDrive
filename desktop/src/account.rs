//! Ordered logout: stop synchronization and safely release native mounts first.
use crate::runtime::{self, Failure};
use serde_json::{json, Value};

pub fn logout(executable: &str, callback: &mut impl FnMut(Value) -> i32) -> Result<Value, Failure> {
    let flatpak = std::env::var_os("FLATPAK_ID").is_some_and(|v| !v.is_empty());
    logout_with(executable, flatpak, &mut |mut request| {
        if request["op"] == "desktop_bus" {
            request = crate::headless::desktop_request(
                executable,
                request["operation"]["request"].clone(),
            )?;
        }
        runtime::call(request, callback)
    })
}
fn logout_with(
    executable: &str,
    flatpak: bool,
    call: &mut impl FnMut(Value) -> Result<Value, Failure>,
) -> Result<Value, Failure> {
    let state = call(json!({"op":"data_dir"}))?;
    let root = state.as_str().ok_or_else(|| Failure {
        message: "Invalid application state directory".into(),
        cancelled: false,
    })?;
    call(
        json!({"op":"service_controller","state":std::path::Path::new(root).join("background"),"operation":{"action":"stop"}}),
    )?;
    if !flatpak {
        let status = call(
            json!({"op":"desktop_bus","operation":{"action":"request","request":{"action":"status"},"launcher":null}}),
        )?;
        if status["mounted"] == true {
            call(
                json!({"op":"desktop_bus","operation":{"action":"request","request":{"action":"unmount"},"launcher":null}}),
            )?;
        }
    }
    call(json!({"op":"drive","executable":executable,"operation":{"action":"logout"}}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logout_waits_for_sync_stop_and_unmount() {
        let mut requests = vec![];
        let result = logout_with("sdk", false, &mut |request| {
            requests.push(request.clone());
            Ok(match request["op"].as_str().unwrap() {
                "data_dir" => json!("/tmp/private"),
                "desktop_bus" => json!({"mounted":true}),
                _ => Value::Null,
            })
        });
        assert!(result.is_ok());
        assert_eq!(
            requests
                .iter()
                .map(|r| r["op"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "data_dir",
                "service_controller",
                "desktop_bus",
                "desktop_bus",
                "drive"
            ]
        );
        assert_eq!(requests[1]["operation"]["action"], "stop");
        assert_eq!(requests[3]["operation"]["request"]["action"], "unmount");
        assert_eq!(requests[4]["operation"]["action"], "logout");
    }
    #[test]
    fn any_cleanup_failure_prevents_logout_and_flatpak_keeps_its_mount() {
        for failed in 1..=3 {
            let mut step = 0;
            let result = logout_with("sdk", false, &mut |request| {
                assert_ne!(request["op"], "drive");
                let current = step;
                step += 1;
                if current == failed {
                    return Err(Failure {
                        message: "busy".into(),
                        cancelled: false,
                    });
                }
                Ok(if current == 0 {
                    json!("/tmp/private")
                } else {
                    json!({"mounted":true})
                })
            });
            assert!(result.is_err());
            assert_eq!(step, failed + 1);
        }
        let mut calls = 0;
        logout_with("sdk", true, &mut |request| {
            calls += 1;
            assert_ne!(request["op"], "desktop_bus");
            Ok(if calls == 1 {
                json!("/tmp/private")
            } else {
                Value::Null
            })
        })
        .unwrap();
        assert_eq!(calls, 3);
    }
}
