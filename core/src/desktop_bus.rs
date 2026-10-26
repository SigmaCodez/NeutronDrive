//! Native bounded session-bus client. Only undelivered calls may activate a source helper.
use crate::process::Failure;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    future::{poll_fn, Future},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    task::Poll,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Failure>;
const APP: &str = "io.github.SigmaCodez.NeutronDrive";
const PATH: &str = "/io/github/SigmaCodez/NeutronDrive/Service";
const MAX_REQUEST: usize = 16384;
const MAX_RESPONSE: usize = 8 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launcher {
    argv: Vec<String>,
    source: PathBuf,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    StopForUpdate,
    Request {
        request: Value,
        launcher: Option<Launcher>,
    },
}
fn bus_name() -> String {
    format!(
        "{APP}.{}",
        if std::env::var_os("SNAP").is_some_and(|v| !v.is_empty()) {
            "SnapService"
        } else {
            "Service"
        }
    )
}
fn poll(callback: &mut impl FnMut(Value) -> i32) -> Result<()> {
    if callback(json!({"kind":"poll"})) != 0 {
        return Err(Failure {
            message: "Dienstanfrage abgebrochen.".into(),
            cancelled: true,
        });
    }
    Ok(())
}
pub(crate) async fn bounded<F: Future>(
    future: F,
    duration: Duration,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<F::Output> {
    poll(callback)?;
    let deadline = Instant::now() + duration;
    let runtime =
        zruntime::SharedRuntime::current().map_err(|_| "D-Bus-Laufzeit ist nicht verfügbar.")?;
    let mut tick = Box::pin(
        runtime.sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100)),
        ),
    );
    let mut future = Box::pin(future);
    poll_fn(|cx|{
        if Instant::now()>=deadline{return Poll::Ready(Err("Dienstanfrage hat das Zeitlimit überschritten; Status vor erneutem Versuch prüfen.".into()));}
        if tick.as_mut().poll(cx).is_ready(){
            if let Err(e)=poll(callback){return Poll::Ready(Err(e));}
            tick=Box::pin(runtime.sleep(deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(100))));
            let _=tick.as_mut().poll(cx);
        }
        future.as_mut().poll(cx).map(Ok)
    }).await
}
async fn connected(callback: &mut impl FnMut(Value) -> i32) -> Result<zbus::Connection> {
    bounded(
        zbus::Connection::session(),
        Duration::from_secs(5),
        callback,
    )
    .await?
    .map_err(|_| "Der Sitzungsbus ist nicht erreichbar.".into())
}
async fn has_owner(
    connection: &zbus::Connection,
    name: &str,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<bool> {
    let reply = bounded(
        connection.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "NameHasOwner",
            &(name,),
        ),
        Duration::from_secs(3),
        callback,
    )
    .await?
    .map_err(|_| "Der Sitzungsbus ist nicht erreichbar.")?;
    reply
        .body()
        .deserialize::<bool>()
        .map_err(|_| "Ungültige Sitzungsbus-Antwort.".into())
}
fn missing(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::FDO(error) => matches!(
            **error,
            zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_)
        ),
        zbus::Error::MethodError(name, _, _) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
        ),
        _ => false,
    }
}

