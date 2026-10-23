//! XDG/Snap autostart entries and Desktop Exec escaping; no shell execution.
use crate::{process::Failure, storage};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, Failure>;
const APP_ID: &str = "io.github.SigmaCodez.NeutronDrive";
const MARKER: &str = "X-NeutronDrive-Managed=true";
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Path,
    Command {
        arguments: Vec<String>,
        source_launcher: PathBuf,
        interpreter: String,
    },
    Quote {
        argument: String,
    },
    Read {
        path: PathBuf,
    },
    Enabled {
        path: PathBuf,
    },
    Set {
        path: PathBuf,
        enabled: bool,
        command: Vec<String>,
    },
}
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}
fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        std::env::current_dir()
            .map(|p| p.join(path))
            .map_err(|_| "Arbeitsverzeichnis ist nicht erreichbar.".into())
    }
}
fn path() -> Result<PathBuf> {
    let base = if env("SNAP").is_some() {
        PathBuf::from(env("SNAP_USER_DATA").ok_or("SNAP_USER_DATA fehlt für den Snap-Autostart.")?)
            .join(".config")
    } else if let Some(config) = env("XDG_CONFIG_HOME") {
        PathBuf::from(config)
    } else {
        crate::mount_manager::current_user_home()?.join(".config")
    };
    absolute(&base.join("autostart").join(format!("{APP_ID}.desktop")))
}
fn command(arguments: Vec<String>, source: &Path, interpreter: String) -> Result<Vec<String>> {
    let mut command = if env("FLATPAK_ID").is_some() {
        vec!["flatpak".into(), "run".into(), APP_ID.into()]
    } else if env("SNAP").is_some() {
        vec!["neutrondrive".into()]
    } else {
        let found = std::env::var_os("PATH").and_then(|v| {
            std::env::split_paths(&v).find_map(|dir| {
                let path = dir.join("neutrondrive");
                let c = CString::new(path.as_os_str().as_bytes()).ok()?;
                (path.is_file() && unsafe { libc::access(c.as_ptr(), libc::X_OK) } == 0)
                    .then_some(path)
            })
        });
        if let Some(found) = found {
            vec![absolute(&found)?.to_string_lossy().into_owned()]
        } else if source.is_file() {
            vec![
                interpreter,
                absolute(source)?.to_string_lossy().into_owned(),
            ]
        } else {
            return Err("Kein installierter NeutronDrive-Starter gefunden.".into());
        }
    };
    command.extend(arguments);
    Ok(command)
}
fn quote(argument: &str) -> Result<String> {
    if argument.chars().any(|c| matches!(c, '\n' | '\r' | '\0')) {
        return Err("Ungültiges Zeichen im Autostart-Pfad.".into());
    }
    let escaped = argument
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    Ok(format!("\"{}\"", escaped.replace('\\', "\\\\")))
}
fn name(path: &Path) -> Result<CString> {
    CString::new(
        path.file_name()
            .ok_or("Ungültiger Autostart-Pfad.")?
            .as_bytes(),
    )
    .map_err(|_| "Ungültiger Autostart-Pfad.".into())
}
fn read_at(parent: &File, name: &std::ffi::CStr) -> Result<Option<(String, std::fs::Metadata)>> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err("Unsicherer Autostart-Eintrag.".into());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let before = file
        .metadata()
        .map_err(|_| "Unsicherer Autostart-Eintrag.")?;
    if !before.is_file()
        || before.uid() != unsafe { libc::getuid() }
        || before.nlink() != 1
        || before.len() > 65536
    {
        return Err("Unsicherer Autostart-Eintrag.".into());
    }
    let mut bytes = Vec::new();
    (&file)
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "Autostart-Eintrag ist nicht lesbar.")?;
    let after = file
        .metadata()
        .map_err(|_| "Autostart-Eintrag ist nicht lesbar.")?;
    if bytes.len() > 65536
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err("Autostart-Eintrag wurde während des Lesens geändert.".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "Autostart-Eintrag ist nicht lesbar.")?;
    Ok(Some((text, after)))
}
fn read(path: &Path) -> Result<Option<String>> {
    let path = absolute(path)?;
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err("Der Autostart-Pfad enthält einen symbolischen Link.".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Autostart-Pfad ist nicht erreichbar.".into()),
        }
    }
    if !path.exists() {
        return Ok(None);
    }
    let parent = storage::directory(
        path.parent().ok_or("Ungültiger Autostart-Pfad.")?,
        false,
        false,
    )?;
    read_at(&parent, &name(&path)?).map(|v| v.map(|(s, _)| s))
}
fn same(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}
fn set(path: &Path, enabled: bool, command: Vec<String>) -> Result<Value> {
    if env("FLATPAK_ID").is_some() {
        return Err("Flatpak-Autostart wird über das Desktop-Portal eingerichtet.".into());
    }
    let path = absolute(path)?;
    // Validate ancestors even when disabling an absent entry.
    let previous = read(&path)?;
    if previous
        .as_ref()
        .is_some_and(|v| !v.lines().any(|s| s == MARKER))
    {
        return Err("Ein bestehender fremder Autostart-Eintrag wird nicht überschrieben.".into());
    }
    if !enabled && previous.is_none() {
        return Ok(Value::Null);
    }
    let parent_path = path.parent().ok_or("Ungültiger Autostart-Pfad.")?;
    let parent = storage::directory(parent_path, enabled, false)?;
    let target = name(&path)?;
    let current = read_at(&parent, &target)?;
    if current.as_ref().map(|v| &v.0) != previous.as_ref() {
        return Err("Autostart-Eintrag wurde geändert.".into());
    }
    if !enabled {
        if unsafe { libc::unlinkat(parent.as_raw_fd(), target.as_ptr(), 0) } < 0 {
            return Err("Autostart-Eintrag konnte nicht entfernt werden.".into());
        }
        parent
            .sync_all()
            .map_err(|_| "Autostart-Änderung konnte nicht gespeichert werden.")?;
        return Ok(Value::Null);
    }
    if command.is_empty() {
        return Err("Kein installierter NeutronDrive-Starter gefunden.".into());
    }
    let exec = command
        .iter()
        .map(|s| quote(s))
        .collect::<Result<Vec<_>>>()?
        .join(" ");
    let content=format!("[Desktop Entry]\nType=Application\nName=NeutronDrive Hintergrunddienst\nComment=Gespeicherte Synchronisierung und Online-Laufwerke starten\nExec={exec}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n{MARKER}\n");
    if content.len() > 65536 {
        return Err("Autostart-Eintrag ist zu groß.".into());
    }
    let mut random = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut random))
        .map_err(|_| "Autostart-Datei konnte nicht vorbereitet werden.")?;
    let temporary = CString::new(format!(
        ".neutrondrive-{}.desktop.tmp",
        random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
    .unwrap();
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            temporary.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err("Autostart-Datei konnte nicht angelegt werden.".into());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    let result = (|| {
        file.write_all(content.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|_| "Autostart-Eintrag konnte nicht geschrieben werden.")?;
        let latest = read_at(&parent, &target)?;
        if match (&current, &latest) {
            (None, None) => false,
            (Some((text, a)), Some((new, b))) => text != new || !same(a, b),
            _ => true,
        } {
            return Err(Failure::from("Autostart-Eintrag wurde geändert."));
        }
        let flags = if current.is_none() {
            libc::RENAME_NOREPLACE
        } else {
            0
        };
        if unsafe {
            libc::renameat2(
                parent.as_raw_fd(),
                temporary.as_ptr(),
                parent.as_raw_fd(),
                target.as_ptr(),
                flags,
            )
        } < 0
        {
            return Err("Autostart-Eintrag konnte nicht gespeichert werden.".into());
        }
        let named = storage::directory(parent_path, false, false)?;
        let a = parent
            .metadata()
            .map_err(|_| "Autostart-Verzeichnis ist nicht lesbar.")?;
        let b = named
            .metadata()
            .map_err(|_| "Autostart-Verzeichnis ist nicht lesbar.")?;
        if a.dev() != b.dev() || a.ino() != b.ino() {
            return Err("Autostart-Verzeichnis wurde geändert.".into());
        }
        parent
            .sync_all()
            .map_err(|_| Failure::from("Autostart-Änderung konnte nicht gespeichert werden."))?;
        Ok(Value::Null)
    })();
    unsafe {
        libc::unlinkat(parent.as_raw_fd(), temporary.as_ptr(), 0);
    }
    result
}
pub fn run(operation: Operation) -> Result<Value> {
    match operation {
        Operation::Path => Ok(json!(path()?)),
        Operation::Command {
            arguments,
            source_launcher,
            interpreter,
        } => Ok(json!(command(arguments, &source_launcher, interpreter)?)),
        Operation::Quote { argument } => Ok(json!(quote(&argument)?)),
        Operation::Read { path } => Ok(json!(read(&path)?)),
        Operation::Enabled { path } => Ok(json!(read(&path)
            .ok()
            .flatten()
            .is_some_and(|s| s.lines().any(|v| v == MARKER)))),
        Operation::Set {
            path,
            enabled,
            command,
        } => set(&path, enabled, command),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "nd-autostart-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn exec_arguments_escape_both_desktop_parsing_layers() {
        assert_eq!(
            quote("a $x%f`b\\c\"d").unwrap(),
            "\"a \\\\$x%%f\\\\`b\\\\\\\\c\\\\\"d\""
        );
        for s in ["a\nb", "a\rb", "a\0b"] {
            assert!(quote(s).is_err());
        }
    }
    #[test]
    fn roundtrip_managed_entry_and_preserve_foreign_content() {
        let tmp = Temp::new();
        let path = tmp.0.join("autostart/app.desktop");
        assert_eq!(read(&path).unwrap(), None);
        set(
            &path,
            true,
            vec!["/tmp/my app/$x%f`name".into(), "--background".into()],
        )
        .unwrap();
        let text = read(&path).unwrap().unwrap();
        assert!(text.lines().any(|l| l == MARKER));
        assert!(text.contains("\\\\$x%%f\\\\`name"));
        set(&path, true, vec!["/other".into()]).unwrap();
        assert!(read(&path).unwrap().unwrap().contains("Exec=\"/other\""));
        set(&path, false, vec![]).unwrap();
        assert!(!path.exists());
        std::fs::write(&path, "foreign").unwrap();
        for enabled in [true, false] {
            assert!(set(&path, enabled, vec!["/bin/true".into()]).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "foreign");
        }
    }
    #[test]
    fn links_hardlinks_and_oversized_entries_are_not_changed() {
        let tmp = Temp::new();
        let target = tmp.0.join("target");
        std::fs::write(&target, MARKER).unwrap();
        let path = tmp.0.join("app.desktop");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(set(&path, false, vec![]).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::hard_link(&target, &path).unwrap();
        assert!(set(&path, false, vec![]).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, vec![b'x'; 65537]).unwrap();
        assert!(set(&path, true, vec!["/bin/true".into()]).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), MARKER);
        let link = tmp.0.join("link");
        std::os::unix::fs::symlink(&tmp.0, &link).unwrap();
        assert!(set(&link.join("absent.desktop"), false, vec![]).is_err());
    }
}
