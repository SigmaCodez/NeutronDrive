# Validation record — 0.3.0 alpha1

Date: 2026-10-20. This records observed results, not universal compatibility.
Previous release evidence: [0.2](VALIDATION-0.2.md), [0.1](VALIDATION-0.1.md).

## Local validation

All **275 Python tests passed without skips** on Ubuntu 26.04.1 x86_64 /
Python 3.14.4, with GTK, isolated D-Bus and actual kernel FUSE tests enabled.
The final run includes 69 sync tests, 40 background tests, 35 local trash tests
and 12 GTK tests, plus eight live-harness regression tests. Desktop/AppStream metadata validation and Python compilation
also passed. The unchanged bridge's nine standalone event protocol tests passed.

Deletion coverage includes v1-to-v2 migration, files in both directions,
file deletion inside retained nested folders, early whole-folder conflicts,
retained old pending-folder journals, delete-versus-edit/type/inode/UID conflicts,
in-tree and out-of-tree cloud moves, stale previews, missing/trashed ancestors,
ambiguous acknowledgements, persistent retries and restart without resurrection.
Local trash tests cover no-follow traversal, same-filesystem atomic moves,
non-overwriting restore, fsync/journal interruption, source and parent races,
changed payloads, lock deadlines, cancellation and recovery retries. A GTK test
performs an actual isolated restore collision followed by a safe retry.

The background, sync and trash implementation received a second code review.
It found and corrected final proof ordering, nested-directory durability and
detectable target and ancestor moves during preflight and pending recovery.
The account test also exposed the public SDK trashTime/event isTrashed distinction;
central normalization and public-node-format regression tests cover that contract.
A read-only account probe then confirmed that directly trashed folders cannot be
enumerated through the pinned public SDK. The production engine now blocks whole-
folder deletion before mutation; it does not claim a complete trash subtree proof. The complete suite
was rerun after the final correction. This is not an independent security audit.

## Account and package validation

The authorized synthetic account test and final package matrix are in progress.
Final measured results are added before release. An earlier account run was
intentionally interrupted to reload the final safety correction; its daemon
stopped and its UUID-scoped synthetic cloud folder was moved to trash.


The unchanged source-built bridge is `bridge-v0.2.0-alpha1`, pinned in
`packaging/backend-lock.json`. Its previous source/typecheck and live FUSE evidence
remains in the 0.2 record; it is not represented as a new backend build.

Strict Snap does not support the unprivileged online-drive mount. Flatpak online
files require the native helper. Full installed-account/autostart tests across
GNOME/KDE, every distribution and architecture, long-duration/large-tree tests
and an independent security audit remain outside the demonstrated coverage.
No error-free operation or complete Windows/macOS feature parity is claimed.
