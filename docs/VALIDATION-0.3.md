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

All 11 checks in `scripts/smoke-deletions-live` passed using the authorized
account, the real background subprocess, kernel inotify and SDK events:

- Dedicated synthetic folder, event handshake and verified initial file/tree sync.
- Local file deletion to Proton trash; restart without resurrection; cloud restore
  downloads the file again.
- Remote file deletion to local trash; verified local restore uploads it again.
- Local whole-folder deletion produces an explicit conflict and leaves the full
  cloud subtree unchanged (UIDs, revisions, hashes, shape). Manually recreating
  the synthetic local folder restores conflict-free convergence.
- Remote whole-folder deletion leaves the local tree and inodes unchanged as a
  conflict. Restoring the cloud folder restores conflict-free convergence.
- Delete-versus-edit conflicts retain the concurrent edit in both directions.
- The test daemon stopped; the UUID-scoped cloud folder was moved to trash.
  No permanent cloud deletion or credential copying occurred.

The final run had zero busy-read retries and no backend errors. Earlier probes
exposed the SDK contract differences described above; they were not counted as
successful tests. Their synthetic cloud folders were also moved to trash.

The [final GitHub matrix](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39500108770)
passed **all 13 jobs** at source commit
`13dfe31e3d98674e8028332d45ebadc59e2cd15a`: Python 3.10–3.13, GTK 4.6 on
Ubuntu 22.04, real Linux FUSE/D-Bus, bridge protocol, DEB amd64/arm64, Fedora and
openSUSE x86_64 RPMs, Flatpak x86_64 and Snap amd64. DEB/RPM jobs install the
packages and run on-demand and local-trash tests with their runtime dependencies.

All six artifacts were downloaded from that run. Both DEBs, both RPMs and Snap
were inspected for the complete 13-module Python source set, version, exact
backend lock and backend SHA512. Fedora's bundled pyfuse3 source, LGPL notice and
Python ABI were also verified.

The final Flatpak was installed into a temporary installation with a separate
app-data mount namespace; the existing user profile was untouched. All 13
installed Python module hashes, backend lock and backend SHA512 matched the
build commit. Backend diagnosis, an actual GUI window with seven pages and six
labelled demo entries, and isolated local-trash/restore all passed without GUI
callback errors. The separate credential store was unauthenticated: no Flatpak
account login or new native-helper transport test is claimed here. Temporary
installation and profile were removed.

Flatpak SHA256: `710751d94eba80c6c4eff8111bedabacb74c51b9bec4ec491a61d1cd9961d1dc`.

Release assets include `BUILD_INFO.json`, `TEST_RESULTS.json`,
`BUILD_SOURCE_SHA256SUMS` and `SHA256SUMS`. Download names are normalized; the
six package payloads are byte-identical to their CI artifacts.


The unchanged source-built bridge is `bridge-v0.2.0-alpha1`, pinned in
`packaging/backend-lock.json`. Its previous source/typecheck and live FUSE evidence
remains in the 0.2 record; it is not represented as a new backend build.

Strict Snap does not support the unprivileged online-drive mount. Flatpak online
files require the native helper. Full installed-account/autostart tests across
GNOME/KDE, every distribution and architecture, long-duration/large-tree tests
and an independent security audit remain outside the demonstrated coverage.
No error-free operation or complete Windows/macOS feature parity is claimed.
