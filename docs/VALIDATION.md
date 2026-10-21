# Validation record — 0.5.0 alpha1

Date: 2026-10-21. This records observed tests, not universal compatibility or a
complete Rust rewrite. Previous evidence: [0.4](VALIDATION-0.4.md).

## Local and native validation

All **288 Python tests passed without skips**, including Qt, isolated D-Bus and
real kernel FUSE tests, on Ubuntu 26.04.1 / Python 3.14.4 x86_64. The UI suite
contains **17 Qt interaction tests**: all seven pages, navigation/search, safe
selection, demo guards, sync preview/busy state, trash collision/retry, service
status, dialog cancellation/acceptance, clipboard, worker completion on the GUI
thread and same-user window reactivation. Qt pages were rendered and inspected.

All **five Rust unit tests**, offline locked builds, formatting and Clippy with
warnings as errors passed. The retained migration tests compare 1,200 planner
scenarios and 600 scheduler cases and exercise 300 concurrent ABI calls. The
12 event-source tests now use Rust batch validation and delivery planning,
covering partial batches, cursor acknowledgement after callbacks/persistence,
replay after failure, cancellation, removal, reconnect and independent backend
cancellation. The unchanged bridge has nine passing protocol tests.

GTK widgets are no longer imported by the interface. PySide6/Qt 6.10.2 implements
it; Gio remains in D-Bus and desktop portal adapters. Rust additionally handles
SDK event validation, delivery/reset planning and retry timing. Python still
owns the event loop, persistence, guarded I/O, FUSE and Qt bindings.

## Real account validation

All **11 synthetic account checks passed** using the authorized account, real
background subprocess, inotify and SDK events. The status handshake verified
that the loaded Rust core includes event validation. Initial files/nested folders,
file deletion in both directions, restart without resurrection, cloud and local
trash restore, whole-folder conflict retention/recovery and delete-versus-edit
conflicts all passed. There were zero busy-read retries and no reported daemon
error. The daemon stopped, the UUID-scoped synthetic cloud folder moved to trash,
and temporary local files were cleaned up. No permanent deletion or credential
copying occurred.

## Package validation

All **14 jobs passed** in [CI run 39586818216](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39586818216)
for source commit `68bfc26848434436f110e8a6f5b23cf4b9aa13e9`, including
Python 3.10–3.13, Ubuntu 22.04 Qt/X11, installed DEB amd64/arm64, Fedora and
openSUSE RPM, Flatpak and Snap builds. Installed native packages passed the
17 Qt interaction tests.

The final Flatpak was installed into an isolated temporary installation/profile.
All seven Qt pages mapped successfully, Rust policy execution and all 15 Python
module hashes matched the build source, and all 175 selected Qt runtime files
matched the pinned wheels (Python bytecode caches excluded). The synthetic local
trash restore passed; the temporary installation/profile was removed. Its separate
account store was unauthenticated; this was not a Flatpak cloud-account test.

All six downloaded CI artifacts passed verification. The five native/Snap payloads
matched all application modules, selected Qt wheel files, Qt notices, backend
SHA512 lock and Rust binary/source/ABI provenance; Flatpak's installed payload
was verified separately as described above. Release filenames are normalized,
with package bytes unchanged. Build/source hashes and detailed test results are
included in the release downloads. Earlier package checks detected missing X11 test dependencies and
unused PDF/virtual-keyboard/TIFF plugins; the runtime selection was corrected.
The build now rejects any selected plugin requiring an omitted Qt library.

## Limits

The official x86_64 Qt wheels require GLIBC 2.34; ARM64 requires GLIBC 2.39, so
native ARM64 targets are Ubuntu 24.04+, Debian 13+ or equivalent. Python is
restricted to 3.10–3.14. Older ARM64 systems need a separately built Qt runtime.

Whole-folder deletion remains a protected conflict because the pinned SDK cannot
fully enumerate directly trashed folders. File deletion/restore remains supported.
Flatpak needs a native helper for FUSE; Strict-Snap cannot mount the online drive.
Portal/keyring combinations, installed Snap account/autostart and long-duration
large-tree workloads are not comprehensively tested. No independent security
audit, performance improvement or full Windows/macOS parity is claimed.
