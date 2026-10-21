# Validation record — 0.6.0 alpha2

Previous release evidence: [0.6.0 alpha1](VALIDATION-0.6-alpha1.md).

This is a small prerelease increment, not a new minor release. The interface
retains Qt Quick/QML, with a light palette, grouped navigation, original line
icons and the existing seven pages and actions.

Rust now plans remote metadata-cache invalidation: old/new parents, descendants,
root aliases, unknown-event fallback and resets. Python retains locking, cache
storage and generation checks. A core error discards stale listings before it
propagates. The pure policy is compared against the previous implementation on
500 seeded graphs, including cycles, aliases and unrelated subtrees.

The native-library build now replaces the output inode atomically, preserving
libraries already mapped by running applications. A mapping regression test
covers this after a build/test overlap caused an initial local test abort.

## Completed local and CI checks

All **301 local Python tests passed without skips**, with Qt Quick, kernel FUSE
and isolated D-Bus enabled. The suite includes 20 UI interaction tests, 500 new
cache-policy comparison scenarios and the mapped-library rebuild regression.
All seven pages were rendered and inspected; the 900×640 layout remains scrollable.
All **10 native Rust tests** and Clippy passed using locked, offline dependencies.

All **14 jobs passed** in [CI run 39622740062](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39622740062)
for source commit `1b5b177e2c403fb8ee37f9536f372168c0a1fbb2`:
Python 3.10–3.13, Qt Quick/X11 on Ubuntu 22.04, Linux FUSE/D-Bus, SDK bridge,
DEB amd64/arm64, Fedora/openSUSE RPM, Flatpak and Snap.

## Live account and package evidence

All **11 live account checks passed**, including initial/nested convergence,
local file deletion with no resurrection after restart, cloud restore, remote
file deletion with local trash restore, protected whole-folder deletions and
recovery in both directions, and delete-versus-edit conflicts. The daemon stopped;
the dedicated UUID-scoped synthetic folder was moved to trash. No permanent cloud
deletion or credential copying occurred.

All six downloaded packages match the tested source commit. DEB/RPM/Snap Python
and QML payloads, selected Qt runtime, licenses, pinned backend and Rust source/
binary provenance were checked. The Flatpak additionally passed an isolated
installation with a private profile, installed-source hashes, execution of the
new Rust invalidation policy, all seven GUI pages and synthetic local trash
restoration. Its temporary installation/profile was removed. No Flatpak account
login was attempted; the live account test uses the native source environment.

The release includes the original six CI package payloads (download filenames
normalized only), checksums, build/test evidence and six matching Qt/PySide/ICU
source archives. Release tag targets the tested source commit; the final
validation documentation is a later documentation-only commit on main.

## Remaining limits

This is incremental migration: Python still owns daemon lifecycle, guarded I/O,
Qt/FUSE adapters and cache storage. It is not a full Rust rewrite or a measured
performance improvement. Whole-folder deletions remain protected conflicts due
to the pinned SDK limitation. Strict Snap cannot mount FUSE; Flatpak online files
need the native helper. ARM64 Qt requires GLIBC 2.39, x86_64 GLIBC 2.34. Not every
distribution/desktop/account integration was tested. Long-running large-tree
workloads, complete Windows/macOS parity and an independent audit remain pending.
