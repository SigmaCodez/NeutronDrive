//! Authenticated, bounded Unix control-socket client shared by GUI and CLI.
use crate::process::Failure;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, net::UnixStream},
    },
    path::Path,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Failure>;
fn unreachable() -> Failure {
    "Hintergrunddienst ist nicht erreichbar.".into()
}
fn wait(stream: &UnixStream, events: i16, deadline: Instant) -> Result<()> {
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(unreachable)?;
        let millis = left.as_millis().saturating_add(1).min(i32::MAX as u128) as i32;
        let mut fd = libc::pollfd {
            fd: stream.as_raw_fd(),
            events,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut fd, 1, millis) };
        if result > 0 {
            return Ok(());
        }
        if result == 0 {
            return Err(unreachable());
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(unreachable());
        }
    }
}
pub fn command(path: &Path, action: &str, job_id: Option<&str>, timeout: f64) -> Result<Value> {
    if !timeout.is_finite() || timeout <= 0.0 || timeout > 300.0 {
        return Err("Invalid service timeout".into());
    }
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.is_empty() || bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(unreachable());
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (to, from) in address.sun_path.iter_mut().zip(bytes) {
        *to = *from as libc::c_char;
    }
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Err(unreachable());
    }
    let mut stream = unsafe { UnixStream::from_raw_fd(fd) };
    let length =
        (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1) as libc::socklen_t;
    if unsafe { libc::connect(fd, (&address as *const libc::sockaddr_un).cast(), length) } < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(unreachable());
        }
        wait(&stream, libc::POLLOUT, deadline)?;
        let mut error = 0i32;
        let mut len = std::mem::size_of_val(&error) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut error as *mut i32).cast(),
                &mut len,
            )
        } < 0
            || error != 0
        {
            return Err(unreachable());
        }
    }
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of_val(&credentials) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut len,
        )
    } < 0
        || len as usize != std::mem::size_of_val(&credentials)
    {
        return Err(unreachable());
    }
    if credentials.uid != unsafe { libc::getuid() } {
        return Err("Dienstsocket gehört einem anderen Benutzer.".into());
    }
    let mut payload =
        serde_json::to_vec(&json!({"action":action,"job_id":job_id})).map_err(|_| unreachable())?;
    payload.push(b'\n');
    let mut remaining = payload.as_slice();
    while !remaining.is_empty() {
        if Instant::now() >= deadline {
            return Err(unreachable());
        }
        match stream.write(remaining) {
            Ok(0) => return Err(unreachable()),
            Ok(n) => remaining = &remaining[n..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                wait(&stream, libc::POLLOUT, deadline)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => (),
            Err(_) => return Err(unreachable()),
        }
    }
    let mut data = Vec::new();
    let mut chunk = [0u8; 16384];
    loop {
        if Instant::now() >= deadline {
            return Err(unreachable());
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err("Unvollständige Dienstantwort.".into()),
            Ok(n) => {
                if data.len() + n > 8 * 1024 * 1024 {
                    return Err("Unvollständige Dienstantwort.".into());
                }
                let previous = data.len();
                data.extend_from_slice(&chunk[..n]);
                if let Some(offset) = chunk[..n].iter().position(|b| *b == b'\n') {
                    let end = previous + offset;
                    let response: Value =
                        serde_json::from_slice(&data[..end]).map_err(|_| unreachable())?;
                    if response["ok"] != true {
                        return Err(response["error"]
                            .as_str()
                            .unwrap_or("Dienstbefehl fehlgeschlagen.")
                            .into());
                    }
                    return response.get("result").cloned().ok_or_else(unreachable);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                wait(&stream, libc::POLLIN, deadline)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => (),
            Err(_) => return Err(unreachable()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        os::unix::net::UnixListener,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn server(
        reply: &'static [u8],
        delay: bool,
    ) -> (std::path::PathBuf, std::thread::JoinHandle<()>) {
        let path = std::env::temp_dir().join(format!(
            "nd-control-{}-{}.sock",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let listener = UnixListener::bind(&path).unwrap();
        let task = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&line).unwrap(),
                json!({"action":"status","job_id":null})
            );
            if delay {
                std::thread::sleep(Duration::from_millis(100));
            }
            for chunk in reply.chunks(3) {
                if stream.write_all(chunk).is_err() {
                    break;
                }
            }
        });
        (path, task)
    }
    #[test]
    fn fragmented_reply_and_errors() {
        for (reply, success) in [
            (
                b"{\"ok\":true,\"result\":{\"running\":true}}\n".as_slice(),
                true,
            ),
            (b"{\"ok\":true}\n", false),
            (b"[]\n", false),
            (b"{\"ok\":false,\"error\":\"blocked\"}\n", false),
            (b"incomplete", false),
        ] {
            let (path, task) = server(reply, false);
            let result = command(&path, "status", None, 2.0);
            assert_eq!(result.is_ok(), success);
            task.join().unwrap();
            std::fs::remove_file(path).unwrap();
        }
    }
    #[test]
    fn timeout_closes_connection() {
        let (path, task) = server(b"{\"ok\":true,\"result\":{}}\n", true);
        assert!(command(&path, "status", None, 0.03).is_err());
        task.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn invalid_timeouts_fail_without_connecting() {
        for timeout in [0.0, -1.0, f64::NAN, f64::INFINITY, 301.0] {
            assert!(command(Path::new("/missing"), "status", None, timeout).is_err());
        }
    }
}

// Borrow an accepted descriptor without changing its file status flags or taking
// ownership from the host. Only local stream sockets from this UID are accepted.
fn accepted(fd: i32) -> Result<UnixStream> {
    use std::os::fd::IntoRawFd;
    let owned = crate::filesystem::duplicate(fd)?;
    let stream = unsafe { UnixStream::from_raw_fd(owned.into_raw_fd()) };
    for (option, expected) in [
        (libc::SO_DOMAIN, libc::AF_UNIX),
        (libc::SO_TYPE, libc::SOCK_STREAM),
    ] {
        let mut value = 0i32;
        let mut length = std::mem::size_of_val(&value) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                option,
                (&mut value as *mut i32).cast(),
                &mut length,
            )
        } < 0
            || length as usize != std::mem::size_of_val(&value)
            || value != expected
        {
            return Err("Ungültiger lokaler Steuerungssocket.".into());
        }
    }
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of_val(&credentials) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    } < 0
        || length as usize != std::mem::size_of_val(&credentials)
        || credentials.uid != unsafe { libc::getuid() }
    {
        return Err("Dienstsocket gehört einem anderen Benutzer.".into());
    }
    Ok(stream)
}
fn server_deadline(timeout: f64) -> Result<Instant> {
    if !timeout.is_finite() || timeout <= 0.0 || timeout > 300.0 {
        return Err("Invalid service timeout".into());
    }
    Ok(Instant::now() + Duration::from_secs_f64(timeout))
}
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ControlRequest {
    action: String,
    job_id: Option<String>,
}
pub fn receive(fd: i32, timeout: f64) -> Result<Value> {
    let deadline = server_deadline(timeout)?;
    let stream = accepted(fd)?;
    let mut data = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        if Instant::now() >= deadline {
            return Err("Steuerungsanfrage hat das Zeitlimit überschritten.".into());
        }
        let count = unsafe {
            libc::recv(
                stream.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                libc::MSG_DONTWAIT,
            )
        };
        if count < 0 {
            match std::io::Error::last_os_error().kind() {
                std::io::ErrorKind::WouldBlock => wait(&stream, libc::POLLIN, deadline)?,
                std::io::ErrorKind::Interrupted => (),
                _ => return Err("Ungültige Steuerungsanfrage.".into()),
            }
            continue;
        }
        if count == 0 || data.len() + count as usize > 32768 {
            return Err("Ungültige IPC-Anfrage.".into());
        }
        let previous = data.len();
        data.extend_from_slice(&buffer[..count as usize]);
        if let Some(offset) = buffer[..count as usize].iter().position(|b| *b == b'\n') {
            let request: ControlRequest = serde_json::from_slice(&data[..previous + offset])
                .map_err(|_| "Ungültige Steuerungsanfrage.")?;
            return serde_json::to_value(request)
                .map_err(|_| "Ungültige Steuerungsanfrage.".into());
        }
    }
}
pub fn respond(fd: i32, response: Value, timeout: f64) -> Result<Value> {
    let deadline = server_deadline(timeout)?;
    let stream = accepted(fd)?;
    if !response.is_object() || !response["ok"].is_boolean() {
        return Err("Ungültige Dienstantwort.".into());
    }
    let mut payload = serde_json::to_vec(&response).map_err(|_| "Ungültige Dienstantwort.")?;
    payload.push(b'\n');
    if payload.len() > 8 * 1024 * 1024 {
        return Err("Dienstantwort überschreitet die sichere Nachrichtengröße.".into());
    }
    let mut remaining = payload.as_slice();
    while !remaining.is_empty() {
        if Instant::now() >= deadline {
            return Err("Dienstantwort hat das Zeitlimit überschritten.".into());
        }
        let count = unsafe {
            libc::send(
                stream.as_raw_fd(),
                remaining.as_ptr().cast(),
                remaining.len(),
                libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
            )
        };
        if count < 0 {
            match std::io::Error::last_os_error().kind() {
                std::io::ErrorKind::WouldBlock => wait(&stream, libc::POLLOUT, deadline)?,
                std::io::ErrorKind::Interrupted => (),
                _ => return Err("Dienstantwort konnte nicht gesendet werden.".into()),
            }
        } else if count == 0 {
            return Err("Unvollständige Dienstantwort.".into());
        } else {
            remaining = &remaining[count as usize..];
        }
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod server_tests {
    use super::*;
    #[test]
    fn native_exchange_preserves_borrowed_fd_and_flags() {
        let (mut peer, server) = UnixStream::pair().unwrap();
        let flags = unsafe { libc::fcntl(server.as_raw_fd(), libc::F_GETFL) };
        peer.write_all(b"{\"action\":\"status\"}\n").unwrap();
        assert_eq!(
            receive(server.as_raw_fd(), 1.0).unwrap(),
            json!({"action":"status","job_id":null})
        );
        respond(
            server.as_raw_fd(),
            json!({"ok":true,"result":{"running":true}}),
            1.0,
        )
        .unwrap();
        let mut reply = String::new();
        use std::io::BufRead;
        std::io::BufReader::new(&mut peer)
            .read_line(&mut reply)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&reply).unwrap()["result"]["running"],
            true
        );
        assert_eq!(
            unsafe { libc::fcntl(server.as_raw_fd(), libc::F_GETFL) },
            flags
        );
        peer.write_all(b"{\"action\":\"stop\",\"job_id\":\"job\"}\n")
            .unwrap();
        assert_eq!(receive(server.as_raw_fd(), 1.0).unwrap()["action"], "stop");
    }
    #[test]
    fn malformed_and_truncated_requests_fail_closed() {
        for bytes in [
            b"[]\n".as_slice(),
            b"{}\n",
            b"{\"action\":7}\n",
            b"{\"action\":\"status\",\"job_id\":[]}\n",
            b"{\"action\":\"status\",\"unexpected\":true}\n",
            b"\xff\n",
            b"{\"action\":\"status\"}",
        ] {
            let (mut peer, server) = UnixStream::pair().unwrap();
            peer.write_all(bytes).unwrap();
            peer.shutdown(std::net::Shutdown::Write).unwrap();
            assert!(receive(server.as_raw_fd(), 1.0).is_err());
        }
    }
    #[test]
    fn oversized_and_idle_requests_are_bounded() {
        let (mut peer, server) = UnixStream::pair().unwrap();
        peer.write_all(&vec![b'x'; 32769]).unwrap();
        assert!(receive(server.as_raw_fd(), 1.0).is_err());
        let (_peer, server) = UnixStream::pair().unwrap();
        assert!(receive(server.as_raw_fd(), 0.02).is_err());
        assert!(receive(server.as_raw_fd(), 0.0).is_err());
    }
    #[test]
    fn closed_peer_and_non_socket_are_safe() {
        let (peer, server) = UnixStream::pair().unwrap();
        drop(peer);
        assert!(respond(server.as_raw_fd(), json!({"ok":true,"result":{}}), 1.0).is_err());
        let file = std::fs::File::open("/dev/null").unwrap();
        assert!(receive(file.as_raw_fd(), 1.0).is_err());
        assert!(file.metadata().is_ok());
    }
    #[test]
    fn blocked_response_has_a_deadline() {
        let (_peer, server) = UnixStream::pair().unwrap();
        assert!(respond(
            server.as_raw_fd(),
            json!({"ok":true,"result":"x".repeat(1024*1024)}),
            0.02
        )
        .is_err());
    }
}
