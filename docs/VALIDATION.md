# Validation record — 0.2.0 alpha1

Date: 2026-10-20. This records observed results, not universal compatibility.
The previous release evidence is preserved in [VALIDATION-0.1.md](VALIDATION-0.1.md).

## Local tests

The full Python suite, with opt-in GTK, isolated D-Bus and actual Linux kernel
FUSE tests enabled, passed **175 tests without skips** on Ubuntu 26.04.1 x86_64 /
Python 3.14.4. This includes nine GUI tests and seventeen desktop integration
tests. The nine GUI checks also passed after the startup-budget adjustment.
Both new pages were visually checked at 1280×720 with clearly labeled demo data;
the online page scrolls to its lower helper and recovery controls.

Coverage includes command serialization/cancellation, partial transfers,
SHA1/SHA256 verification, manual sync safety, inotify, event acknowledgement and
replay, scheduler pause/restart, account/root identity changes, remote metadata
cache invalidation, on-demand hydration, pins, eviction, failed-write recovery,
editor replacement, D-Bus activation and desktop autostart argument quoting.

The source-built backend passed 10 Bun tests and a complete TypeScript typecheck.
x64-baseline and arm64 ELF artifacts declare GLIBC symbols through 2.17. x64
uses Bun's baseline target without AVX2; this is not a claim of runtime validation
on every CPU. Provenance records the exact SDK commit, compiler version,
patch/source hashes and dependency-lock transformation.

## Authorized account tests

The existing browser-authenticated test session was used without exposing
passwords or session keys. Every mutation is confined to newly created,
uniquely named synthetic test folders. Cleanup moves these folders to trash;
nothing is permanently deleted.

`scripts/smoke-live` passed against the new bridge: Unicode upload/download,
checksum verification, rename/move, temporary public-link creation/revocation,
trash/restore, manual two-way changes, local backup retention and conflicts.

`scripts/smoke-background-live` exercises a real daemon subprocess, local inotify
uploads, external uploads followed by actual Drive-event downloads, pause/resume
and stop/restart. **All nine checks, including cleanup, passed** with the real
SDK event source; the daemon was confirmed stopped afterward.

`scripts/smoke-fuse-live` exercises real kernel mounts against
Proton, download on open, pin/offline remount, editing/revisions, editor replacement,
folder/file operations and eviction. **All seven checks passed**, followed by
confirmed cleanup to trash. Remote SHA1 and downloaded SHA256 content were
verified. Writes used separate application processes, as in normal desktop use.

An initial FUSE harness ran client I/O and the service in one process, causing
an inherited-file-descriptor deadlock when a backend child closed its FUSE fd.
The harness was corrected to use separate client processes, matching the real
D-Bus service architecture. Only that synthetic test connection was aborted.

## Packaging boundaries

Native DEB/RPM provide pyfuse3/Trio and the D-Bus helper. CI tests
actual FUSE mounts and D-Bus activation without cloud credentials, native package
installation/runtime dependencies, both DEB architectures and both RPM families.
Flatpak and strict Snap are built separately.

Flatpak online files additionally require the native host package/helper and may
require a separate native browser login. Full GNOME/KDE Background-portal and
installed-package account/restart combinations remain to be validated.
Strict Snap online mounts are explicitly disabled: its confinement does not
provide the required unprivileged FUSE helper. Browsing, transfer and background
folder sync are included; installed Snap account/autostart behavior still needs
verification on target desktops.

No claim of error-free operation, complete Windows/macOS parity, security audit,
all-distribution compatibility or Flathub/Snap Store publication is made.

## Final package run and release identity

All **13 jobs** in [GitHub Actions run 39492373435](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39492373435)
passed at build source `86f9a77360c70130eea7e65033f6d4bfe5b0d574`. Release packages
come from this run. The matrix tests Python 3.10–3.13, GTK 4.6 on Ubuntu 22.04,
real FUSE and isolated D-Bus, the event protocol and all six packages. GUI CI
uses a software renderer and a bounded 20-second cold-start budget; interactions
retain their shorter timeout.

| Package | Observed final result |
| --- | --- |
| DEB x86_64 / ARM64 | Built and installed on respective native Ubuntu 24.04 runners; backend, imports and all nine GTK checks passed |
| Fedora 44 RPM x86_64 | Pinned pyfuse3 source compiled; package installed; private extension imported; 36 on-demand tests passed |
| openSUSE Tumbleweed RPM x86_64 | Package installed using distro pyfuse3; imports and 36 on-demand tests passed |
| Flatpak x86_64 | Full SDK build; exact final bundle installed in isolation, backend diagnosed, GTK mapped on Wayland, helper D-Bus transport checked |
| Snap amd64 | Strict core24 package built and bundled backend reverified; installed-account/autostart flows remain untested |

The exact final Flatpak SHA256 is
`926c8e7573432c8b5f1c2c3f0dd88fedbb5342aa3da5aaa3e53174cc3873fb00`.
Its actual packaged permissions displayed a GTK 4.22.5 Wayland demo with six
entries, all six pages, both new scrollable pages and zero callback errors.
The sandbox successfully called a native source helper using a fake backend,
without added privileges or credential copying. The existing helper name was
unowned before testing; the test owner was verified and stopped afterward.
The temporary installation was removed. No new account login was performed.

The final source-built pyfuse3 3.4.2 binding additionally passed all five native
kernel tests locally. Fedora ships its complete source archive, LGPL notice and
replacement instructions alongside the extension.

Release `BUILD_INFO.json` records source/SDK identities, CI job results and all
package hashes; `TEST_RESULTS.json` records sanitized test evidence.
`BUILD_SOURCE_SHA256SUMS` maps original CI filenames, while `SHA256SUMS` verifies
the final download names. Renaming package files does not change their contents
or internal package versions.