fn response(raw: &str) -> Result<Value> {
    if raw.len() > MAX_RESPONSE {
        return Err("Die Dienstantwort ist zu groß.".into());
    }
    let result: Value = serde_json::from_str(raw)
        .map_err(|_| "Der Laufwerksdienst lieferte eine ungültige Antwort.")?;
    if !result.is_object() || !result["ok"].is_boolean() {
        return Err("Der Laufwerksdienst lieferte eine ungültige Antwort.".into());
    }
    if result["ok"] != true {
        return Err(result["error"]
            .as_str()
            .unwrap_or("Laufwerksaktion fehlgeschlagen.")
            .into());
    }
    let value = result.get("result").cloned().unwrap_or_else(|| json!({}));
    if !value.is_object() {
        return Err("Der Laufwerksdienst lieferte einen ungültigen Status.".into());
    }
    Ok(value)
}
struct Startup(Option<Child>);
impl Drop for Startup {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Startup {
    fn release(mut self) -> Result<()> {
        let child = Arc::new(Mutex::new(self.0.take()));
        let thread_child = child.clone();
        match std::thread::Builder::new()
            .name("desktop-service-reaper".into())
            .spawn(move || {
                if let Some(mut child) = thread_child
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take()
                {
                    let _ = child.wait();
                }
            }) {
            Ok(_) => Ok(()),
            Err(_) => {
                self.0 = child.lock().unwrap_or_else(|p| p.into_inner()).take();
                Err("Dienstprozess konnte nicht überwacht werden.".into())
            }
        }
    }
}
fn start(launcher: Launcher) -> Result<Startup> {
    if launcher.argv.is_empty() || !launcher.source.is_absolute() {
        return Err("Ungültiger Dienststarter.".into());
    }
    let mut command = Command::new(&launcher.argv[0]);
    command.args(&launcher.argv[1..]);
    // The launcher executes a native program. `source` remains accepted for
    // local protocol compatibility, but no interpreter environment is injected.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let maximum = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 4u32) < 0 {
                if maximum < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                for fd in 3..maximum.min(i32::MAX as i64) as i32 {
                    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                }
            }
            Ok(())
        });
    }
    let child = command.spawn().map_err(|_| {
        "Der native Laufwerksdienst konnte nicht starten. FUSE-Abhängigkeiten prüfen."
    })?;
    Ok(Startup(Some(child)))
}
async fn request(
    request: Value,
    launcher: Option<Launcher>,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    let action = request
        .get("action")
        .and_then(Value::as_str)
        .ok_or("Ungültige Dienstanfrage.")?;
    let payload = serde_json::to_string(&request).map_err(|_| "Ungültige Dienstanfrage.")?;
    if payload.len() > MAX_REQUEST {
        return Err("Dienstanfrage ist zu groß.".into());
    }
    let connection = connected(callback).await?;
    let name = bus_name();
    let status = action == "status";
    if status && !has_owner(&connection, &name, callback).await? {
        return Ok(json!({"available":false,"mounted":false}));
    }
    let proxy = zbus::proxy::Builder::<zbus::Proxy<'_>>::new(&connection)
        .destination(name.as_str())
        .path(PATH)
        .interface(name.as_str())
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .map_err(|_| "Ungültiger Laufwerksdienst.")?;
    let flags = if status {
        zbus::proxy::MethodFlags::NO_AUTO_START
    } else {
        zbus::proxy::MethodFlags::empty()
    };
    let timeout = Duration::from_secs(if status { 5 } else { 650 });
    let answer = bounded(
        proxy.call_with_flags::<_, _, String>("Request", flags, &(payload.as_str(),)),
        timeout,
        callback,
    )
    .await?;
    let raw=match answer{
        Ok(Some(raw))=>raw,
        Err(error) if missing(&error)=>{
            if status{return Ok(json!({"available":false,"mounted":false}));}
            if std::env::var_os("FLATPAK_ID").is_some_and(|v|!v.is_empty()){
                return Err("Der native NeutronDrive-Helfer ist nicht erreichbar. Für Flatpak das passende native Paket installieren.".into());
            }
            let mut child=start(launcher.ok_or("Der native Laufwerksdienst ist nicht installiert.")?)?;
            let deadline=Instant::now()+Duration::from_secs(4);
            loop{
                if has_owner(&connection,&name,callback).await?{break;}
                if child.0.as_mut().unwrap().try_wait().map_err(|_|"Dienstprozess konnte nicht geprüft werden.")?.is_some(){return Err("Der native Laufwerksdienst wurde vorzeitig beendet.".into());}
                if Instant::now()>=deadline{return Err("Der native Laufwerksdienst konnte nicht starten. FUSE-Abhängigkeiten prüfen.".into());}
                poll(callback)?;
                zruntime::SharedRuntime::current().map_err(|_|"D-Bus-Laufzeit ist nicht verfügbar.")?.sleep(Duration::from_millis(100)).await;
            }
            child.release()?;
            bounded(proxy.call::<_,_,String>("Request",&(payload.as_str(),)),Duration::from_secs(650),callback).await?
                .map_err(|_|"Der native Laufwerksdienst ist nicht erreichbar. Status vor erneutem Versuch prüfen.")?
        }
        _=>return Err("Der native NeutronDrive-Helfer ist nicht erreichbar. Bei einer unterbrochenen Aktion zuerst den Status prüfen.".into())
    };
    response(&raw)
}
// Two-phase, non-activating shutdown. The first reply acknowledges a safe
// unmount; disappearance of that exact connection confirms worker cleanup.
async fn stop_for_update(callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    let connection = connected(callback).await?;
    let name = bus_name();
    if !has_owner(&connection, &name, callback).await? {
        return Ok(json!({"running":false}));
    }
    let reply = bounded(
        connection.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetNameOwner",
            &(name.as_str(),),
        ),
        Duration::from_secs(3),
        callback,
    )
    .await?
    .map_err(|_| "Could not identify the drive service.")?;
    let owner: String = reply
        .body()
        .deserialize()
        .map_err(|_| "Invalid service owner.")?;
    let proxy = zbus::proxy::Builder::<zbus::Proxy<'_>>::new(&connection)
        .destination(owner.as_str())
        .path(PATH)
        .interface(name.as_str())
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .map_err(|_| "Invalid drive service.")?;
    let raw: String = bounded(
        proxy.call("Request", &(r#"{"action":"prepare_update"}"#,)),
        Duration::from_secs(650),
        &mut |_| 0,
    )
    .await?
    .map_err(|_| "Drive service could not prepare the update. Close open files and try again.")?;
    if response(&raw)?["prepared"] != true {
        return Err("Drive service did not confirm update preparation.".into());
    }
    // Once preparation is acknowledged, cancellation must not strand a frozen
    // service. A lost finish reply is resolved by the exact owner's departure.
    let _ = bounded(
        proxy.call::<_, _, String>("Request", &(r#"{"action":"finish_update"}"#,)),
        Duration::from_secs(5),
        &mut |_| 0,
    )
    .await;
    let deadline = Instant::now() + Duration::from_secs(30);
    while has_owner(&connection, &owner, &mut |_| 0).await? {
        if Instant::now() >= deadline {
            return Err(
                "Drive service shutdown is not confirmed; update was not installed.".into(),
            );
        }
        zruntime::SharedRuntime::current()
            .map_err(|_| "D-Bus runtime unavailable.")?
            .sleep(Duration::from_millis(50))
            .await;
    }
    Ok(json!({"running":true}))
}
fn sandbox_reply(operation: &Operation) -> Result<Value> {
    match operation {
        Operation::Request { request, .. } if matches!(request["action"].as_str(), Some("status" | "restore")) =>
            Ok(json!({"available":false,"mounted":false,"reason":"Online placeholders are unavailable in the sandbox. Use folder synchronization in Settings."})),
        _ => Err("Online placeholders are unavailable in the sandbox. Use folder synchronization in Settings.".into()),
    }
}
pub fn run(operation: Operation, callback: &mut impl FnMut(Value) -> i32) -> Result<Value> {
    if ["FLATPAK_ID", "SNAP"]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()))
    {
        return sandbox_reply(&operation);
    }
    match operation {
        Operation::StopForUpdate => zbus::block_on(stop_for_update(callback)),
        Operation::Request {
            request: payload,
            launcher,
        } => zbus::block_on(request(payload, launcher, callback)),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sandbox_never_activates_a_host_mount_service() {
        for action in ["status", "restore"] {
            let value = sandbox_reply(&Operation::Request {
                request: json!({"action":action}),
                launcher: None,
            })
            .unwrap();
            assert_eq!(value["available"], false);
            assert_eq!(value["mounted"], false);
        }
        for action in ["mount", "pin", "evict", "login", "unmount"] {
            assert!(sandbox_reply(&Operation::Request {
                request: json!({"action":action}),
                launcher: None
            })
            .is_err());
        }
        assert!(sandbox_reply(&Operation::StopForUpdate).is_err());
    }
    #[test]
    fn standard_errors_only_retry_provably_undelivered_requests() {
        for error in [
            zbus::fdo::Error::ServiceUnknown("fixture".into()),
            zbus::fdo::Error::NameHasNoOwner("fixture".into()),
        ] {
            assert!(missing(&error.into()));
        }
        for error in [
            zbus::fdo::Error::NoReply("fixture".into()),
            zbus::fdo::Error::AccessDenied("fixture".into()),
            zbus::fdo::Error::Disconnected("fixture".into()),
        ] {
            assert!(!missing(&error.into()));
        }
    }
    #[test]
    fn response_validation_preserves_domain_errors() {
        assert_eq!(
            response(r#"{"ok":true,"result":{"mounted":true}}"#).unwrap(),
            json!({"mounted":true})
        );
        for raw in ["[]", r#"{"ok":1}"#, r#"{"ok":true,"result":[]}"#, "no JSON"] {
            assert!(response(raw).is_err());
        }
        assert_eq!(
            response(r#"{"ok":false,"error":"synthetic"}"#)
                .unwrap_err()
                .message,
            "synthetic"
        );
        assert!(response(&" ".repeat(MAX_RESPONSE + 1)).is_err());
    }
    #[test]
    fn deadline_and_cancellation_do_not_need_a_bus() {
        let error = zbus::block_on(bounded(
            std::future::pending::<()>(),
            Duration::from_millis(20),
            &mut |_| 0,
        ))
        .unwrap_err();
        assert!(error.message.contains("Zeitlimit"));
        let mut calls = 0;
        let error = zbus::block_on(bounded(
            std::future::pending::<()>(),
            Duration::from_secs(5),
            &mut |_| {
                calls += 1;
                i32::from(calls >= 2)
            },
        ))
        .unwrap_err();
        assert!(error.cancelled);
    }
}
