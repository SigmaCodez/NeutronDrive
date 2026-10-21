# Validation record — 0.6.0 alpha3

Previous evidence: [0.6.0 alpha2](VALIDATION-0.6-alpha2.md).

The GUI exposes automatic synchronization only; the separate manual page,
actions and controller fields were removed. Existing automatic jobs, Rust sync
policy, guarded execution and saved formats are unchanged.

The appearance follows the Linux Settings portal (`org.freedesktop.appearance`,
`color-scheme`) with Qt color-scheme/palette fallback. Reads are asynchronous,
changes update the running window, and stale initial replies cannot overwrite
newer notifications. No desktop setting is written. Six pages are rendered in
both light and dark mode, with a scrollable 900×640 layout.

Local regression: **304 tests passed without skips**, including 22 Qt UI tests
and a real private-D-Bus portal test covering nested initial variants, live
changes, reset, initial-read races and subscription cleanup. No real desktop
appearance preference is changed by these tests.

The 11 live-account checks from alpha2
remain evidence for the unchanged sync implementation; they are not presented
as a new account test for this UI-only update.

## CI and package checks

All **14 CI jobs passed** in [run 39630155787](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39630155787)
for source commit `7d47fdc6804a3fd80678fe443a6228518b3b91b8`. This includes
10 Rust tests and Clippy, Python 3.10–3.13, Qt Quick/X11 and the private appearance
portal on Ubuntu 22.04, Linux FUSE/D-Bus, installed DEB amd64/arm64 and
Fedora/openSUSE RPM tests, Flatpak and Snap builds.

The final Flatpak passed an isolated installation/profile test with all six GUI
pages, live light/dark switching, installed Python/QML/Qt/backend hashes, Rust
policy calls and synthetic local trash restoration. The temporary installation
was removed. No account login or real desktop preference change was performed.

All six final package payloads match the tested commit, including application
sources, selected Qt runtime, licenses, pinned backend and Rust provenance.
Downloads retain the original CI bytes; only filenames are normalized. The
release includes checksum/build/test evidence and six matching Qt/PySide/ICU
source archives. The tag targets the tested source commit; final documentation
is a later documentation-only commit on main.

## Limits

Linux appearance detection depends on a working Settings portal or Qt platform
integration; unknown preferences use Qt's palette. Not every GNOME/KDE/Wayland,
X11 or sandbox combination was tested on real hardware. Existing alpha, SDK,
whole-folder-deletion and FUSE/architecture limits from alpha2 remain unchanged.
