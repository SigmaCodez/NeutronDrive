# Validation record — 0.6.0 alpha5

Previous evidence: [0.6.0 alpha4](VALIDATION-0.6-alpha4.md).

Release source: `e9ae23b05835df110ac13c74b689a5e363fd4936`.
All **15 CI jobs passed**: [final build](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39650290070).
Packages: DEB amd64/arm64, Fedora/openSUSE RPM x86_64, Flatpak x86_64 and Snap amd64.

Backend processes, private state I/O, job transactions and recursive inotify
runtime now run in Rust. Late deletion echoes after an empty snapshot are fixed.
Python remains for transfer/trash/FUSE orchestration and the Qt Quick controller;
this release does not complete the Rust migration.

All **332 local Python tests passed without skips** with Qt, D-Bus and real FUSE.
331 tests used local POSIX temporary storage; the package-staging test used the
project filesystem to avoid the temporary filesystem quota. An initial attempt
to run every test on the NAS was rejected by the private-directory permission
checks. No security checks were relaxed. An initial CI run exposed a test-only
procfs race when a terminated descendant disappeared between open and read;
the final test handles both ENOENT and ESRCH. The full final CI then passed.

16 Rust tests and Clippy passed, including native ARM64 CI. The identical runtime
before the version-only bump (`2a1c100bfa0a0569c9ebd1a081374171ca9a00ae`) also
passed 122 focused Python tests and all 11 synthetic live-account checks.
The live daemon stopped and its dedicated synthetic cloud folder was moved to
trash; no permanent cloud deletion or credential copying occurred.

The final installed Flatpak passed Python/QML, Rust, backend and all 398 bundled
Qt file checksums; native process execution, job storage and real inotify;
all six GUI pages, light/dark appearance and synthetic local trash restore.
Its temporary installation/profile was removed. All six package artifacts passed
source/provenance checks. All 16 published asset sizes and GitHub SHA256 digests
match the local release files; the release tag points to the tested source.

The amd64 DEB was also installed on Ubuntu 26.04.1. Package-manager status,
version 0.6.0a5, installed Python/QML and Rust hashes, native-core diagnostics and
GUI process startup were checked. No local sync jobs were configured and no
online drive was mounted; old background processes were stopped cleanly before
installation. Account configuration was preserved. No new account sign-in was
performed during this installation test.

Remaining limits: whole-folder deletion remains a protected conflict. Strict
Snap lacks FUSE online mounts; Flatpak online mounts need the native helper.
ARM64 native Qt needs GLIBC 2.39; x86_64 needs GLIBC 2.34. Long-duration workloads,
every Linux desktop/distribution combination and independent security auditing
remain outstanding. This remains an alpha.
