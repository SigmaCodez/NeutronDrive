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
        "status",
        "start",
        "stop",
        "restart",
        "remove",
        "pause",
        "pause_1h",
        "pause_4h",
        "pause_24h",
        "resume",
        "retry",
        "sync",
        "add",
        "computer",
        "remove_backup",
        "remove_computer",
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
        "restart" => {
            let current = call(status.clone())?;
            if current["jobs"]
                .as_array()
                .is_some_and(|jobs| jobs.iter().any(|job| job["state"] == "syncing"))
            {
                return Err(failure(
                    "Wait for active synchronization to finish before restarting the service.",
                ));
            }
            call(service(json!({"action":"stop"})))?;
            call(start)?;
        }
        "stop" => {
            call(service(json!({"action":"stop"})))?;
        }
        "remove_backup" | "remove_computer" => {
            let jobs = call(
                json!({"op":"jobs","state":state,"planned":[],"operation":{"action":"list"}}),
            )?;
            let all = jobs
                .as_array()
                .ok_or_else(|| failure("Invalid sync job list"))?;
            let selected: Vec<Value> = if action == "remove_backup" {
                let job = all
                    .iter()
                    .find(|j| j["id"] == operation["id"])
                    .ok_or_else(|| failure("Backup job no longer exists"))?;
                if !job["remote_root"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("/devices/")
                {
                    return Err(failure("Not a computer backup"));
                }
                vec![job.clone()]
            } else {
                let prefix = operation["path"]
                    .as_str()
                    .filter(|p| p.starts_with("/devices/") && p.matches('/').count() == 2)
                    .ok_or_else(|| failure("Invalid computer path"))?;
                // A cloud rename must not leave the original local backup job running.
                let record = call(
                    json!({"op":"store_read","path":root.join("computer.json"),"default":null,"limit":1048576,"private":true}),
                )?;
                let saved_prefix = if record["uid"] == operation["uid"] {
                    record["path"]
                        .as_str()
                        .filter(|p| p.starts_with("/devices/") && p.matches('/').count() == 2)
                } else {
                    None
                };
                all.iter()
                    .filter(|j| {
                        j["remote_root"].as_str().is_some_and(|p| {
                            std::iter::once(prefix)
                                .chain(saved_prefix)
                                .any(|base| p == base || p.starts_with(&format!("{base}/")))
                        })
                    })
                    .cloned()
                    .collect()
            };
            let running = call(status.clone())?["running"] == true;
            for job in &selected {
                if running {
                    call(command("pause", job["id"].clone()))?;
                } else {
                    call(
                        json!({"op":"jobs","state":state,"planned":[],"operation":{"action":"update","id":job["id"],"name":null,"paused":true}}),
                    )?;
                }
            }
            let latest = call(status.clone())?;
            if running && latest.get("current").is_none() {
                return Err(failure("Restart the sync service before removing backups."));
            }
            if selected.iter().any(|j| latest["current"] == j["id"])
                || latest["jobs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|j| j["state"] == "syncing" && selected.iter().any(|v| v["id"] == j["id"]))
            {
                return Err(failure("Backup is stopping. Wait for the current transfer to finish, then remove it again."));
            }
            if action == "remove_backup" {
                let job = &selected[0];
                let root = call(
                    json!({"op":"drive","executable":executable,"operation":{"action":"info","path":"/my-files"}}),
                )?;
                let node = call(
                    json!({"op":"drive","executable":executable,"operation":{"action":"info","path":job["remote_root"]}}),
                )?;
                if root["uid"] != job["account_root_uid"]
                    || node["uid"] != job["remote_uid"]
                    || node["type"] != "folder"
                {
                    return Err(failure(
                        "Backup identity changed; cloud files were not removed.",
                    ));
                }
                call(
                    json!({"op":"drive","executable":executable,"operation":{"action":"trash","paths":[job["remote_root"]]}}),
                )?;
            } else {
                call(
                    json!({"op":"drive","executable":executable,"operation":{"action":"computer_remove","uid":operation["uid"],"root_uid":operation["root_uid"]}}),
                )?;
                // A subsequent setup must register a fresh device, not reuse a removed UID.
                let record = call(
                    json!({"op":"store_read","path":root.join("computer.json"),"default":null,"limit":1048576,"private":true}),
                );
                if let Ok(record) = record {
                    if record["uid"] == operation["uid"] {
                        call(
                            json!({"op":"store_write","path":root.join("computer.json"),"value":null}),
                        )?;
                    }
                }
            }
            for job in &selected {
                call(
                    json!({"op":"jobs","state":state,"planned":[],"operation":{"action":"remove","id":job["id"]}}),
                )?;
            }
            if running {
                call(command("reload", Value::Null))?;
            }
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
        "pause" | "pause_1h" | "pause_4h" | "pause_24h" | "resume" | "retry" | "sync" => {
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
    fn timed_pause_forwards_selected_job_and_rejects_missing_identity() {
        for action in ["pause_1h", "pause_4h", "pause_24h", "retry"] {
            let mut calls = Vec::new();
            execute(
                "sdk",
                &json!({"action":action,"id":"selected"}),
                Path::new("/app"),
                false,
                &mut |r| {
                    calls.push(r.clone());
                    if r["op"] == "data_dir" {
                        return Ok(json!("/tmp/nd-timed-pause"));
                    }
                    Ok(json!({"running":true,"jobs":[]}))
                },
            )
            .unwrap();
            assert!(calls.iter().any(|r| r["op"] == "control_command"
                && r["action"] == action
                && r["job_id"] == "selected"));
            let mut changed = false;
            let result = execute(
                "sdk",
                &json!({"action":action}),
                Path::new("/app"),
                false,
                &mut |r| {
                    if r["op"] == "data_dir" {
                        return Ok(json!("/tmp/nd-timed-pause"));
                    }
                    changed = true;
                    Ok(Value::Null)
                },
            );
            assert!(result.is_err());
            assert!(!changed);
        }
    }
    #[test]
    fn backup_removal_waits_for_active_worker_and_verifies_identity() {
        for (active, changed) in [(true, false), (false, true), (false, false)] {
            let mut calls = Vec::new();
            let job = json!({"id":"job","remote_root":"/devices/PC/docs","remote_uid":"folder","account_root_uid":"account"});
            let result = execute(
                "sdk",
                &json!({"action":"remove_backup","id":"job"}),
                Path::new("/app"),
                false,
                &mut |r| {
                    calls.push(r.clone());
                    if r["op"] == "data_dir" {
                        return Ok(json!("/tmp/nd-backup-test"));
                    }
                    if r["op"] == "jobs" && r["operation"]["action"] == "list" {
                        return Ok(json!([job]));
                    }
                    if r["op"] == "service_controller" {
                        return Ok(
                            json!({"running":true,"current":if active {json!("job")}else{Value::Null},"jobs":[]}),
                        );
                    }
                    if r["op"] == "drive" && r["operation"]["action"] == "info" {
                        return Ok(if r["operation"]["path"] == "/my-files" {
                            json!({"uid":"account"})
                        } else {
                            json!({"uid":if changed{"other"}else{"folder"},"type":"folder"})
                        });
                    }
                    Ok(Value::Null)
                },
            );
            assert_eq!(result.is_ok(), !active && !changed);
            assert_eq!(
                calls
                    .iter()
                    .filter(|r| r["operation"]["action"] == "trash")
                    .count(),
                usize::from(!active && !changed)
            );
            assert_eq!(
                calls
                    .iter()
                    .filter(|r| r["op"] == "jobs" && r["operation"]["action"] == "remove")
                    .count(),
                usize::from(!active && !changed)
            );
        }
    }
    #[test]
    fn renamed_computer_removal_pauses_original_job_before_cloud_delete() {
        let mut calls = Vec::new();
        let result = execute(
            "sdk",
            &json!({"action":"remove_computer","path":"/devices/New","uid":"device","root_uid":"root"}),
            Path::new("/app"),
            false,
            &mut |r| {
                calls.push(r.clone());
                Ok(match r["op"].as_str().unwrap_or("") {
                    "data_dir" => json!("/tmp/nd-backup-test"),
                    "jobs" if r["operation"]["action"] == "list" => {
                        json!([{"id":"old-job","remote_root":"/devices/Old/docs"}])
                    }
                    "store_read" => json!({"uid":"device","path":"/devices/Old"}),
                    "service_controller" => json!({"running":true,"current":"old-job","jobs":[]}),
                    _ => Value::Null,
                })
            },
        );
        assert!(result.is_err());
        assert!(calls.iter().any(|r| r["op"] == "control_command"
            && r["action"] == "pause"
            && r["job_id"] == "old-job"));
        assert!(!calls.iter().any(|r| r["op"] == "drive"));
    }
    #[test]
    fn restart_waits_for_confirmed_stop_and_refuses_active_sync() {
        for (syncing, stop_fails) in [(true, false), (false, true), (false, false)] {
            let mut calls = Vec::new();
            let result = execute(
                "sdk",
                &json!({"action":"restart"}),
                Path::new("/app"),
                false,
                &mut |r| {
                    calls.push(r.clone());
                    if r["op"] == "data_dir" {
                        return Ok(json!("/tmp/state"));
                    }
                    if r["operation"]["action"] == "stop" && stop_fails {
                        return Err(failure("Still stopping"));
                    }
                    Ok(
                        json!({"running":true,"jobs":[{"state":if syncing {"syncing"} else {"idle"}}]}),
                    )
                },
            );
            assert_eq!(result.is_ok(), !syncing && !stop_fails);
            assert_eq!(
                calls.iter().any(|r| r["operation"]["action"] == "start"),
                !syncing && !stop_fails
            );
            if !syncing && !stop_fails {
                assert_eq!(calls[2]["operation"]["action"], "stop");
                assert_eq!(calls[3]["operation"]["action"], "start");
            }
        }
    }
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
