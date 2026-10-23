//! Python-free runtime entry point for native integrations. Input is one bounded
//! versioned JSON request on stdin; stdout is NDJSON events followed by a reply.
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static CANCELLED: AtomicBool = AtomicBool::new(false);
extern "C" fn cancel(_: libc::c_int) {
    CANCELLED.store(true, Ordering::Relaxed);
}

fn main() {
    if std::env::args().skip(1).eq(["--version"]) {
        println!("NeutronDrive runtime {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if std::env::args().len() != 1 {
        eprintln!("Usage: neutrondrive-runtime [--version]\nPass one versioned JSON runtime request on stdin.");
        std::process::exit(2);
    }
    // SAFETY: handler only stores a lock-free atomic flag and has static lifetime.
    unsafe {
        libc::signal(libc::SIGINT, cancel as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, cancel as *const () as libc::sighandler_t);
    }
    let mut bytes = Vec::new();
    if io::stdin()
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        eprintln!("Cannot read runtime request");
        std::process::exit(2);
    }
    let mut output = io::stdout().lock();
    let result = neutrondrive_core::runtime_request(&bytes, &mut |event| {
        if CANCELLED.load(Ordering::Relaxed) {
            return 1;
        }
        if event["kind"] != "poll"
            && (writeln!(output, "{}", serde_json::json!({"event":event})).is_err()
                || output.flush().is_err())
        {
            return 1;
        }
        0
    });
    let success = result["ok"] == true;
    if writeln!(output, "{result}").is_err() || output.flush().is_err() || !success {
        std::process::exit(1);
    }
}
