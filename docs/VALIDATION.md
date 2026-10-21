# Validation record — 0.6.0 alpha1

Date: 2026-10-21. Previous evidence: [0.5](VALIDATION-0.5.md).

The UI now uses Qt Quick/QML and a PySide6 controller. All seven pages and
asynchronous confirmations, file/folder selection, sharing and cancellation remain.
20 interaction tests pass with the selected packaged Qt runtime. All pages were
rendered and inspected, including a 900×640 window with scrolling.

The Rust core additionally decodes complete Linux inotify batches. Eight Rust
unit tests and Clippy pass. Four host-adapter tests include 500 seeded event
classification comparisons, non-UTF-8 names, watch removal and rejection of
partial batches before any filesystem-side effects.

All **296 local Python tests passed without skips**, with the selected QML
runtime, real kernel FUSE and isolated D-Bus enabled. The initial run found only
a missing Rust compiler path in the staging-test environment; rerunning with the
explicit prebuilt core passed. UI tests also exercise Qt Quick's file picker,
local URL conversion, rejection of stale dialog responses and keyboard selection
through model updates. A separate mouse test verifies that selecting a row
retains its delegate and that an actual double-click opens its folder. Package-version consistency is now a separate regression
check, after final artifact review detected stale DEB/Flatpak version strings.

All **11 live account checks passed** with the new Rust inotify decoder running
in the real background process: initial file/nested-folder convergence, local
file deletion and no resurrection after restart, cloud restore, remote deletion
and local trash restore, whole-folder conflict retention/recovery in both
directions, and delete-versus-edit conflicts. Zero busy-read retries; no daemon
error. The test daemon stopped and its UUID-scoped synthetic cloud folder was
moved to trash. Temporary local files were removed; no permanent deletion or
credential copying occurred.

All **14 jobs passed** in [CI run 39594497409](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39594497409)
for source commit `fb690f17afb614bc29b9b854fab41865d0b2ee15`. This includes
Python 3.10–3.13, Ubuntu 22.04 Qt Quick/X11, Linux FUSE/D-Bus, installed DEB
amd64/arm64 and Fedora/openSUSE RPM interaction tests, Flatpak and Snap builds.

The final Flatpak was installed in an isolated temporary installation/profile.
All seven QML pages mapped, Rust policy execution passed, and all 15 Python
modules plus Main.qml matched the source commit. All 398 selected Qt files
matched the pinned wheels. Synthetic local trash restore passed and temporary
installation/profile data was removed. The separate account store was
unauthenticated; this was not a Flatpak cloud-account test.

All six final CI artifacts passed verification. DEB/RPM/Snap contents matched
all application Python/QML sources, selected Qt wheel files and notices, backend
SHA512 pins, and Rust source/binary/ABI provenance. Flatpak's installed payload
was verified separately as above. Release filenames are normalized without
changing package bytes. Build/source hashes, Qt source archives and detailed
results accompany the downloads.

Python retains daemon lifecycle, guarded filesystem/SDK I/O, FUSE and QML bindings.
Whole-folder deletions remain protected conflicts; individual file trash/restore
is supported. Native ARM64 requires GLIBC 2.39; x86_64 requires GLIBC 2.34.
Strict Snap cannot mount FUSE; Flatpak needs the native online-drive helper.
No performance improvement, universal compatibility or full Windows/macOS parity
is claimed. This is an alpha, not an independent security audit.
