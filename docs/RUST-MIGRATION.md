# Incremental Rust migration — stages 1–3 (0.4–0.6)

The application requires `libneutrondrive_core.so`. This release is not a full
Rust rewrite of the background process. The 0.6 interface is implemented in Qt Quick/QML with a PySide6 controller.

## Implemented in Rust

- Manual and automatic reconciliation planning, including type/content conflicts,
  baseline identity comparisons, moved identity detection and pending trash intents.
- File deletion eligibility and whole-folder deletion blocking.
- Post-transfer convergence checks and baseline advancement/pruning decisions.
- Background debounce, retry/backoff, job selection and local echo decisions.
- SDK batch/schema/scope/cursor validation, ordered event delivery planning,
  reconnect/reset announcements and event-source retry timing (new in 0.5).

- Local inotify batch decoding and event classification (new in 0.6). The core
  validates the complete read before any host-side watcher changes, preserves
  non-UTF-8 Linux names as bytes, and classifies overflow, ignored watches,
  recursive registration and stale-path rebuilds. Malformed reads stop the watcher
  with a recoverable error instead of applying partial events.

- Remote metadata-cache invalidation planning (0.6.0 alpha 2): old/new parents,
  moved folder descendants, root aliases and conservative unknown-event/reset
  fallback. Traversal is iterative and cycle-safe. Python retains the cache,
  lock and generation guard; a failed core decision clears stale listings.

- Daemon cloud-event routing (0.6.0 alpha 4): matching events to job/root/node
  identities, ready/reset/error handling, unknown-parent lookup eligibility,
  duplicate suppression and bounded queue overflow decisions. The host applies
  the complete decision under its existing lock and persists before returning
  the event acknowledgement. Invalid requests fail before any host mutation.

## Language choice

No additional C implementation is introduced in this step. The existing C ABI
already connects the Rust core to Python. Another C layer would add ownership
and error-handling obligations without a measured benefit for these decisions.
Qt and Python's hashing primitives already execute native library code. There is
no comparative performance claim: this step centralizes policy in Rust while
retaining tested I/O adapters. Future native I/O migration needs dedicated
filesystem race, cancellation and crash-recovery validation.

## Retained adapters

Python retains daemon lifecycle, locks, Unix control socket, job persistence,
inotify/SDK event ingestion, filesystem snapshots, guarded transfer execution,
local trash receipts and restore, and the FUSE/Qt integrations. The pinned
Proton SDK bridge still owns authentication, cryptography and remote API access.
Existing baseline v2, jobs and pending operation formats are unchanged.

The library exchanges versioned JSON observations/decisions over a narrow C ABI.
It receives no credentials and performs no I/O. Rust explicitly requests fresh
remote absence observations; the host checks those before requesting a final
plan. Existing stale-plan and immediately-before-mutation checks remain active.
An absent/incompatible core stops operations with a diagnostic; there is no
silent Python fallback. Owned replies are freed exactly once; panics are caught
at the ABI boundary. Inputs are bounded to 64 MiB and large integer timestamps
and inode identities retain their precision.

The process status and `--diagnose` expose core version/components. Source builds
use the pinned toolchain, Cargo.lock and checked-in vendor sources offline.
Packages include the library and compiler/source/binary provenance plus notices.

## Validation strategy and remaining work

The released 0.3 planning policy is frozen only under tests as a differential
oracle. Seeded scenarios compare decisions directly, alongside existing regression
suites, concurrent ABI calls, installed-package checks and synthetic account tests.
See [measured results](VALIDATION.md). This is not a security audit or a claim of
improved performance; no comparative performance benchmark has been completed.

Later migration stages can move daemon orchestration and guarded I/O into Rust,
retaining the control protocol and disk formats. They require separate crash,
interruption and filesystem race validation. This release does not claim these
future stages are complete.

The pinned SDK still prevents complete enumeration of directly trashed folders.
Whole-folder deletion therefore remains an explicit conflict; individual file
trash/restore is supported. Flatpak still needs the native online-drive helper;
strict Snap still cannot mount FUSE. Rust does not remove these platform limits.
