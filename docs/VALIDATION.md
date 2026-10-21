# Validation record — 0.6.0 alpha1

Date: 2026-10-21. Previous evidence: [0.5](VALIDATION-0.5.md).

The UI now uses Qt Quick/QML and a PySide6 controller. All seven pages and
asynchronous confirmations, file/folder selection, sharing and cancellation remain.
18 interaction tests pass with the selected packaged Qt runtime. All pages were
rendered and inspected, including a 900×640 window with scrolling.

The Rust core additionally decodes complete Linux inotify batches. Eight Rust
unit tests and Clippy pass. Four host-adapter tests include 500 seeded event
classification comparisons, non-UTF-8 names, watch removal and rejection of
partial batches before any filesystem-side effects.

Full regression, live account and final package validation are in progress.
Publication requires those checks to complete successfully.

Python retains daemon lifecycle, guarded filesystem/SDK I/O, FUSE and QML bindings.
Whole-folder deletions remain protected conflicts; individual file trash/restore
is supported. Native ARM64 requires GLIBC 2.39; x86_64 requires GLIBC 2.34.
Strict Snap cannot mount FUSE; Flatpak needs the native online-drive helper.
No performance improvement, universal compatibility or full Windows/macOS parity
is claimed. This is an alpha, not an independent security audit.
