//! Bootstrap before Qt, account access, or service initialization.
use std::{os::unix::process::CommandExt, path::PathBuf, process::Command};
pub fn run() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--appimage-bootstrap")) {
        return;
    }
    let result = (|| -> std::io::Result<()> {
        let source = PathBuf::from(
            args.next()
                .ok_or_else(|| std::io::Error::other("Missing AppImage directory"))?,
        )
        .canonicalize()?;
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|v| PathBuf::from(v).join(".cache"))
            })
            .ok_or_else(|| std::io::Error::other("No user cache directory is available"))?;
        if !cache.is_absolute() {
            return Err(std::io::Error::other(
                "AppImage cache directory must be absolute",
            ));
        }
        let target =
            neutrondrive_core::appimage::prepare(&source, &cache.join("neutrondrive-appimage"))?
                .canonicalize()?;
        let executable = if target == source {
            std::env::current_exe()?
        } else {
            target.join("AppRun")
        };
        Err(Command::new(executable).args(args).exec())
    })();
    if let Err(error) = result {
        eprintln!("AppImage startup failed: {error}");
    }
    std::process::exit(1);
}
