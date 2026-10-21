# Rust migration — implementation status

The application requires `libneutrondrive_core.so`. **The complete Rust migration
is still in progress.** The current interface uses Qt Quick/QML with a PySide6
controller. Do not describe this branch as a Python-free application.

## Native runtime work after 0.6.0 alpha 4

- Rust owns backend child processes, session/process-group cleanup, both output
  pipes, cancellation, timeouts, login-URL validation and the interprocess lock.
  Callback exceptions are propagated only after the child has been terminated
  and reaped. SDK output and credential-bearing errors are never logged.
- SDK node/trash metadata, JSON result validation, upload path checks and upload
  tree traversal now execute in Rust. Python retains small method/data bindings.
- Private state reads and atomic writes now execute in Rust, including file and
  directory fsync, non-following directory traversal, bounded reads and explicit
  distinction between a missing file and a stored JSON null. Event cursors still
  advance only after consumer callbacks and durable persistence succeed.
- Job configuration transactions execute in Rust: process locks, schema checks,
  local/remote overlap rejection, mount exclusions, immutable roots and account
  identity pins. Existing version-1 job JSON is retained.
- Rust owns inotify registration, recursive discovery, polling and descriptor
  cleanup. Opaque handles retain ownership during concurrent stop/callback calls;
  non-UTF-8 filenames cross the callback boundary as byte arrays.

These changes are a migration checkpoint, not completion of the requested full
rewrite. The SDK bridge remains the authority for authentication and encryption.

| Remaining component | Current implementation | Required completion work |
| --- | --- | --- |
| Daemon orchestration and control socket | Python | Native service lifecycle, IPC, cancellation and recovery |
| Transfer execution and filesystem snapshots | Python with Rust planning | Native guarded filesystem/SDK I/O with race and crash tests |
| Local trash and restore | Python | Native durable receipts, no-clobber restore and recovery |
| Online-drive cache and FUSE | Python | Native cache, file handles, offline/pin/evict and writeback |
| Desktop service and Qt controller | Python/PySide6/Gio | Native service/Qt bindings preserving QML and system appearance |
| Packaging | Python runtime remains required | Switch launcher/dependencies only after all runtime users are removed |

Completion requires the complete regression suite, real FUSE/D-Bus/UI tests,
synthetic account tests, baseline-compatible packages and installed-package
verification. Removing dependencies or inflating GitHub's Rust percentage does
not satisfy those gates.

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

Python retains daemon orchestration, its condition lock and Unix control socket,
SDK event-source orchestration, filesystem snapshots, guarded transfer execution,
local trash receipts/restore and FUSE/Qt integrations. Native runtime bindings
retain synchronous callback and exception conversion only where migrated. The pinned
Proton SDK bridge still owns authentication, cryptography and remote API access.
Existing baseline v2, jobs and pending operation formats are unchanged.

The library exchanges versioned JSON observations/decisions over a narrow C ABI.
The policy ABI performs no I/O. A separate backend ABI owns the native I/O
listed above and inherits only the configured SDK environment; it does not
read credential stores. Rust policy explicitly requests fresh
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

The remaining migration must move daemon orchestration and guarded transfer I/O into Rust,
retaining the control protocol and disk formats. They require separate crash,
interruption and filesystem race validation. This release does not claim these
future stages are complete.

The pinned SDK still prevents complete enumeration of directly trashed folders.
Whole-folder deletion therefore remains an explicit conflict; individual file
trash/restore is supported. Flatpak still needs the native online-drive helper;
strict Snap still cannot mount FUSE. Rust does not remove these platform limits.
