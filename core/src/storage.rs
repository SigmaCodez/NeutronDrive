//! Descriptor-relative private state I/O with atomic, durable replacement.
use crate::process::Failure;
use serde_json::Value;
use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

type Result<T> = std::result::Result<T, Failure>;
use libc::{
    getuid, mkdirat, openat, renameat, unlinkat, O_CLOEXEC, O_CREAT, O_DIRECTORY, O_EXCL,
    O_NOFOLLOW, O_NONBLOCK, O_WRONLY,
};
const DIRECTORY: i32 = O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC;

fn component(value: &std::ffi::OsStr) -> Result<CString> {
    if value.as_bytes().is_empty()
        || matches!(value.as_bytes(), b"." | b"..")
        || value.as_bytes().contains(&b'/')
    {
        return Err("Ungültiger privater Dateiname.".into());
    }
    CString::new(value.as_bytes()).map_err(|_| "Ungültiger privater Dateiname.".into())
}

pub fn directory(path: &Path, create: bool, private: bool) -> Result<File> {
    if !path.is_absolute()
        || path
            .as_os_str()
            .as_bytes()
            .split(|c| *c == b'/')
            .any(|s| s == b".." || s == b".")
    {
        return Err("Absolutes Dienstverzeichnis ohne Punktkomponenten erforderlich.".into());
    }
    let mut current =
        File::open("/").map_err(|_| "Dienstverzeichnis ist nicht sicher zugänglich.")?;
    for part in path.components() {
        if matches!(part, Component::RootDir) {
            continue;
        }
        let name = component(part.as_os_str())?;
        // SAFETY: parent fd and NUL-terminated name live through each syscall.
        let mut fd = unsafe { openat(current.as_raw_fd(), name.as_ptr(), DIRECTORY) };
        if fd < 0
            && create
            && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound
        {
            let result = unsafe { mkdirat(current.as_raw_fd(), name.as_ptr(), 0o700) };
            if result < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err("Dienstverzeichnis konnte nicht angelegt werden.".into());
            }
            fd = unsafe { openat(current.as_raw_fd(), name.as_ptr(), DIRECTORY) };
        }
        if fd < 0 {
            return Err(
                "Dienstverzeichnis ist nicht sicher zugänglich; Symlinks sind nicht erlaubt."
                    .into(),
            );
        }
        current = unsafe { File::from_raw_fd(fd) };
    }
    let info = current
        .metadata()
        .map_err(|_| "Dienstverzeichnis ist nicht lesbar.")?;
    if private && (info.uid() != unsafe { getuid() } || info.mode() & 0o077 != 0) {
        return Err(
            "Das Dienstverzeichnis muss Ihnen gehören und Zugriffsmodus 0700 haben.".into(),
        );
    }
    Ok(current)
}

pub fn read(path: &Path, default: Value, limit: usize, private: bool) -> Result<Value> {
    Ok(read_optional(path, limit, private)?.unwrap_or(default))
}

pub fn read_optional(path: &Path, limit: usize, private: bool) -> Result<Option<Value>> {
    if limit == 0 || limit > 64 * 1024 * 1024 {
        return Err("Ungültige Zustandsgrößengrenze.".into());
    }
    let parent = directory(
        path.parent().ok_or("Ungültiger Zustandspfad.")?,
        false,
        false,
    )?;
    let name = component(path.file_name().ok_or("Ungültiger Zustandspfad.")?)?;
    let fd = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK,
        )
    };
    if fd < 0 {
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err("Private Dienstdatei ist nicht sicher lesbar.".into());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let info = file
        .metadata()
        .map_err(|_| "Ungültige private Dienstdatei.")?;
    if !info.is_file()
        || info.uid() != unsafe { getuid() }
        || info.len() > limit as u64
        || (private && info.mode() & 0o077 != 0)
    {
        return Err("Ungültige private Dienstdatei.".into());
    }
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Private Dienstdatei ist nicht vollständig lesbar.")?;
    if bytes.len() > limit {
        return Err("Private Dienstdatei überschreitet die Größenbegrenzung.".into());
    }
    serde_json::from_slice::<Value>(&bytes)
        .map(Some)
        .map_err(|_| "Eine Dienstdatei ist beschädigt; sie wird nicht überschrieben.".into())
}

struct Temporary<'a> {
    parent: &'a File,
    name: CString,
}
impl Drop for Temporary<'_> {
    fn drop(&mut self) {
        // The random name is relative to the pinned parent. Unlinking it after
        // successful rename is harmless; failure leaves no staging file behind.
        unsafe {
            unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), 0);
        }
    }
}

pub fn write(path: &Path, value: Value) -> Result<()> {
    let parent = directory(
        path.parent().ok_or("Ungültiger Zustandspfad.")?,
        false,
        true,
    )?;
    let target = component(path.file_name().ok_or("Ungültiger Zustandspfad.")?)?;
    let mut random = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut random))
        .map_err(|_| "Zufälliger Zustandsdateiname konnte nicht erzeugt werden.")?;
    let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let name = CString::new(format!(".state-{suffix}.tmp")).expect("fixed ASCII");
    let fd = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
            0o600u32,
        )
    };
    if fd < 0 {
        return Err("Private Zustandsdatei konnte nicht angelegt werden.".into());
    }
    let temporary = Temporary {
        parent: &parent,
        name,
    };
    let mut file = unsafe { File::from_raw_fd(fd) };
    let mut bytes = serde_json::to_vec(&value).map_err(|_| "Ungültiger privater Zustand.")?;
    bytes.push(b'\n');
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Private Zustandsdatei konnte nicht dauerhaft gespeichert werden.")?;
    if unsafe {
        renameat(
            parent.as_raw_fd(),
            temporary.name.as_ptr(),
            parent.as_raw_fd(),
            target.as_ptr(),
        )
    } < 0
    {
        return Err("Private Zustandsdatei konnte nicht atomar ersetzt werden.".into());
    }
    parent
        .sync_all()
        .map_err(|_| "Private Zustandsänderung konnte nicht dauerhaft bestätigt werden.")?;
    let current = directory(
        path.parent().ok_or("Ungültiger Zustandspfad.")?,
        false,
        true,
    )?;
    let before = parent
        .metadata()
        .map_err(|_| "Dienstverzeichnis wurde verändert.")?;
    let after = current
        .metadata()
        .map_err(|_| "Dienstverzeichnis wurde verändert.")?;
    if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
        return Err(
            "Dienstverzeichnis wurde während der Speicherung ersetzt; Zustand nicht bestätigt."
                .into(),
        );
    }
    Ok(())
}
