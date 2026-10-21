# Validation record — 0.6.0 alpha4

Previous evidence: [0.6.0 alpha3](VALIDATION-0.6-alpha3.md).

Daemon cloud-event routing now runs in Rust. Decisions include job relevance,
ready/reset/error handling, unknown-parent lookup eligibility, deduplication and
bounded queue overflow. The Python host retains the condition lock, applies the
returned decision, invalidates caches and persists status before returning the
ACK. Policy failure occurs before host state is changed.

The previous routing policy is frozen only as a test oracle. 1,500 seeded cases
compare multiple jobs, paused states, known/unknown node and parent identities,
root matches, reset/error/ready and queued duplicates. Separate tests exercise
the 1,024-entry limit, overflow, duplicate jobs, malformed identities and a Rust
failure before scheduling/persistence.

All **308 local Python tests passed without skips**, with Qt, real FUSE and an
isolated D-Bus enabled. All **12 native Rust tests** and Clippy passed. The
live-account test also passed **all 11 checks**, using the new native routing in
its real background process. Initial convergence, both file-deletion directions,
restart safety, cloud/local restoration, whole-folder conflicts and recovery,
and delete-versus-edit conflicts passed. The daemon stopped and its dedicated
UUID-scoped synthetic cloud folder was moved to trash. No permanent cloud deletion
or credential copying occurred.

The installed Flatpak passed isolated source/backend/Qt hash verification, a call
to the new Rust routing command, all six GUI pages, light/dark switching and
synthetic local trash restoration. Its temporary installation/profile was removed.

The initial Snap job encountered HTTP 504 while downloading the unchanged pinned
backend, before building. Only that failed job was retried; no integrity checks
were relaxed and no source change was needed.

No additional C layer was added: no measured performance benefit justified one.
Rust policy, the existing C ABI and guarded Python I/O remain the architecture.
No claim of full Rust migration or measured speed improvement is made.

All **14 CI jobs passed**, including the retried Snap job, for source commit
`e16307f076aec2c2f613568a66657acc05ac2a6d`:
[GitHub Actions run](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39633262903).
The six final package artifacts passed the package audit: DEB amd64/arm64,
Fedora/openSUSE RPM x86_64, Flatpak x86_64 and Snap amd64. Python/QML source,
native-core provenance, pinned backend and bundled Qt payloads were checked
against the build inputs; Flatpak payloads were checked through its separate
isolated installation. Release files preserve the CI package bytes and include
SHA-256 checksums, build/test evidence and matching Qt/PySide/ICU sources.

Remaining limits: Python still owns daemon lifecycle, guarded I/O and Qt/FUSE
adapters. Whole-folder deletion remains a protected conflict. Strict Snap lacks
FUSE online mounts; Flatpak online mounts need the native helper. ARM64 native
Qt requires GLIBC 2.39, x86_64 GLIBC 2.34. Installed Snap account/autostart,
every desktop/distribution combination and long-duration/large-tree workloads
were not revalidated. This remains an alpha, without an independent security
audit or a guarantee of universal Linux compatibility.
