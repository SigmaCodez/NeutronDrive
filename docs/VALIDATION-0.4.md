# Validation record — 0.4.0 alpha1

Date: 2026-10-20–21. Results below apply to the first incremental Rust migration,
not to a complete Rust daemon or universal distribution compatibility.
Previous evidence: [0.3](VALIDATION-0.3.md), [0.2](VALIDATION-0.2.md), [0.1](VALIDATION-0.1.md).

## Local and differential validation

All **283 Python tests passed with zero skips** on Ubuntu 26.04.1 x86_64 /
Python 3.14.4 with GTK, isolated D-Bus and a real kernel FUSE mount enabled.
The retained suites include 69 sync, 40 background, 35 local-trash, 12 GTK and
eight live-harness regression tests. Eight new Rust integration tests include:

- 1,200 seeded sync snapshots compared with the frozen released 0.3 decision policy.
- 600 seeded scheduler comparisons and bounded retry cases.
- 300 concurrent native ABI requests with owned-response cleanup.
- Explicit remote absence proof requirements, large inode/nanosecond preservation,
  missing-library rejection, invalid protocol/path rejection and queue ordering.

Two native Rust ABI tests passed. `cargo fmt --check` and Clippy with warnings
as errors passed. Cargo builds/tests use the pinned toolchain, committed lockfile
and vendored sources offline. Nine unchanged bridge protocol tests, desktop and
AppStream validation also passed. This is not an independent security audit or
a comparative performance benchmark.

## Build and installed package validation

The [GitHub matrix](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39504613194)
passed **all 14 jobs** at source commit
`ff392d344b52b88e88cee66c4c3cce9d84df68c9`: native Rust build/test/lint,
Python 3.10–3.13, GTK 4.6 on Ubuntu 22.04, Linux FUSE/D-Bus, bridge protocol,
DEB amd64/arm64, Fedora/openSUSE RPM, Flatpak x86_64 and Snap amd64.
DEB/RPM jobs install their packages and run the Rust integration tests through
the installed Python/native-library paths, as well as existing runtime checks.

All six final artifacts were downloaded from that run. The native and Snap
payload audit compares every application module to the build commit, verifies
the pinned backend and checks the Rust ELF architecture, ABI, source/binary
provenance, Cargo.lock and dependency notices. The core's recorded maximum GLIBC
requirement is checked against the supported 2.35 ceiling. Fedora's bundled
pyfuse3 source, license, extension and Python ABI are also checked.

The final Flatpak was installed in a temporary installation with its own app-data
mount namespace. Installed Python sources, backend lock/binary and Rust core
provenance matched the build commit. Native Rust planning/retry calls and the
packaged diagnostic passed. All seven GTK pages mapped successfully; a synthetic
local file moved to trash and restored successfully. The temporary installation
and profile were removed; the existing user profile was untouched. The separate
Flatpak credential store was unauthenticated and no new browser login was attempted.

## Account validation

All **11 live checks passed** with the already authorized account, real background
subprocess, kernel inotify and SDK event stream. The daemon status explicitly
confirmed the Rust implementation before transfer tests. Checks covered initial
file/nested-folder sync, local deletion to cloud trash, restart without
resurrection, cloud restore to local, remote deletion to local trash, local
restore back to cloud, safe whole-folder conflicts in both directions, manual
folder recovery and delete-versus-edit conflicts preserving both edits.

There were zero busy-read retries and no reported daemon error. The test daemon
stopped; the newly created UUID-scoped synthetic cloud folder was moved to trash.
No permanent cloud deletion, credential copying or unrelated user-file mutation
was performed. Temporary local test artifacts were cleaned up.

## Scope and limitations

This release migrates planning, baseline/convergence, deletion policy and daemon
scheduling/echo decisions to Rust. Python retains lifecycle, locks, persistence,
event ingestion, guarded I/O, local trash, FUSE and GTK. See
[RUST-MIGRATION.md](RUST-MIGRATION.md). Baseline v2 and job formats remain compatible.

Whole-folder automatic deletion remains blocked before mutation because the
pinned SDK cannot enumerate directly trashed folders completely. Individual file
deletion/restore remains supported. There is no atomic conditional remote trash
operation; races require recoverable data and conflict handling. Flatpak online
files require the native helper; strict Snap cannot mount FUSE. Installed Snap
account/autostart, every GNOME/KDE portal/keyring combination, large-tree and
long-duration workloads have not all been revalidated. No error-free or complete
Windows/macOS parity claim is made.
