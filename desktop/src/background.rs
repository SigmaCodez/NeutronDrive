//! Native desktop orchestration of existing Rust jobs, computer setup and daemon IPC.
use crate::runtime::{self, Failure};
use serde_json::{json, Value};
use std::path::Path;
fn failure(message: &str) -> Failure {
    Failure {
        message: message.into(),
        cancelled: false,
    }
}
pub fn run(
    executable: &str,
    operation: &Value,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value, Failure> {
    let app = std::env::current_exe()
        .map_err(|_| failure("Native application executable unavailable"))?;
    let flatpak = std::env::var_os("FLATPAK_ID").is_some_and(|v| !v.is_empty());
    execute(executable, operation, &app, flatpak, &mut |request| {
        runtime::call(request, callback)
    })
}
fn execute(
    executable: &str,
    operation: &Value,
    app: &Path,
    flatpak: bool,
    call: &mut impl FnMut(Value) -> Result<Value, Failure>,
) -> Result<Value, Failure> {
    let action = operation["action"].as_str().unwrap_or("");
    if ![
        "status", "start", "stop", "remove", "pause", "resume", "sync", "add", "computer",
    ]
    .contains(&action)
    {
        return Err(failure("Unknown background action"));
    }
    let root = call(json!({"op":"data_dir"}))?;
    let root = Path::new(
        root.as_str()
            .ok_or_else(|| failure("Invalid application state directory"))?,
    );
    let state = root.join("background");
    let service =
        |operation: Value| json!({"op":"service_controller","state":state,"operation":operation});
    let mut argv = vec![
        json!(app),
        json!("--daemon"),
        json!("--state-dir"),
        json!(state),
    ];
    if !executable.is_empty() {
        argv.extend([json!("--cli"), json!(executable)]);
    }
    let start = service(json!({"action":"start","argv":argv,"environment":{}}));
    let status = service(json!({"action":"status"}));
    let command = |action: &str, id: Value| json!({"op":"control_command","path":state.join("control.sock"),"action":action,"job_id":id,"timeout":5.0});
    match action {
        "status" => return call(status),
        "start" => {
            call(start)?;
        }
        "stop" => {
            call(service(json!({"action":"stop"})))?;
        }
        "remove" => {
            let id = operation["id"]
                .as_str()
                .filter(|v| !v.is_empty())
                .ok_or_else(|| failure("Missing job identity"))?;
            if call(status.clone())?["running"] == true {
                call(command("pause", json!(id)))?;
            }
            call(
                json!({"op":"jobs","state":state,"planned":[],"operation":{"action":"remove","id":id}}),
            )?;
            if call(status.clone())?["running"] == true {
                call(command("reload", Value::Null))?;
            }
        }
        "pause" | "resume" | "sync" => {
            let id = operation["id"]
                .as_str()
                .filter(|v| !v.is_empty())
                .ok_or_else(|| failure("Missing job identity"))?;
            call(start)?;
            call(command(action, json!(id)))?;
        }
        "add" | "computer" => {
            let local = operation["local"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| failure("Missing local sync folder"))?;
            let local =
                std::path::absolute(local).map_err(|_| failure("Invalid local sync folder"))?;
            // No job/cloud mutation before the Flatpak desktop grants background operation.
            if flatpak
                && call(json!({"op":"portal","operation":{"action":"background","enabled":true}}))?
                    != true
            {
                return Err(failure("Der Desktop hat Hintergrundbetrieb nicht freigegeben. Es wurde kein Auftrag aktiviert."));
            }
            if action == "computer" {
                call(
                    json!({"op":"computer_setup","executable":executable,"state":root,"root":local}),
                )?;
            } else {
                let remote = operation["remote"]
                    .as_str()
                    .ok_or_else(|| failure("Missing remote sync folder"))?;
                call(
                    json!({"op":"jobs","state":state,"planned":[],"operation":{"action":"add","local_root":local,"remote_root":remote,"name":"","paused":false}}),
                )?;
            }
            call(start)?;
            call(command("reload", Value::Null))?;
        }
        _ => unreachable!(),
    }
    call(status)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn denied_portal_prevents_all_job_and_cloud_changes() {
        for action in ["add", "computer"] {
            let mut calls = vec![];
            let result = execute(
                "sdk",
                &json!({"action":action,"local":"/tmp/folder","remote":"/my-files"}),
                Path::new("/native/app"),
                true,
                &mut |r| {
                    calls.push(r.clone());
                    Ok(if r["op"] == "data_dir" {
                        json!("/tmp/state")
                    } else {
                        json!(false)
                    })
                },
            );
            assert!(result.is_err());
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[1]["op"], "portal");
        }
    }
    #[test]
    fn successful_setup_starts_native_daemon_then_reloads_and_reads_status() {
        for action in ["add", "computer"] {
            let mut calls = vec![];
            execute(
                "/sdk path",
                &json!({"action":action,"local":"/tmp/folder","remote":"/my-files"}),
                Path::new("/native/app"),
                true,
                &mut |r| {
                    calls.push(r.clone());
                    Ok(match r["op"].as_str().unwrap() {
                        "data_dir" => json!("/tmp/state"),
                        "portal" => json!(true),
                        _ => json!({}),
                    })
                },
            )
            .unwrap();
            assert_eq!(
                calls[2]["op"],
                if action == "add" {
                    "jobs"
                } else {
                    "computer_setup"
                }
            );
            assert_eq!(
                calls[3]["operation"]["argv"],
                json!([
                    "/native/app",
                    "--daemon",
                    "--state-dir",
                    "/tmp/state/background",
                    "--cli",
                    "/sdk path"
                ])
            );
            assert_eq!(calls[4]["action"], "reload");
            assert_eq!(calls[5]["operation"]["action"], "status");
        }
    }
    #[test]
    fn removal_pauses_before_mutation_and_does_not_continue_on_failure() {
        for fail_pause in [false, true] {
            let mut calls = vec![];
            let result = execute(
                "sdk",
                &json!({"action":"remove","id":"job-id"}),
                Path::new("/native/app"),
                false,
                &mut |r| {
                    calls.push(r.clone());
                    if r["action"] == "pause" && fail_pause {
                        return Err(failure("could not pause"));
                    }
                    Ok(if r["op"] == "data_dir" {
                        json!("/tmp/state")
                    } else {
                        json!({"running":true})
                    })
                },
            );
            assert_eq!(result.is_err(), fail_pause);
            assert_eq!(calls[2]["action"], "pause");
            if fail_pause {
                assert_eq!(calls.len(), 3);
            } else {
                assert_eq!(calls[3]["op"], "jobs");
                assert_eq!(calls[5]["action"], "reload");
            }
        }
    }
}
