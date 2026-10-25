//! Transactional update handoff. No signals, forced unmounts or GUI exit before
//! successful installation and restoration by the newly installed executable.
use crate::{
    headless,
    runtime::{self, Failure},
    updates::failure,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub app: PathBuf,
    pub cli: String,
    pub sync: bool,
    pub drive: bool,
}
static RELAUNCH: OnceLock<Plan> = OnceLock::new();
pub fn request_relaunch(plan: Plan) {
    let _ = RELAUNCH.set(plan);
}
pub fn relaunch() {
    use std::os::unix::process::CommandExt;
    if let Some(plan) = RELAUNCH.get() {
        let mut command = Command::new(&plan.app);
        if !plan.cli.is_empty() {
            command.args(["--cli", &plan.cli]);
        }
        let error = command.exec();
        eprintln!("Update installed, but NeutronDrive could not reopen: {error}");
        std::process::exit(1);
    }
}

pub fn install(path: &Path, callback: &mut impl FnMut(Value) -> i32) -> Result<Plan, Failure> {
    // Capture before apt replaces the file: current_exe afterwards can refer to
    // an unlinked '(deleted)' executable on Linux.
    let installer = crate::update_installer::prepare(path, callback)?;
    let app = std::env::current_exe().map_err(|_| failure("Application path unavailable"))?;
    let cli = crate::startup::backend_executable();
    let data = runtime::value(json!({"op":"data_dir"}))?;
    let state = Path::new(
        data.as_str()
            .ok_or_else(|| failure("Invalid state directory"))?,
    )
    .join("background");
    transaction(app, cli, &mut |action, plan| match action {
        "sync_status" => runtime::value(
            json!({"op":"service_controller","state":state,"operation":{"action":"status"}}),
        ),
        "drive_status" => {
            runtime::value(headless::desktop_request("", json!({"action":"status"}))?)
        }
        "stop_sync" => runtime::value(
            json!({"op":"service_controller","state":state,"operation":{"action":"stop_idle"}}),
        ),
        "stop_drive" => {
            runtime::value(json!({"op":"desktop_bus","operation":{"action":"stop_for_update"}}))
        }
        "install" => installer.install(path, callback).map(|()| Value::Null),
        "restore" => restore_with_binary(plan).map(|()| Value::Null),
        _ => Err(failure("Invalid update transition")),
    })
}
fn transaction(
    app: PathBuf,
    cli: String,
    call: &mut impl FnMut(&str, &Plan) -> Result<Value, Failure>,
) -> Result<Plan, Failure> {
    let mut plan = Plan {
        app,
        cli,
        sync: false,
        drive: false,
    };
    let sync = call("sync_status", &plan)?;
    if sync["running"] == true
        && sync["jobs"]
            .as_array()
            .is_some_and(|jobs| jobs.iter().any(|job| job["state"] == "syncing"))
    {
        return Err(failure(
            "Bitte den laufenden Vorgang abschließen und erneut versuchen.",
        ));
    }
    plan.sync = sync["running"] == true;
    let drive = call("drive_status", &plan)?;
    if drive["busy"] == true {
        return Err(failure(
            "Bitte den laufenden Vorgang abschließen und erneut versuchen.",
        ));
    }
    plan.drive = drive["available"] == true;
    // Stop the daemon first so it cannot open new FUSE files during unmount.
    // StopIdle makes the final idle decision atomically inside the daemon.
    call("stop_sync", &plan)?;
    let result: Result<(), Failure> = (|| {
        let stopped = call("stop_drive", &plan)?;
        plan.drive |= stopped["running"] == true;
        call("install", &plan)?;
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(recovery) = call("restore", &plan) {
            return Err(failure(&format!(
                "{} Service recovery also failed: {}",
                error.message, recovery.message
            )));
        }
        return Err(error);
    }
    Ok(plan)
}

pub fn restore_with_binary(plan: &Plan) -> Result<(), Failure> {
    let payload = serde_json::to_string(plan).map_err(|_| failure("Invalid restart plan"))?;
    let status = Command::new(&plan.app)
        .args(["--restore-update-services", &payload])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .status().map_err(|_| failure("Update installed, but the new service launcher could not start. Retry the restart."))?;
    if !status.success() {
        return Err(failure("Service restoration failed. Retry the restart; the update does not need to be installed again."));
    }
    Ok(())
}
pub fn restore_mode(args: &[String]) -> Option<i32> {
    if args.first().map(String::as_str) != Some("--restore-update-services") {
        return None;
    }
    let result: Result<(), Failure> = (|| {
        if args.len() != 2 {
            return Err(failure("Invalid service restart arguments"));
        }
        let plan: Plan =
            serde_json::from_str(&args[1]).map_err(|_| failure("Invalid service restart plan"))?;
        let mut error = None;
        if plan.drive {
            if let Err(e) = runtime::value(headless::desktop_request(
                &plan.cli,
                json!({"action":"restore"}),
            )?) {
                error = Some(e);
            }
        }
        // Attempt both restorations even if one failed; preserve paused jobs.
        if plan.sync {
            if let Err(e) =
                crate::background::run(&plan.cli, &json!({"action":"start"}), &mut |_| 0)
            {
                error = Some(e);
            }
        }
        error.map_or(Ok(()), Err)
    })();
    Some(match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{}", e.message);
            1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relaunch_executes_saved_path_with_literal_backend_argument() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("nd-relaunch-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let app = root.join("new app");
        let marker = root.join("arguments");
        std::fs::write(
            &app,
            b"#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$ND_RELAUNCH_MARKER\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "update_restart::tests::relaunch_child"])
            .env("ND_RELAUNCH_CHILD", &app)
            .env("ND_RELAUNCH_MARKER", &marker)
            .status()
            .unwrap();
        assert!(result.success());
        assert_eq!(
            std::fs::read_to_string(marker).unwrap(),
            "--cli\n/sdk with spaces\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn relaunch_child() {
        if let Some(app) = std::env::var_os("ND_RELAUNCH_CHILD") {
            request_relaunch(Plan {
                app: app.into(),
                cli: "/sdk with spaces".into(),
                sync: false,
                drive: false,
            });
            relaunch();
            panic!("exec must replace the process");
        }
    }
    #[test]
    fn stopped_services_are_not_enabled_by_the_update() {
        let plan = transaction("/app".into(), String::new(), &mut |action, _| {
            Ok(match action {
                "sync_status" => json!({"running":false,"jobs":[]}),
                "drive_status" => json!({"available":false}),
                _ => Value::Null,
            })
        })
        .unwrap();
        assert!(!plan.sync && !plan.drive);
    }
    fn execute(fail: &str, active: bool) -> (Result<Plan, Failure>, Vec<String>) {
        let mut calls = vec![];
        let result = transaction(
            PathBuf::from("/new/app"),
            "/sdk".into(),
            &mut |action, _| {
                calls.push(action.into());
                if action == fail {
                    return Err(failure("synthetic"));
                }
                Ok(match action {
                    "sync_status" => {
                        json!({"running":true,"jobs":[{"state":if active {"syncing"} else {"idle"}}]})
                    }
                    "drive_status" => json!({"available":true,"busy":false}),
                    _ => Value::Null,
                })
            },
        );
        (result, calls)
    }
    #[test]
    fn refuses_active_transfers_before_touching_services() {
        let (result, calls) = execute("", true);
        assert!(result.is_err());
        assert_eq!(calls, ["sync_status"]);
    }
    #[test]
    fn installation_requires_both_confirmed_stops_and_restores_after_failure() {
        for fail in ["stop_drive", "install"] {
            let (result, calls) = execute(fail, false);
            assert!(result.is_err());
            assert_eq!(calls.last().unwrap(), "restore");
            if fail == "stop_drive" {
                assert!(!calls.iter().any(|c| c == "install"));
            }
        }
        let (result, calls) = execute("stop_sync", false);
        assert!(result.is_err());
        assert!(!calls.iter().any(|c| c == "stop_drive" || c == "install"));
    }
    #[test]
    fn successful_install_preserves_restart_plan_without_relaunching_old_code() {
        let (result, calls) = execute("", false);
        let plan = result.unwrap();
        assert!(plan.sync && plan.drive);
        assert_eq!(plan.app, Path::new("/new/app"));
        assert_eq!(
            calls,
            [
                "sync_status",
                "drive_status",
                "stop_sync",
                "stop_drive",
                "install"
            ]
        );
    }
}
