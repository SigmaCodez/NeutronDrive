//! Native service entry points run before constructing any Qt application.
use crate::runtime::{self, Failure};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
static CANCELLED: AtomicBool = AtomicBool::new(false);
extern "C" fn cancel(_: libc::c_int) {
    CANCELLED.store(true, Ordering::Relaxed);
}
struct Signals {
    int: libc::sigaction,
    term: libc::sigaction,
}
impl Signals {
    fn install() -> Result<Self, Failure> {
        // SAFETY: valid sigaction storage; handler only stores a lock-free atomic.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = cancel as *const () as libc::sighandler_t;
            libc::sigemptyset(&mut action.sa_mask);
            let mut int = std::mem::zeroed();
            let mut term = std::mem::zeroed();
            if libc::sigaction(libc::SIGINT, &action, &mut int) != 0 {
                return Err(failure("Could not install signal handler"));
            }
            if libc::sigaction(libc::SIGTERM, &action, &mut term) != 0 {
                libc::sigaction(libc::SIGINT, &int, std::ptr::null_mut());
                return Err(failure("Could not install signal handler"));
            }
            Ok(Self { int, term })
        }
    }
}
impl Drop for Signals {
    fn drop(&mut self) {
        unsafe {
            libc::sigaction(libc::SIGINT, &self.int, std::ptr::null_mut());
            libc::sigaction(libc::SIGTERM, &self.term, std::ptr::null_mut());
        }
    }
}
fn failure(message: &str) -> Failure {
    Failure {
        message: message.into(),
        cancelled: false,
    }
}
const MODES: &[&str] = &[
    "--open-uri",
    "--daemon",
    "--background",
    "--desktop-service",
    "--sync-status",
    "--stop-background",
    "--mount",
    "--mount-foreground",
    "--unmount",
    "--pin",
    "--evict",
    "--service-status",
];
#[derive(Debug, PartialEq)]
struct Options {
    mode: String,
    cli: Option<String>,
    state: Option<PathBuf>,
    target: String,
    remote: String,
    writable: bool,
    offline: bool,
}
fn parse(args: &[String]) -> Result<Options, Failure> {
    let mut options = Options {
        mode: String::new(),
        cli: None,
        state: None,
        target: String::new(),
        remote: "/my-files".into(),
        writable: false,
        offline: false,
    };
    let mut diagnose = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if MODES.contains(&arg.as_str()) {
            if !options.mode.is_empty() {
                return Err(failure("Only one service mode is allowed"));
            }
            options.mode = arg.clone();
            if [
                "--mount",
                "--mount-foreground",
                "--pin",
                "--evict",
                "--open-uri",
            ]
            .contains(&arg.as_str())
            {
                options.target = option_value(&mut args)?.to_string();
            }
        } else {
            match arg.as_str() {
                "--cli" => options.cli = Some(option_value(&mut args)?.to_string()),
                "--state-dir" => options.state = Some(PathBuf::from(option_value(&mut args)?)),
                "--remote" => options.remote = option_value(&mut args)?.to_string(),
                "--read-write" => options.writable = true,
                "--offline" => options.offline = true,
                "--diagnose" => diagnose = true,
                "--demo" => {}
                _ => return Err(failure("Unknown command-line option")),
            }
        }
    }
    if options.mode.is_empty() && diagnose {
        options.mode = "--diagnose".into();
    }
    if options.mode.is_empty() {
        return Err(failure("Missing service mode"));
    }
    Ok(options)
}
fn option_value<'a>(args: &mut impl Iterator<Item = &'a String>) -> Result<&'a str, Failure> {
    args.next()
        .filter(|v| !v.is_empty() && !v.starts_with("--"))
        .map(String::as_str)
        .ok_or_else(|| failure("Missing option value"))
}
pub fn executable(explicit: Option<&str>) -> String {
    if let Some(value) = explicit.filter(|s| !s.is_empty()) {
        return value.into();
    }
    if let Ok(value) = std::env::var("NEUTRONDRIVE_CLI") {
        if !value.is_empty() {
            return value;
        }
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let path = dir.join("proton-drive");
        if path.is_file() {
            use std::os::unix::fs::PermissionsExt;
            if path
                .metadata()
                .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
            {
                if let Some(value) = path.to_str() {
                    return value.into();
                }
            }
        }
    }
    String::new()
}
pub fn desktop_request(executable: &str, request: Value) -> Result<Value, Failure> {
    let app = std::env::current_exe()
        .map_err(|_| failure("Native application executable unavailable"))?;
    let source = app.parent().unwrap_or(Path::new("/"));
    let mut argv = vec![json!(app), json!("--desktop-service")];
    if !executable.is_empty() {
        argv.extend([json!("--cli"), json!(executable)]);
    }
    Ok(
        json!({"op":"desktop_bus","operation":{"action":"request","request":request,
        "launcher":{"argv":argv,"source":source}}}),
    )
}
fn mount_value(value: Value) -> Result<Value, Failure> {
    if value.get("errno").is_some() {
        return Err(failure(
            value["message"]
                .as_str()
                .unwrap_or("Mount operation failed"),
        ));
    }
    if let Some(error) = value["backend_error"].as_str() {
        return Err(Failure {
            message: error.into(),
            cancelled: value["cancelled"].as_bool().unwrap_or(false),
        });
    }
    value
        .get("value")
        .cloned()
        .ok_or_else(|| failure("Invalid mount reply"))
}
fn foreground(
    options: &Options,
    data: &Path,
    cli: &str,
    call: &mut impl FnMut(Value) -> Result<Value, Failure>,
) -> Result<Value, Failure> {
    let id = mount_value(call(
        json!({"op":"mount_manager","executable":cli,"operation":{"action":"create","state_dir":data.join("ondemand-foreground")}}),
    )?)?;
    let result = (|| {
        mount_value(call(
            json!({"op":"mount_manager","executable":cli,"operation":{"action":"prepare","id":id,
            "mountpoint":std::path::absolute(&options.target).map_err(|_|failure("Invalid mount directory"))?,
            "remote_root":options.remote,"writable":options.writable,"offline":options.offline}}),
        )?)?;
        let status = mount_value(call(
            json!({"op":"mount_manager","executable":cli,"operation":{"action":"run","id":id}}),
        )?)?;
        if let Some(error) = status["error"].as_str().filter(|s| !s.is_empty()) {
            return Err(Failure {
                message: error.into(),
                cancelled: status["cancelled"] == true,
            });
        }
        Ok(status)
    })();
    if result.is_err() {
        let _ = call(json!({"op":"mount_manager","operation":{"action":"abort","id":id}}));
    }
    let forgotten = call(json!({"op":"mount_manager","operation":{"action":"forget","id":id}}))
        .and_then(mount_value);
    result.and_then(|value| forgotten.map(|_| value))
}
fn execute(
    options: &Options,
    data: &Path,
    cli: &str,
    call: &mut impl FnMut(Value) -> Result<Value, Failure>,
) -> Result<(Value, bool), Failure> {
    let state = options
        .state
        .clone()
        .unwrap_or_else(|| data.join("background"));
    let (request, print) = match options.mode.as_str() {
        "--open-uri" => (
            json!({"op":"portal","operation":{"action":"open_uri","uri":options.target}}),
            false,
        ),
        "--sync-status" | "--stop-background" => (
            json!({"op":"service_controller","state":state,
            "operation":{"action":if options.mode=="--sync-status" {"status"} else {"stop"}}}),
            options.mode == "--sync-status",
        ),
        "--desktop-service" => (
            json!({"op":"desktop_runtime","config":{"state_dir":data.join("ondemand-service"),"executable":cli}}),
            false,
        ),
        "--daemon" | "--background" => {
            if options.mode == "--background" {
                let _ = call(desktop_request(cli, json!({"action":"restore"}))?);
            }
            (
                json!({"op":"daemon","config":{"state":state,"executable":cli}}),
                false,
            )
        }
        "--mount-foreground" => return foreground(options, data, cli, call).map(|v| (v, false)),
        _ => {
            let request = match options.mode.as_str() {
                "--mount" => {
                    json!({"action":"mount","mountpoint":std::path::absolute(&options.target).map_err(|_|failure("Invalid mount directory"))?,"remote_root":options.remote,"writable":options.writable,"offline":options.offline})
                }
                "--unmount" => json!({"action":"unmount"}),
                "--pin" | "--evict" => {
                    let target = Path::new(&options.target);
                    let relative = if target.is_absolute() {
                        let status = call(desktop_request(cli, json!({"action":"status"}))?)?;
                        if status["mounted"] != true {
                            return Err(failure("The sync folder is not active"));
                        }
                        let root = status["mountpoint"]
                            .as_str()
                            .ok_or_else(|| failure("Sync folder unavailable"))?;
                        let path = target.strip_prefix(root).map_err(|_| {
                            failure("The selected file is outside the active sync folder")
                        })?;
                        if path
                            .components()
                            .any(|c| !matches!(c, std::path::Component::Normal(_)))
                        {
                            return Err(failure("Invalid sync path"));
                        }
                        path.to_string_lossy().into_owned()
                    } else {
                        options.target.clone()
                    };
                    json!({"action":options.mode.trim_start_matches("--"),"path":relative})
                }
                "--service-status" => json!({"action":"status"}),
                _ => return Err(failure("Unknown native command")),
            };
            (desktop_request(cli, request)?, true)
        }
    };
    call(request).map(|value| (value, print))
}
fn diagnose(cli: &str, text: &crate::cli_text::CliText) -> i32 {
    use std::ffi::CString;
    let available = Path::new(cli).is_file()
        && CString::new(cli).is_ok_and(|path| {
            // SAFETY: path is a live NUL-terminated string for this call.
            unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
        });
    let mut result =
        json!({"app":env!("CARGO_PKG_VERSION"),"backend_found":available,"backend":cli});
    let info = neutrondrive_core::policy_request(br#"{"version":1,"op":"info"}"#);
    if info["ok"] == true {
        result["rust_core"] = info["result"].clone();
    }
    match runtime::call(
        json!({"op":"drive","executable":cli,"operation":{"action":"version"}}),
        &mut |_| i32::from(CANCELLED.load(Ordering::Relaxed)),
    ) {
        Ok(version) => result["backend_version"] = version,
        Err(error) => result["error"] = text.diagnostic(&error.message).into(),
    }
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    i32::from(result.get("error").is_some())
}
pub fn run(args: &[String]) -> Option<i32> {
    if args.iter().any(|a| a == "--version") {
        println!("NeutronDrive {}", env!("CARGO_PKG_VERSION"));
        return Some(0);
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", crate::cli_text::CliText::system().help());
        return Some(0);
    }
    if !args
        .iter()
        .any(|a| MODES.contains(&a.as_str()) || a == "--diagnose")
    {
        return None;
    }
    let text = crate::cli_text::CliText::system();
    let options = match parse(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{}", text.diagnostic(&error.message));
            return Some(2);
        }
    };
    let cli = executable(options.cli.as_deref());
    let _signals = match Signals::install() {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("{}", text.diagnostic(&error.message));
            return Some(1);
        }
    };
    if options.mode == "--diagnose" {
        return Some(diagnose(&cli, &text));
    }
    let result = (|| {
        let data = runtime::value(json!({"op":"data_dir"}))?;
        let data = PathBuf::from(
            data.as_str()
                .ok_or_else(|| failure("Invalid application data directory"))?,
        );
        let mut callback = |_| i32::from(CANCELLED.load(Ordering::Relaxed));
        execute(&options, &data, &cli, &mut |request| {
            runtime::call(request, &mut callback)
        })
    })();
    Some(match result {
        Ok((value, print)) => {
            if print {
                println!("{}", serde_json::to_string_pretty(&value).unwrap());
            }
            0
        }
        Err(error) if error.cancelled && CANCELLED.load(Ordering::Relaxed) => 0,
        Err(error) => {
            eprintln!("{}", text.diagnostic(&error.message));
            1
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_helper_routes_to_portal_without_backend_execution() {
        let options = parse(&args(&["--open-uri", "https://example.test/path#fragment"])).unwrap();
        let mut requests = Vec::new();
        let (_, print) = execute(
            &options,
            Path::new("/isolated"),
            "/unused-sdk",
            &mut |request| {
                requests.push(request);
                Ok(json!(true))
            },
        )
        .unwrap();
        assert!(!print);
        assert_eq!(
            requests,
            vec![
                json!({"op":"portal","operation":{"action":"open_uri","uri":"https://example.test/path#fragment"}})
            ]
        );
        assert!(parse(&args(&["--open-uri"])).is_err());
    }
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }
    #[test]
    fn service_arguments_preserve_paths_and_reject_ambiguous_modes() {
        assert_eq!(
            parse(&args(&[
                "--cli",
                "/opt/sdk with space",
                "--daemon",
                "--state-dir",
                "/tmp/my state"
            ]))
            .unwrap(),
            Options {
                mode: "--daemon".into(),
                cli: Some("/opt/sdk with space".into()),
                state: Some(PathBuf::from("/tmp/my state")),
                target: String::new(),
                remote: "/my-files".into(),
                writable: false,
                offline: false
            }
        );
        for argv in [
            vec!["--daemon", "--background"],
            vec!["--desktop-service", "--cli"],
            vec!["--daemon", "--state-dir", "--cli"],
            vec!["--daemon", "--unknown"],
            vec![],
        ] {
            assert!(parse(&args(&argv)).is_err());
        }
    }
    #[test]
    fn native_service_launcher_never_uses_an_interpreter() {
        let request = desktop_request("/sdk with space", json!({"action":"status"})).unwrap();
        let argv = &request["operation"]["launcher"]["argv"];
        assert_eq!(argv[0], json!(std::env::current_exe().unwrap()));
        assert_eq!(argv[1], "--desktop-service");
        assert_eq!(argv[3], "/sdk with space");
        assert_eq!(
            desktop_request("", json!({"action":"status"})).unwrap()["operation"]["launcher"]
                ["argv"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn all_command_routes_keep_options_and_output_contracts() {
        let root = Path::new("/tmp/private");
        for mode in [
            "--sync-status",
            "--stop-background",
            "--service-status",
            "--unmount",
        ] {
            let options = parse(&args(&[mode, "--state-dir", "/tmp/custom"])).unwrap();
            let mut request = Value::Null;
            let (result, print) = execute(&options, root, "sdk", &mut |r| {
                request = r;
                Ok(json!({"running":false}))
            })
            .unwrap();
            assert_eq!(result, json!({"running":false}));
            assert_eq!(print, mode != "--stop-background");
            if mode == "--sync-status" || mode == "--stop-background" {
                assert_eq!(request["state"], "/tmp/custom");
            } else {
                assert_eq!(request["op"], "desktop_bus");
            }
        }
        for mode in ["--pin", "--evict"] {
            let options = parse(&args(&[mode, "folder with space/file"])).unwrap();
            execute(&options, root, "sdk", &mut |r| {
                assert_eq!(r["operation"]["request"]["path"], "folder with space/file");
                Ok(json!({}))
            })
            .unwrap();
        }
        let options = parse(&args(&[
            "--mount",
            "/tmp/my mount",
            "--remote",
            "/my-files/folder",
            "--read-write",
            "--offline",
        ]))
        .unwrap();
        execute(&options,root,"sdk",&mut |r| {assert_eq!(r["operation"]["request"],json!({"action":"mount","mountpoint":"/tmp/my mount","remote_root":"/my-files/folder","writable":true,"offline":true}));Ok(json!({}))}).unwrap();
    }
    #[test]
    fn background_restore_failure_does_not_prevent_daemon_start() {
        let options = parse(&args(&["--background"])).unwrap();
        let mut requests = vec![];
        execute(&options, Path::new("/tmp/private"), "sdk", &mut |r| {
            requests.push(r.clone());
            if r["op"] == "desktop_bus" {
                Err(failure("optional mount unavailable"))
            } else {
                Ok(Value::Null)
            }
        })
        .unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["operation"]["request"]["action"], "restore");
        assert_eq!(requests[1]["op"], "daemon");
    }
    #[test]
    fn foreground_mount_always_releases_manager_after_failure() {
        let options = parse(&args(&["--mount-foreground", "/tmp/mount", "--offline"])).unwrap();
        for failure_at in ["prepare", "run", ""] {
            let mut actions = vec![];
            let result = foreground(&options, Path::new("/tmp/private"), "sdk", &mut |r| {
                let action = r["operation"]["action"].as_str().unwrap();
                actions.push(action.to_string());
                Ok(if action == failure_at {
                    json!({"errno":5,"message":"synthetic mount failure"})
                } else if action == "create" {
                    json!({"value":7})
                } else {
                    json!({"value":{"mounted":false}})
                })
            });
            assert_eq!(result.is_err(), !failure_at.is_empty());
            assert_eq!(actions.last().unwrap(), "forget");
            assert_eq!(
                actions.contains(&"abort".to_string()),
                !failure_at.is_empty()
            );
            if failure_at == "prepare" {
                assert!(!actions.contains(&"run".to_string()));
            }
        }
    }
}
