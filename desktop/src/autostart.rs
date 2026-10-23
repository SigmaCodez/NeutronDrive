//! Native UI autostart orchestration; never falls back to a Python launcher.
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
    sdk: &str,
    enabled: bool,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value, Failure> {
    let flatpak = std::env::var_os("FLATPAK_ID").is_some_and(|v| !v.is_empty());
    let snap = std::env::var_os("SNAP").is_some_and(|v| !v.is_empty());
    // APPIMAGE names the persistent outer executable, unlike current_exe in its temporary mount.
    let image = std::env::var_os("APPIMAGE")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from);
    let app = image.clone().or_else(|| std::env::current_exe().ok());
    execute(
        persistent_sdk(
            sdk,
            image.as_deref(),
            std::env::var_os("APPDIR").as_deref().map(Path::new),
        ),
        enabled,
        flatpak,
        snap,
        app.as_deref(),
        &mut |request| runtime::call(request, callback),
    )
}
// Only the bundled SDK disappears with an AppImage mount; preserve a custom SDK.
fn persistent_sdk<'a>(sdk: &'a str, image: Option<&Path>, appdir: Option<&Path>) -> &'a str {
    if image.is_some() && appdir.is_some_and(|root| Path::new(sdk).starts_with(root)) {
        ""
    } else {
        sdk
    }
}
fn execute(
    sdk: &str,
    enabled: bool,
    flatpak: bool,
    snap: bool,
    app: Option<&Path>,
    call: &mut impl FnMut(Value) -> Result<Value, Failure>,
) -> Result<Value, Failure> {
    if flatpak {
        if call(json!({"op":"portal","operation":{"action":"background","enabled":enabled}}))?
            != true
        {
            return Err(failure(
                "Das Desktop-Portal hat Autostart nicht freigegeben.",
            ));
        }
        return Ok(json!(enabled));
    }
    let mut command = Vec::new();
    if enabled {
        let launcher = if snap {
            "neutrondrive"
        } else {
            app.filter(|p| p.is_absolute())
                .and_then(Path::to_str)
                .ok_or_else(|| failure("Native application executable unavailable"))?
        };
        command.extend([json!(launcher), json!("--background")]);
        if !snap && !sdk.is_empty() {
            // A session PATH may differ on login. Resolve the selected SDK now.
            let sdk =
                std::path::absolute(sdk).map_err(|_| failure("Invalid backend executable path"))?;
            command.extend([json!("--cli"), json!(sdk)]);
        }
    }
    let path = call(json!({"op":"autostart","operation":{"action":"path"}}))?;
    call(
        json!({"op":"autostart","operation":{"action":"set","path":path,"enabled":enabled,"command":command}}),
    )?;
    Ok(json!(enabled))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn appimage_relocates_bundled_sdk_but_preserves_custom_backend() {
        let image = Some(Path::new("/apps/Drive.AppImage"));
        let dir = Some(Path::new("/tmp/.mount_drive"));
        assert_eq!(
            persistent_sdk("/tmp/.mount_drive/usr/bin/sdk", image, dir),
            ""
        );
        assert_eq!(persistent_sdk("/opt/sdk", image, dir), "/opt/sdk");
        assert_eq!(
            persistent_sdk("/tmp/.mount_drive-other/sdk", image, dir),
            "/tmp/.mount_drive-other/sdk"
        );
    }
    #[test]
    fn portal_denial_never_writes_entry() {
        for enabled in [true, false] {
            let mut requests = vec![];
            assert!(execute("sdk", enabled, true, false, None, &mut |request| {
                requests.push(request);
                Ok(json!(false))
            })
            .is_err());
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0]["operation"]["enabled"], enabled);
        }
    }
    #[test]
    fn persistent_launchers_and_disable_without_executable() {
        for (snap, enabled, expected) in [
            (
                false,
                true,
                json!([
                    "/apps/Neutron Drive.AppImage",
                    "--background",
                    "--cli",
                    "/sdk/proton-drive"
                ]),
            ),
            (true, true, json!(["neutrondrive", "--background"])),
            (false, false, json!([])),
        ] {
            let mut requests = vec![];
            let app = enabled.then(|| Path::new("/apps/Neutron Drive.AppImage"));
            assert_eq!(
                execute(
                    "/sdk/proton-drive",
                    enabled,
                    false,
                    snap,
                    app,
                    &mut |request| {
                        requests.push(request);
                        Ok(json!("/isolated/autostart.desktop"))
                    }
                )
                .unwrap(),
                enabled
            );
            assert_eq!(requests[1]["operation"]["command"], expected);
        }
    }
    #[test]
    fn write_failure_is_not_success() {
        assert!(execute("", false, false, false, None, &mut |_| Err(failure(
            "Permission denied"
        )))
        .is_err());
    }
}
