# Rust migration — implementation status

**The complete Rust migration is still in progress.** The current source launcher
and package staging use the Rust Qt controller and Rust services with the existing
Qt Quick/QML interface. They no longer start Python or load the former C ABI
adapter. The former Python UI/controller modules have been deleted. Remaining
core compatibility adapters now live exclusively in `tests/core_contract`; no
Python modules remain under `src/`. Regression completion, CI recipes and fresh
portable package builds still require validation; do not describe the entire
migration as complete. Qt and the pinned Proton SDK remain dependencies.

The following sections are a chronological engineering log, including superseded
intermediate implementations. Portable artifacts recorded below must be rebuilt
after subsequent source edits; a successful historical build is not validation
of the current source tree.

## Current native sync runtime

Sync preview, remote/local proofs, baseline persistence and complete execution now
run in Rust. Execution owns the process lock, staged transfers, durable deletion
intent, recoverable local trash, post-transfer verification, convergence and
pending-deletion reconciliation. `src/neutrondrive/sync.py` is now a 207-line DTO/
backend adapter with caller-side preview ownership/cancellation binding; it no
longer implements filesystem transactions or the sync algorithm.

Sync-execution checkpoint verification: 84 Rust unit tests, 6 native executable integration tests,
Clippy with warnings denied and all 353 application tests without skips/failures.
The executable suite includes a full SDK-fixture download and replan without
Python. The full application run covers Qt, real kernel FUSE, isolated D-Bus and
package staging against `build/rust-native-execution-check`, built offline on the
isolated Ubuntu 22.04.5 baseline (maximum GLIBC 2.34). No new real-account test,
release or local installation was performed.

The production background daemon now runs in Rust, including shared state,
thread coordination, scheduling and durable event handling. The overall migration
remains incomplete: update controller/installer, Qt/PySide6
controllers and the remaining launcher/DTO bindings still need migration. Qt/QML and the pinned Proton SDK
remain dependencies. The online filesystem coordinator now also runs in Rust;
its Python module is only a DTO/byte binding to owned native mounts.
The sections below record earlier checkpoints and must not
be mistaken for the latest completion status.

## Agreed target and current uninstalled work

The migration target is all first-party application runtime logic in Rust.
Qt/QML and the pinned Proton SDK remain dependencies; neither a replacement GUI
framework nor reimplementation of Proton authentication/cryptography is intended.
Build tooling and regression tests are not application runtime dependencies.

The current working tree additionally moves these production paths into Rust:

- Typed Drive commands: login/logout, listing, metadata, folder creation, rename,
  move, trash/restore, transfers and sharing. Input checks, SDK argument construction,
  upload traversal, ambiguous-trash rejection and optional sharing response checks
  execute in `core/src/drive.rs`. Python only converts returned node records and
  sorts display names using the existing Unicode case folding.
- Event polling and durable acknowledgement: `core/src/event_runtime.rs` loads
  private cursor state, invokes the SDK, validates the entire batch, delivers
  notifications synchronously and commits only after all callbacks succeed.
  Callback errors/cancellation and persistence errors preserve the in-memory cursor
  for replay. The Python thread scheduler and dynamic root selection remain.
- `neutrondrive-runtime` is a native executable sharing the same safe Rust entry
  point as the C ABI. It does not start a Python interpreter. It is an integration
  entry point, **not yet a replacement for the desktop app or background daemon**.
  See [native interface](NATIVE-RUNTIME.md).

- Descriptor-relative file scans, SHA-1 content proofs, upload staging and guarded
  download installation now execute in `core/src/filesystem.rs`. Native tests
  exercise cancellation between chunks, concurrent writers, destination races,
  failed installation and backup recovery. SHA-1 preserves the existing SDK
  comparison protocol; it is not used as an authenticity signature.
- Local-trash tree proofs now execute in Rust, including mount identity, hardlink
  rejection, stable metadata, content hashing and nested directory fsync. Receipt
  storage, receipt listing, same-filesystem trash-location selection and restore
  orchestration now also run in Rust. Move-to-trash orchestration also runs in
  Rust, with the final remote validation delivered as a synchronous callback.
- Computer registration and folder setup now execute in `core/src/computer_setup.rs`.
  A private process lock serializes setup; account/device/local identities are
  checked and jobs stay paused until identity pins have been saved. The Python
  adapter only forwards the request and constructs the returned job object.

- Offline language processing now executes in `core/src/localization.rs`: Linux
  locale precedence, regional/script matching, English fallback, bounded catalog
  reads, placeholder validation and diagnostic substitution. Captured filenames
  and other values remain literal. Qt-facing Python bindings keep the selected
  preference and catalog cache until the controller is migrated.
- `core/src/control.rs` owns the local control client: nonblocking Unix sockets,
  peer credentials, bounded newline-framed replies and a whole-request timeout.
  The daemon's command handlers, server and lifecycle remain Python.

- Server-side control-message transport now also executes in `core/src/control.rs`:
  peer authentication, typed request decoding, framing, receive/send size limits
  and deadlines. Borrowed descriptors retain their ownership and flags. The
  daemon still accepts connections and dispatches commands in Python. Native
  regressions cover malformed/truncated/oversized requests, disconnected peers,
  blocked sends and safe descriptor handling; the daemon regression confirms
  malformed requests cannot stop service or resume paused jobs.

- Background filesystem-echo verification now executes in Rust. It compares
  file identity, size and nanosecond modification/change timestamps, traverses
  directories through no-follow descriptors, and checks the observed subtree
  against the settled snapshot. Mount exclusions and root/parent revalidation remain
  conservative: uncertainty requests reconciliation. Reserved staging/backup
  names stay excluded. A missing folder with known snapshot children is not
  accepted as a settled deletion echo.

- Local-trash journal locks and receipt persistence now execute in
  `core/src/trash_journal.rs`. Rust owns opaque transaction handles, process locks,
  bounded private reads, exclusive initial receipts and atomic durable updates.
  State/entries/lock identities are revalidated; symlinks, hardlinks and replaced
  directories fail closed. Existing receipt formats remain readable. Move-to-trash orchestration and receipt listing are still Python.
  Restore orchestration and its recovery decisions now execute in Rust. Long Unicode recovery
  names are truncated only at UTF-8 boundaries so journal serialization and
  restoration remain valid.

- The complete local-trash restore transaction now executes in
  `core/src/trash_restore.rs`: journal locking, ancestor and content verification,
  no-overwrite rename, durable completion, recovery metadata and reconciliation
  after an interrupted commit. The Python entry point only forwards arguments,
  cancellation and receipt/error objects. Three Python syscall-mock tests moved
  to native fault-injection tests covering the actual rename/commit boundaries.
- The complete delete-to-trash transaction now executes in `core/src/trash_move.rs`:
  stable local proofs, final remote validation, exclusive prepared receipts,
  standard trash metadata, no-overwrite rename, post-move proof, guarded rollback
  and recovery metadata. Eight syscall/journal fault cases moved from Python mocks
  to native tests at the actual transaction boundaries. `local_trash.py` is now
  an adapter for argument/result objects and synchronous callbacks.

The local-trash checkpoint x86-64 core was built offline in an isolated Ubuntu 22.04.5 rootfs.
Its measured maximum GLIBC symbol version is 2.34, within the supported 2.35
baseline. Verified source/binary provenance is in `build/rust-native-trash-check`.
This establishes core-library compatibility, not a complete package release or
validation on every distribution. That checkpoint validation passed 71 Rust unit
tests, 3 native executable integration tests, Clippy with warnings denied, and
all 357 application tests without skips. The application suite included Qt
(offscreen), real kernel FUSE, isolated D-Bus and package staging, using synthetic
SDK data. No new real-account test was performed. See [validation record](RUST-MIGRATION-VALIDATION.json)
for the current test results.

This work is not installed or released. The installed local3 package is unchanged.
It does not complete the requested migration. Remaining first-party runtime work
includes Qt controller/appearance/preference state, daemon lifecycle/accept/command dispatch, sync
orchestration/remote scanning, online-drive/FUSE,
desktop D-Bus service and update HTTP/download/control. These must retain existing
storage formats, cancellation, recovery and user-visible behavior before removing
Python/PySide6 from packages.

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
| Desktop service and Qt controller | Python/PySide6/Gio | Native service/Qt bindings preserving QML and system appearance |
| Update transport and controller | Python with Rust policy | Native network/download and UI coordination |
| Launcher, DTO bindings and packaging | Python runtime remains required | Remove temporary adapters and switch launcher/dependencies after integration |


Completion requires the complete regression suite, real FUSE/D-Bus/UI tests,
synthetic account tests, baseline-compatible packages and installed-package
verification. Removing dependencies or inflating GitHub's Rust percentage does
not satisfy those gates.

## Checkpoint validation

Runtime source: `2a1c100bfa0a0569c9ebd1a081374171ca9a00ae`.
All 15 jobs in [the Linux/architecture workflow](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39644716748)
passed, including native ARM64 execution and installed DEB checks on both
architectures. RPM, Flatpak and Snap builds also passed. The duplicate workflow
started when opening the draft PR was cancelled; the linked source run completed.

All 332 local Python tests passed in aggregate with Qt, real FUSE and isolated
D-Bus enabled, without skips. The first full run passed 331 tests; its staging
test hit the host's temporary-directory disk quota. That one test passed when
rerun on the project filesystem, without source or assertion changes. All 122
focused runtime tests, 16 Rust tests and Clippy passed. Tests cover callback
failure cleanup, SDK termination grace, descendants holding pipes, concurrent
job updates, malformed/private state, atomic replacement and native byte-name
watching. A deterministic regression covers a late delete echo after the last
file disappears, followed by detection of a genuinely new file.

The installed Flatpak passed source/backend/Qt/core hash verification, real
native process execution, job transactions, inotify delivery/stop, all six GUI
pages, light/dark switching and local trash restoration. Its installation and
profile were isolated and removed afterward. No Flatpak login was performed.
The x86-64 core retains a measured GLIBC 2.34 symbol baseline; package builds
continue to enforce the existing compatibility check.

All 11 real-account checks passed against the same source/build: initial sync,
both file-deletion directions, restart/no-resurrection, cloud/local restores,
whole-folder conflict preservation and recovery, and delete-versus-edit conflict
preservation. The dedicated synthetic test daemon stopped and its UUID-scoped
cloud folder was moved to trash. No permanent cloud deletion or credential
copying occurred.

These checks validate this checkpoint, not completion of the remaining runtime
migration, universal Linux compatibility or a comparative speed improvement.

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
SDK event-source orchestration, sync orchestration and remote scanning,
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

## Historical checkpoint: native local trash complete

The continuous migration goal remains active. Local-trash initialization,
location selection, tree proofs, journals, moving, listing and restoration all
execute in Rust. The thin Python adapter remains until its GUI/sync callers are
migrated; it does not implement filesystem mutation or recovery decisions.

The new transaction tests cover target collision, last-window source replacement,
post-move editing with an occupied source, interrupted journal commits, failed
initial persistence, cross-device rename refusal, source-parent replacement and
hash/remote-proof/rename ordering. Existing binding tests continue to exercise
cancellation, metadata, Unicode names, duplicate operation IDs, complete-folder
proofs, restoration and automatic deletion propagation.

Local-trash checkpoint validation: 71 Rust unit tests, 3 executable integration tests, Clippy
with warnings denied, 87 focused sync/trash binding tests, and all 357 application
tests (zero skips/failures/errors). The full suite used the provenance-verified
Ubuntu 22.04 baseline build at `build/rust-native-trash-check`. Test-count changes
reflect moving fault scenarios into Rust, not dropping scenario coverage.

Next: sync orchestration/remote scanning and baseline execution, daemon lifecycle,
FUSE/cache, update and Qt-controller migration. Qt/QML and the pinned SDK stay.

## Historical checkpoint: remote sync traversal

`core/src/sync_remote.rs` now owns remote node validation, root/account ancestry
proofs, recursive enumeration, duplicate-name/identifier detection, metadata
hydration and fresh preflight lookup. Missing-digest hydration verifies that the
returned node has the original stable UID. Traversal is bounded to 256 levels
and 100,000 distinct IDs and propagates cancellation.

The `Services` boundary supports both direct native SDK invocation and the
existing serialized/cached Python backend during migration. The latter only
answers synchronous metadata requests; it no longer implements tree traversal
or validation decisions. `host.rs` owns thread-bound, single-use callback replies
and removes them on success, missing response or cancellation. The direct SDK
adapter does not use a Python interpreter or host-reply callbacks.

For this source increment: 76 Rust unit tests, 4 native executable integration
tests, Clippy with warnings denied, and 134 background/sync/trash integration
tests pass. The additional executable test drives the real native SDK transport
against a synthetic shell fixture with isolated state. No account was contacted.
The earlier full application suite and Ubuntu-baseline binary describe the
previous local-trash checkpoint; they must be rebuilt/retested for this source.
Sync plan/execution and baseline persistence still remain in Python and are next.

## Historical checkpoint: native sync baseline persistence

`core/src/sync_state.rs` now prepares private state, derives the compatible
SHA-256 key, reads/validates v1/v2 baselines and pending-deletion records, and
atomically persists validated v2 state. Reads are bounded, nonblocking and
nofollow; symlinks, hardlinked baselines and changes observed during reading
fail closed. Old v1 entries remain identity-version 1 until convergence proves
new identities. Reading never rewrites the original baseline. Raw-byte SHA-256
digests retain stale-preview detection independently of JSON formatting.

RustCrypto `sha2` 0.10.9 was vendored from the local Cargo archive after verification
against its registry checksum, with both upstream licenses retained. All builds
remain locked/offline. The native key derivation was also compared against the
old Python SHA-256 derivation for ASCII and Unicode roots.

Current source validation: 78 Rust unit tests, 4 native executable integration
tests, Clippy with warnings denied, 87 focused sync/trash tests and all 357
application tests with zero skips/failures/errors. The full suite used the current
provenance-verified `build/rust-native-state-check` library, built offline in the
isolated Ubuntu 22.04.5 rootfs (maximum GLIBC 2.34). Qt, real kernel FUSE, isolated
D-Bus and package staging were included; real account/cloud tests were not run.
An exhausted temporary-file quota was resolved by deleting only this task's Rust
incremental cache; subsequent builds disabled incremental caching.

Sync plan/execution and deletion-proof orchestration are still Python. These,
the daemon lifecycle, FUSE/cache, updates and Qt controller remain part of the
active migration goal. No release or local installation was performed.

## Historical checkpoint: native preview and deletion preflight

`sync_plan.rs` now owns complete preview orchestration: state preparation,
baseline loading, root identity pinning, local/remote scans, policy evaluation,
fresh remote deletion proofs, conflict resolution and the resulting plan data.
`sync_proof.rs` owns deletion preflight: root/account verification, ancestor
location checks, local absence and inode-move checks, unchanged revision/content
proofs and confirmation of remote trash and its original parent. Whole-folder
deletion remains explicitly blocked by the pinned SDK limitation. The Python
adapter converts returned plan/result records for callers.

Two Python scan-mock tests were moved to native fault injection immediately after
preflight's local scan. They prove that trashing a selected cloud ancestor during
the last preflight blocks both local and remote deletion. An additional native
test rejects moved or revised deleted UIDs. A native executable integration test
constructs a complete preview using synthetic SDK transport without Python and
checks that the local tree remains unchanged.

Latest validation: 81 Rust unit tests, 5 native executable integration tests,
Clippy with warnings denied, and 132 background/sync/trash integration tests
passed. The earlier 357-test full application run and baseline artifact refer to
the preceding state-persistence checkpoint; this increment has not yet repeated
the whole application build. Next: sync execution/transfer orchestration and
process-lock ownership, then the remaining daemon, FUSE/cache, update and Qt work.

## Native sync execution verification

`sync_execute.rs` serializes execution with a private nofollow process lock and
checks for replaced state/lock paths. A contended lock does not consume the
caller's preview. Uploads stage a stable source; downloads verify SDK content and
recheck remote identity before guarded installation. All deletion intents are
fsynced before mutation. Pending local deletion is cleared only when a matching
persisted trash receipt and fresh remote proof agree. Changed/conflicting paths
retain their previous baseline. The direct SDK service implementation supports
metadata and mutations without interpreter callbacks.

Two Python mocks that no longer reached native operations were replaced by native
fault tests: failed initial intent persistence must prevent cloud mutation, and
lost local acknowledgement must preserve and reconcile exactly one receipt.
Native tests also cover lock contention/replacement and release. Existing binding
tests retain remote lost acknowledgements, callback interruption, cancellations,
source/destination races, root replacement and baseline compatibility coverage.
`host_failure` now transports recoverable SDK exceptions into Rust so execution
can record per-file errors and retain durable intent. Cancellation still aborts;
Python `BaseException` interrupts remain propagated after native cleanup.

## Current continuation: daemon metadata cache and event ancestry

`metadata_cache.rs` now owns directory listings, alias/UID/parent/child mappings,
generation checks and post-mutation invalidation. Its opaque handles own mutex-
protected state; active operations retain ownership across handle release. No
cache lock is held while calling SDK services. An event during a fetch prevents
that response from repopulating invalidated metadata. Metadata `info` requests
always reach the backend. Failed/ambiguous writes still invalidate old parents
and descendants. Python `RemoteTreeCache` is a binding with deterministic native
handle release on object destruction. Sync and cache share `backend.service_call`
for SDK callbacks and recoverable errors instead of duplicating that adapter.

`event_ancestry.rs` now resolves unknown event parents against known job UIDs and
account roots. Proven outside events do not enqueue unrelated jobs; unknown,
unavailable, cyclic or account-inconsistent ancestry conservatively schedules
only active jobs. Cancellation retains the existing stop/queue behavior. Python
still schedules those decisions, persists accepted work and manages threads.

Validation for this source increment: 89 Rust unit tests, 6 native executable
integration tests, Clippy with warnings denied, 130 background/sync/trash tests
and 52 backend/process tests passed. Native concurrency tests cover invalidation
while a fetch is blocked, handle release during a fetch, and invalidation after
lost write acknowledgement. No prior scenario tests were removed. The preceding
full Qt/FUSE/D-Bus/application run and baseline artifact belong to the sync-
execution checkpoint and have not yet been repeated for this increment.

Next: move daemon state, scheduling application and lifecycle into Rust; then
continue online-drive/FUSE, D-Bus, updates and Qt controller migration.

### Native job-root proof and service controller

`job_roots.rs` now proves the complete active job set before any identity is
pinned: account changes, replaced roots, aliases, overlapping subtrees, trashed
ancestors, cycles and the ancestry depth bound. SDK failures remain retryable;
unsafe identities remain blocked; a paused or removed selection remains
cancelled. The Python daemon only applies the proven pins to its current state.

`service_controller.rs` owns service status fallback, process creation, private
logging, startup acknowledgement, timeout termination/reaping and stop
acknowledgement. Child descriptors are closed on exec; only the newly spawned,
unreaped process is signalled. Private logs reject links and nonregular files.
The desktop binding still supplies the **temporary Python daemon entrypoint**;
the server loop, watchers/event orchestration and dispatch have not yet been
migrated. This is not a claim that the daemon or application is Python-free.

Validation: 96 Rust unit tests and 6 executable integration tests passed,
including an acknowledged start/stop exchange with an isolated Unix service,
a failed child and an unacknowledged child that must be terminated/reaped.
130 background/sync/trash regression tests passed using the Rust controller
and job-root proof. No tests were removed. Full Qt/FUSE/D-Bus tests and the
portable baseline artifact have not yet been repeated for this increment.

### Native daemon job transitions and owned control server

`daemon_job.rs` applies the job transitions for beginning/completing a run,
retrying failures, cancellation, blocked jobs and final pause enforcement. It
owns result accounting, echo snapshot construction, filtering settled local
writes without hiding concurrent changes, and preserving unknown/remote work.
The Python binding only converts the current DTO state and applies the result.

`control_server.rs` now owns the exclusive daemon lock, Unix listener, peer
request/reply loop and PID/socket cleanup. The listener is bound through the
pinned directory descriptor. Private lock files reject hardlinks and unsafe
owners/modes. The opaque handle remains alive through an active callback even
when released concurrently; cleanup checks inode identity so replacements are
not deleted. Python no longer opens/flocks the daemon lock, accepts sockets or
unlinks the daemon resources. It still supplies the command handler and main
scheduler loop; moving these remaining orchestration components is necessary
before switching the application to a Python-free daemon.

Validation for this checkpoint: 102 Rust unit tests, 6 native executable tests,
353 application tests with zero skips, Clippy with warnings denied and
`git diff --check` passed. The application run enabled all Qt offscreen,
real isolated D-Bus and kernel FUSE tests, including package staging and all
25 languages. The x86_64 core was rebuilt in the isolated Ubuntu 22.04.5
baseline, verified against current source/binary hashes and GLIBC 2.34, and
staged in `build/rust-native-daemon-check`.

The first full run exposed two obsolete cache tests that accessed removed
Python cache fields or injected failure into a Python policy call no longer
used by the native cache. These now verify actual cache hits and backend
refetches after an event during a fetch or a failed write. Native tests also
cover generation checks and concurrent handle release. The corrected full run
passed. This checkpoint does not claim a real Proton-account end-to-end test,
an ARM build, release publication, local installation or completed migration.

### Native daemon command and configuration reconciliation

`daemon_control.rs` now owns status/stop/reload/pause/resume/manual-sync control
flow and configuration reconciliation. Removing or pausing an active job cancels
its transfer. Explicit resume retries blocked watcher setup while preserving
pinned account/root identities. A failed watcher setup blocks that job, and a
manual sync does not unpause paused jobs. Changes are persisted before command
acknowledgement. The compatibility binding holds its coordination lock and
implements only primitive state, store and watcher operations.

Validation for this increment: 105 Rust unit tests, 6 native executable tests,
130 background/sync/trash regression tests and Clippy with warnings denied
passed. New native command tests cover cancellation, identity-preserving resume,
watcher failure, removed jobs, paused manual sync and persistence ordering.
The last full application run and portable baseline belong to the preceding
control-server checkpoint; they have not been repeated for this increment.
The shared daemon state owner, scheduler and thread orchestration still require
migration before the Python daemon entrypoint can be removed.

### Native daemon runtime and production entry

`daemon.rs` now runs the production background service: native shared job state,
condition-variable scheduling, inotify/control/event threads, serialized SDK
requests, durable event cursor delivery, ancestry reconciliation and sync worker
execution. Main-thread cancellation stays responsive while workers perform SDK
I/O. Stopping joins workers and releases owned sockets, locks and caches. Job
order and randomized retry jitter remain compatible with the previous service.

The normal application daemon entry now calls this Rust runtime directly through
a thin signal/FFI binding. The old Python `BackgroundDaemon`, `_Runtime` and
watcher-thread orchestration were removed from the shipped application. A
clearly marked `tests/reference_daemon.py` retains the injected-backend scenario
harness for regression testing of native sync/cache/control boundaries; it is
not staged into application packages. Separate native executable tests exercise
the actual Rust owner and threads with no Python on PATH, and a binding test
rejects any accidental use of a legacy Python daemon.

Direct native scenarios include SDK-fixture download and rescan, local inotify
changes, preserving local content after a failed upload, automatic retry state,
pause/resume with pinned account identity, durable event cursors, stop and
SIGTERM during a blocked SDK event process. No real account data is used.

Final checkpoint verification: 106 Rust unit tests, 8 executable integration
tests and all 354 application tests passed with zero skips, including Qt,
real isolated D-Bus, kernel FUSE and native/sandbox package staging. Clippy passed
with warnings denied. The final x86_64 Ubuntu 22.04.5 baseline artifact in
`build/rust-native-service-check` matches the current source/binary hashes and
requires at most GLIBC 2.34. Earlier runs found a temporary-filesystem quota
exhaustion during Qt staging and an obsolete watcher-test import; obsolete
project incremental caches were removed, the import was corrected, and the full
run passed. No release, installation or real Proton-account test was performed.

The overall application is still not Python-free. Remaining work includes the
online-drive/FUSE runtime and GUI event binding, desktop D-Bus, update transport
and controller, Qt/PySide6 controller, and final native launcher/package wiring.

### Native online-drive cache I/O and writeback

`ondemand_io.rs` owns private cache I/O, bounded atomic JSON, stable borrowed-FD
SHA1 verification, cache keys compatible with existing Unicode metadata, and
the entire verified download/cache-install transaction. Metadata enumeration
still does not download file contents. Hydration detects a staged file replaced
between the content proof and final remote check, and leaves no verified object
when that proof fails.

`ondemand_write.rs` owns revision checks, upload staging, durable writeback
journals, post-upload content verification and independent cache-copy creation.
Lost acknowledgements preserve the old baseline and local edit; a confirmed
remote commit followed by local cache failure retains the new baseline in the
recovery record. Later edits cannot modify a verified cache inode. Native POSIX
errors are retained by the FUSE-facing binding. No recovery journal is replayed
automatically. Both operations support native SDK services without Python.

Python still owns the on-demand inode/handle tables, directory metadata policy,
namespace operations and pyfuse3 integration. Those are the next migration scope;
this checkpoint does not claim a Python-free online filesystem.

### Native online-drive directory reconciliation

`ondemand_tree.rs` now owns filename/node validation, saved directory metadata,
reachable-tree persistence and cloud listing reconciliation. The Python adapter
passes typed snapshots and converts returned entries to its temporary DTOs; it
no longer implements these algorithms. Existing v1 cache metadata is retained.
Open-file snapshots, pinned revision content, stable inode allocation, local
create conflicts and offline empty-directory listings remain supported.

Additional native tests cover pinned/open/local reconciliation, ambiguous and
unverifiable files, Unicode component limits, metadata round trips, corrupt
parent/duplicate-inode rejection without overwriting the cache, and the actual
executable protocol with integer-indexed JSON child maps. Inode/handle lifetime,
namespace operations, FUSE and the other previously listed Python runtime
components still require migration; the overall goal remains active.

### Native online-drive open files and local edits

`ondemand_handles.rs` now owns file descriptors, verified private edit-copy
creation, local read/write/truncate/sync, durable pre-write journal checks and
release cleanup/recovery. Python receives opaque handle identifiers and a
borrowed descriptor for the existing native writeback transaction. The former
Python copy/write/truncate/descriptor-close implementation was removed. Failed
truncating opens also release their new native handle without changing the
original content. The inode/handle DTOs and cloud namespace coordinator remain
in Python for the next stage; this is not the final Python-free runtime.

Native regression cases prove that read handles survive the original borrowed
FD closing, read handles cannot write, pending edits require durable intent,
append/truncate and dirty release retain expected contents, failed journals stop
mutation while preserving bytes, edit copies cannot alter cache objects, and
failed source verification cleans its unexposed work file. Existing on-demand
fault-injection tests remain active, including ENOSPC without phantom creates.

### Native editor replacement and recovery inspection

`ondemand_replace.rs` replaces the Python editor-save transaction, including
root/revision preflight, native source hydration, independent recovery work
file, verified target writeback, durable target metadata, source revision check,
source trashing and final metadata/recovery cleanup. The native response carries
updated tree state even on a failure after a confirmed target commit. Local
replacement bytes and the confirmed baseline survive source-trash errors,
concurrent source edits and metadata persistence failures. Open or stale targets
are rejected before any upload. The temporary Python wrapper only transports
snapshots and adapts outcomes.

Recovery-journal enumeration and validation also moved to Rust. Tests cover
active-journal exclusion, malformed JSON shapes and unsafe data paths while
preserving work files. The migration is still incomplete: mount coordination,
remaining namespace operations, FUSE, desktop/Qt and update bindings remain.

### Native cloud namespace operations

`ondemand_namespace.rs` replaces the remaining Python mkdir, rename/move,
replacement dispatch and unlink algorithms. It owns fresh metadata reconciliation,
account/open-file/revision guards, native replacement invocation and persistence.
The temporary binding only supplies source inodes and mount snapshots and applies
returned state. The direct SDK and host adapters now expose rename and move.

Native tests reject open files, changed source revisions, wrong accounts and
nonempty directories without sending a mutation. Nonempty checks also include
unsupported documents hidden from the FUSE view. Only a confirmed replacement
commit updates the last-read target observation; an unrelated refreshed cloud
revision cannot silently authorize overwriting it. Mount state and ordinary file
open/flush coordination, FUSE, Qt/desktop and updates still need migration.

### Owned native online-filesystem runtime

`ondemand_fs.rs` now combines the native cache, handles, directory, replacement
and namespace components into owned mounts. Rust owns cache locking, inode and
open-file state, metadata TTL/refresh/fallback, offline pin/evict, root-account
blocking, open/create/read/write/truncate/flush/release and shutdown recovery.
`src/neutrondrive/ondemand.py` is now a 171-line binding instead of the filesystem
implementation. The existing Qt/QML and pyfuse3 callers keep their public API.

Two Python journal monkeypatch tests moved to Rust fault injection at the actual
journal boundary: failed creates cannot expose phantom entries or leak handles,
and failed truncating opens preserve the original bytes and release the handle.
Additional native tests cover exclusive mount locking, confirmed writeback,
offline remount/read without SDK access, unflushed-close recovery and SDK timestamp
fraction/offset conversion. An isolated native test process exercises the actual
runtime API and SDK transport without Python on PATH, then opens the pinned cache
with an unavailable SDK. FUSE/desktop/Qt/update integration and final packaging
still remain before the overall migration is complete.


## Native kernel FUSE checkpoint

`core/src/fuse.rs` now implements the real Linux kernel adapter using pinned,
vendored `fuser` 0.18.0 without its optional libfuse bindings. Rust owns directory
handle snapshots, private metadata and permissions, direct-I/O file handles,
read/write/truncate, flush/fsync/release, namespace operations, statfs and the
online/pinned/cache xattr. Atomic open/truncate is explicitly negotiated; kernel
writeback caching is not enabled. Rust worker requests cross an owned channel to
a synchronous service pump, so foreign callbacks never outlive their calling
thread. Cancellation detaches the mount and drains pending responses.

The production mount uses Rust's direct SDK transport. Injected test backends use
the same native adapter through synchronous host requests. Nonblocking native
recovery inspection keeps status responsive during hydration. `fuse.py` now only
contains transitional mount lifecycle and desktop bindings; pyfuse3/Trio code and
package dependencies have been removed, including Fedora's private binding build.
This does not remove Python from the desktop application yet.

Verification: **131 Rust unit tests, 11 native executable integration tests and
358 application tests passed, with zero skips or failures**. The application run
includes Qt, isolated D-Bus, nine real kernel-FUSE tests and package staging.
New regressions cover atomic truncating open, status during blocked hydration,
private mode/xattrs, busy unmount, callback-failure cleanup and direct native SDK
transport with Python backend methods forbidden. Clippy passes with warnings
denied. `build/rust-native-fuse-check` was built offline against Ubuntu 22.04.5;
its maximum GLIBC requirement remains 2.34. No real-account test, installer build,
ARM64 runtime test, release or installation was performed at this checkpoint.
The continuous migration goal remains active.


## Owned native mount-manager checkpoint

`core/src/mount_manager.rs` now owns the mount lifecycle: path/home expansion,
symlink and overlap validation, directory identity revalidation before startup,
exclusive session reservation, readiness/termination coordination, mountinfo
identity checks, bounded helper execution, busy-unmount handling, status/recovery
inspection, pin/evict/refresh and account-event invalidation. Failed startup closes
the native filesystem and releases the cache/session. Released managers reject
already-retained references, preventing a concurrent late start from leaking a
filesystem. Status remains responsive during SDK work and closed mounts retain
recovery inspection while ignoring late events.

`fuse.py` is now an 83-line call/thread binding. Its temporary thread supplies
scoped callbacks to the native service pump; it owns no mount state or policy.
Qt/QML and the pinned Proton SDK remain unchanged. The GUI event-source scheduler,
D-Bus service, Qt controllers, update transport and final launcher/package removal
of Python bindings remain outstanding. The migration goal is still active.

Verified: **136 Rust unit tests, 11 native executable integration tests and all
360 application tests passed, with no skips/failures**; Clippy with warnings denied
and `git diff --check` pass. The native executable suite was run with
`NEUTRONDRIVE_FUSE_TEST=1`: an isolated Rust test process creates a manager, mounts,
reads through the kernel, unmounts and releases it using a synthetic SDK. Its PATH
contains only the mount helper, with no Python interpreter, and host requests are
rejected. Twelve application kernel tests include failed thread startup, changed
prepared mountpoint, exclusive sessions, account events and closed-mount events;
47 focused filesystem/kernel tests also pass. The previous account-event test that
injected Python manager state now exercises the actual native manager/kernel.

`build/rust-native-manager-check` was built offline on Ubuntu 22.04.5, verified
against current source hashes, and has maximum GLIBC 2.34. Full installer builds,
ARM64 runtime checks, live-account tests, release and installation were not
performed for this checkpoint.


## Owned native desktop event-source checkpoint

`core/src/event_source.rs` now owns desktop event-source state, root selection
validation, cursor files, reload/reconnect announcements, polling, scheduling,
retry/backoff, redacted errors and cancellation. Direct SDK queries and the
acknowledgement transaction are native. No SDK lock is held while consumers
accept notifications. Failed delivery or persistence leaves both in-memory and
durable cursors unacknowledged. Stop wakes waiting work, cancels/reaps active SDK
processes, supports invocation inside a consumer callback and does not cancel a
separate user's transfer. A signal callback is also polled during idle waits.

`events.py` only adapts DTOs/scoped host callbacks and the temporary host thread.
Injected test services use the same native state and scheduling. Production
polling invokes the SDK directly. Cursor-file hashing is shared with the native
daemon and preserves Python's historical ASCII JSON escaping and comma spacing,
including Unicode roots. The production status-injection helper was removed;
its reconnection regression now targets native state in a Rust unit test.

Verified: **141 Rust unit tests, 12 native executable integration tests and all
361 application tests passed, with zero skips/failures**. Sixteen focused event
binding tests pass, including actual in-flight SDK cancellation/reaping and
restart after thread-start failure. Native tests cover rejected consumer
acknowledgement/replay, empty-batch reconnection, Unicode cursor filenames,
redacted background errors, stop inside a consumer, and cancellation during an
idle wait without another SDK query. An isolated executable test runs SDK
query/delivery/persistence/stop with `PATH=/nonexistent` and rejects host-service
requests. Clippy with warnings denied and `git diff --check` pass.

`build/rust-native-event-source-check` was built offline against Ubuntu 22.04.5,
verified against the current Rust sources, and retains maximum GLIBC 2.34. The
application run includes Qt, isolated D-Bus, real kernel FUSE and package staging.
No live-account test, full installer build, ARM64 execution, release or installation
was performed. The overall migration remains active; update transport, desktop
D-Bus, Qt controllers and final launcher/binding removal are still outstanding.


## Native update transport checkpoint

`core/src/update_runtime.rs` owns installation-format detection, bounded release
checks, HTTPS package download, SHA-256/size verification, private staging,
cancellation cleanup and stable file/path revalidation. HTTPS uses pinned and
vendored ureq/rustls with the operating system certificate verifier, HTTPS-only
redirects and bounded timeouts. `updates.py` now only converts DTOs and native
errors; it no longer performs network, hashing or staging I/O. The Qt update
controller and authorized package-installation orchestration remain to migrate.

Verified: **148 Rust unit tests, 13 native executable integration tests and all
360 application tests pass, without skips or failures**. Seven focused update
binding tests also pass. Previous Python URL-open monkeypatch cases moved to
native transport fixtures. Rust tests additionally exercise real local TLS,
certificate rejection, HTTPS downgrade rejection, private modes, short/corrupt/
oversized download cleanup, mid-stream cancellation, replaced-file cleanup,
hardlink rejection and directory replacement during verification. The native
executable verifies a package with no Python or hash commands on PATH. A public
GitHub release check passed through the production system TLS verifier. Clippy
with warnings denied passes.

`build/rust-native-update-check` was built offline on Ubuntu 22.04.5 and verified
against these Rust sources with maximum GLIBC 2.34. The full application run
includes offscreen Qt, isolated D-Bus, real kernel FUSE and package staging.
No live-account test, full installer build, ARM64 execution, release or local
installation was performed. The continuous migration goal remains active.


## Native autostart checkpoint

`core/src/autostart.rs` now owns XDG/Snap autostart paths, launcher selection,
Desktop Entry argument escaping, bounded owned-file reads, managed-entry checks,
private random staging, durable atomic writes and removal. Foreign entries,
symlink ancestors, hardlinks and oversized files are rejected. Flatpak uses its
portal instead of direct file writes; Snap retains its declared wrapper command.
The home-directory lookup shares the existing native passwd fallback. The Python
module now only binds these operations and retains the pending portal callback.

Verified: **151 Rust unit tests, 14 native executable integration tests and all
360 application tests pass, without skips or failures**. Five focused binding
tests pass, along with Clippy with warnings denied and `cargo fmt --check`.
The native executable tests XDG entry creation/removal and Snap/Flatpak launcher
boundaries with no Python on PATH. The full application suite retains the real
Gio Desktop Entry argument-parsing test, Qt, isolated D-Bus, kernel FUSE and
package staging. Its first run exhausted the temporary-directory quota during
Qt package extraction. The task-owned host build cache was moved to
`build/rust-host-target` (with its old /tmp path retained as a symlink); the full
suite then passed after sufficient temporary space was restored.

`build/rust-native-autostart-check` was built offline on Ubuntu 22.04.5 from the
final formatted Rust sources and verified with maximum GLIBC 2.34. No new
live-account test, full installer build, ARM64 execution, release or installation
was performed. Desktop D-Bus/portals, Qt controllers, installer orchestration and
final launcher/binding removal remain; the continuous goal remains active.


## Native desktop D-Bus client and portal checkpoint

`core/src/desktop_bus.rs` now owns the session-bus client, bounded request/reply
validation, non-activating status queries, separate Snap service name, cancellable
waits, source-helper startup and reaping. Only undelivered missing-service/name
errors allow startup and a retry. NoReply, disconnection, denied access and
timeouts never repeat a mutation. An existing Snap integration test caught the
new library's typed FDO error representation; that case is now handled and
covered by a native regression test.

`core/src/portals.rs` owns Background consent transactions and Settings watches.
It subscribes before requesting, accepts early responses and changed request
paths, and verifies the response's unique sender. Appearance watches preserve
change-before-read ordering, accept legacy nested Read variants, and cancel on
close. Python now only supplies temporary threads/Qt/main-context callbacks.
A new failure test verifies Qt fallback and no false consent when a binding
thread cannot start. A previously unclosed test-only application left a native-watch callback thread alive at interpreter shutdown; the test now disposes it, and the full suite exits zero with only the main Python thread alive. The service dispatcher/mount-event orchestration and Qt
controllers remain Python and are still being migrated.

Verified: **156 Rust unit tests, 14 native executable integration tests, two
native D-Bus integration test entries (six isolated bus cases), and all 361
application tests pass, without skips or failures**. Three focused existing
Qt/service integration tests also pass. Native bus fixtures have no Python on
PATH and exercise early responses, spoofed senders, cancellation, nested
settings, stale initial reads and no retry after NoReply. The application suite
retains real Qt appearance changes, service activation/fallback, Snap isolation,
D-Bus-to-kernel-FUSE behavior and package staging. Clippy with warnings denied,
`cargo fmt --check` and `git diff --check` pass.

Dependencies are pinned and vendored (zbus 6.1.0, zruntime 0.2.1); Rust's declared
minimum is 1.89, while the pinned build compiler stays 1.98.0. Two build wrappers
returned SIGTERM (the second after Cargo reported successful compilation). A
fresh cached build then verified the completed artifacts with exit code zero. `build/rust-native-desktop-check` was built offline
on Ubuntu 22.04.5, verified against the final sources and retains maximum GLIBC
2.34. No new live-account test, full installer build, ARM64 execution, release or
local installation was performed. The overall migration goal remains active.


## Native online-drive service core checkpoint

`core/src/desktop_service.rs` now owns service operation serialization and busy
state, independent status queries, guarded mount configuration, restore policy,
sync-folder overlap checks, mount rollback on persistence failure, account-switch
restrictions, and event-watch startup/stop failure decisions. Corrupt stored
boolean flags are rejected before restoration. An event watcher that has not
stopped remains reachable for a later retry, and failed watcher startup leaves
the mount usable with a safe diagnostic. The Python `ServiceCore` now supplies
scoped mount/event/SDK calls; the D-Bus server and direct native worker ownership
remain migration work, along with Qt controllers and final launcher wiring.

Verification: **160 Rust unit tests, 14 executable integration tests, two native
D-Bus test entries (six isolated cases), and all 363 application tests pass**.
The initial parallel executable run had two five-second fixture-start timeouts;
all 14 passed on serial repetition. Twelve targeted application service tests
also pass, including real D-Bus/FUSE integration, busy unmount, write rollback,
responsive status, corrupt restore flags and degraded event startup. Clippy
with warnings denied, formatting and whitespace checks pass. The full application
suite exits zero, with only the main Python thread remaining after cleanup.

`build/rust-native-service-check` was built offline on Ubuntu 22.04.5, checked
against the source hashes, and still requires at most GLIBC 2.34. No release,
local installation, live-account test, ARM64 execution or full installer rebuild
was performed. The complete migration remains active and unfinished.


## Native production desktop service checkpoint

`core/src/desktop_runtime.rs` now runs the production session-bus server and owns
the mount thread, event thread, SDK calls, cancellation and shutdown. It checks
caller UID, limits request/response sizes, uses a bounded mutation queue, and
keeps status/cancel responsive during SDK work. Native and Snap names remain
separate. Service registration explicitly disables both replacement flags that
zbus enables by default, so a second instance cannot steal the active name.
The user-facing client protocol is unchanged.

The Rust event worker invalidates mount state before event acknowledgement.
A busy user-requested unmount preserves both the live mount and watcher.
Foreground cancellation is independent of FUSE/event lifetime. Shutdown cancels
and reaps SDK work, stops the event source and releases the mount. External ABI
callbacks run only on the invoking thread. `desktop_service.py` is now just a
launch/client binding; the former injectable service adapter and Gio server
live only in `tests/reference_desktop_service.py` and are not shipped.

Verified: **160 Rust unit tests, 14 native executable integration tests, four
D-Bus test entries covering eleven isolated scenarios, and all 363 application
tests pass**. The five new native scenarios cover native/Snap ownership,
responsive status and cancellation during login, shutdown during an SDK call,
real FUSE reads/pinning/offline cache/busy unmount, and a standalone native
service process stopped with SIGTERM. They run with no Python on PATH. Shutdown
checks ensure the SDK PID is gone and no test mount remains in mountinfo.
Two existing application transport tests initially killed their Python fixtures
before startup at 250 ms; their startup deadline is now one second while keeping
the descendant-reaping and 200 ms graceful-cleanup assertions. The complete
application rerun exits zero with only its main Python thread remaining.

Clippy with warnings denied, formatting and whitespace checks pass. A sandboxed
Clippy attempt could not open the Cargo lock on the project share; the subsequent
permitted run passed. `build/rust-native-desktop-runtime-check` was built offline
on Ubuntu 22.04.5, verified against source hashes and requires at most GLIBC 2.34.
No release, installation, live-account test, ARM64 execution or complete installer
rebuild was performed. Remaining work includes the Qt controller/preferences,
update controller/installer, foreground/CLI launch binding, native Qt packaging
and removal of the remaining Python bindings. The overall goal remains active.


## Direct Rust/Qt foundation

The new `desktop/` crate introduces CXX-Qt 0.10.0 / CXX 1.0.205, allowing Qt
QObject properties, invokables and notifications to be declared and implemented
in Rust. `NativeAppearance` implements manual/system appearance, Qt fallback,
the native Settings portal watch, queued GUI-thread delivery and owned-thread
shutdown. Its QML check verifies dependent bindings update on property signals,
invalid modes fall back to system, and repeated shutdown leaves no worker.

The locked offline native build, one native state test, Clippy with warnings
denied, formatting and the real QML check pass. The test runs on a private bus
without service activation and with no Python on PATH. All original Rust-core
package identities/checksums remain in the desktop lock; 35 pinned package
versions were added to the vendor directory without replacing existing ones.
The unchanged production-core artifact's source provenance was reverified.

This is a **development foundation, not the completed native desktop app**.
The installed/production frontend still uses its existing Python controller.
The new crate deliberately accepts only its validation command until the main
controller is migrated. Its initial isolated SDK uses Qt 6.10.2 Ubuntu host
headers/tools and the existing standalone Qt libraries; it is not a portable
installer SDK. No application installation or release was performed, and the
unchanged full application suite was not repeated for this new separate crate.
See `desktop/README.md` and `docs/NATIVE-QT-VALIDATION.json` for the reproducible
check and remaining gates. The overall migration goal stays active.

## Native Qt file controller and preference checkpoint

`desktop/src/files.rs` now implements file-browser state and actions in Rust;
`file_controller.rs` owns the asynchronous Rust worker and Qt notifications.
Dialog tokens reject stale answers, dialog targets survive navigation, unsafe
nodes cannot be mutated, trash actions are gated, and stale completions cannot
release a newer operation. File entries emit a separate notification from
selection/status. Sharing retains copy/create and separately confirmed revoke.
The existing core remains the validation/SDK boundary.

`NativePreferences` preserves the existing INI location, manual/system language
and appearance options, next-launch language semantics and failed-write behavior.
The native localization adapter loads all existing catalogs and uses Qt locale
formatting. Qt API wrappers are restricted to missing constructors/Qt calls;
application policy is Rust. A regression test caught Qt's simple case folding
failing `STRASSE` → `Straße`. The Rust browser now uses pinned Unicode 16 full
folding; all 1,112,064 scalar values match the former Python casefold reference.

Verified: **11 native Qt-crate Rust tests**, Clippy with warnings denied,
formatting, and **two real QML checks**. The controller check runs with no Python
on PATH and isolated INI settings on a private session bus. It verifies reactive
preferences, stable file-list bindings, native dialogs, demo mutation rejection,
missing-SDK errors, explicit cancellation and destruction during synthetic SDK
calls. Both actual child PIDs are gone afterward. The first sandboxed bus check
could not bind its local socket; the permitted isolated test passed. No account
data or production settings were used.

This is still component migration. Production still uses the Python main
controller; service pages, account/window/update orchestration, CLI launch,
portable frontend packaging and final removal of Python remain. The unchanged
production application's full regression suite was not rerun for these separate
native components. No release or installation was performed. The continuous
migration goal remains active.

## Native account and headless-service checkpoint

The Rust Qt controller now receives validated login URLs during SDK execution,
queues them to the GUI thread, discards stale operation events, and removes the
URL/login control on completion or cancellation. URLs are not included in general
state snapshots. Successful login refreshes the listing. Logout requires a new
confirmation, stops background sync and unmounts native drives before SDK logout;
cleanup failures stop the sequence. Flatpak retains the previous mount behavior.
The existing QML cancellation action now reaches the native cancellation token.

Native `--daemon`, `--background` and `--desktop-service` entry points run before
Qt initialization and handle SIGINT/SIGTERM through owned Rust runtime cleanup.
The desktop-service fallback launcher uses the same native executable. No Python
interpreter is used for these paths. The new direct libc dependency reuses its
already-pinned core version; no new package version was added.

Validation now includes 16 Rust unit tests, both real QML component checks,
and two additional headless process checks on a private bus. Tests cover
operation-scoped login events, cleanup ordering/failure short-circuiting, Flatpak
behavior, service argument parsing, synthetic streaming login cancellation,
and signal shutdown without a graphical session. The daemon removes its control
socket and PID record; both synthetic SDK processes are reaped. Clippy, formatting
and whitespace checks pass. No live Proton login or production install occurred.
The combined `--background` restoration path still needs its own end-to-end gate.
Main-window/service-page integration, updater, window lifecycle, remaining CLI
modes, portable frontend builds and Python removal remain; the goal stays active.

## Native CLI and foreground-mount checkpoint

The native launcher now implements all prior non-GUI mode routes: sync
status/stop, online mount/unmount, pin/evict, service status, foreground mounts,
diagnosis, version and help, in addition to the existing service modes. Typed
option handling preserves targets, remote roots, read/write/offline flags and
custom daemon-state paths. Main-window launch and localized CLI resource wiring
are still pending. Diagnosis calls the new safe Rust policy entry point for core
information; the existing C ABI delegates to that same function and an equivalence
test checks valid, invalid, version-mismatched and empty requests.

The foreground path owns a native mount manager through prepare/run/cleanup,
always attempts registry release, and preserves cancellation in the run result.
A real kernel-FUSE test mounts synthetic data, reads it, signals shutdown, then
mounts and reads the same cache offline. Both instances exit zero after SIGTERM
and leave no mount. The headless check also verifies combined restore/daemon
startup, stopping a running daemon through the CLI, JSON status/diagnosis output,
missing SDK failure, and cancellation/reaping of an in-flight diagnostic SDK.

Verified: **19 desktop Rust tests, 161 core unit tests, both QML checks, headless
CLI/service checks, and online/offline foreground FUSE checks**. Clippy with
warnings denied passes for both crates; formatting and whitespace checks pass.
An initial sandboxed core run failed nine tests needing local sockets/TLS test
servers; the permitted rerun passes all 161. No live account, release, install,
complete application-suite rerun or portable package rebuild was performed.
The older portable core artifact now predates source changes and needs rebuilding;
its historical validation is not evidence for the current core sources. The
full native GUI and Python runtime removal remain incomplete and the goal active.

## Native background-page controller checkpoint

Rust now handles background-folder selection, captured setup confirmations,
computer-folder setup, start/stop/pause/resume/sync/remove orchestration and job
status presentation. Flatpak permission precedes job/cloud mutation. Active-job
removal pauses before changing configuration. Demo mode blocks service work.
Status polling owns a separate worker and ignores replies from an older action
generation or a different page. Controller destruction joins both workers.

Verified: 25 desktop Rust tests and three real QML checks, including an actual
synthetic local job's add/start/pause/remove/stop cycle through the native Qt
controller. The core job/computer/sync runtimes are unchanged in this checkpoint.
Headless process, combined startup/CLI stop, diagnosis cancellation and real
online/offline foreground FUSE tests also pass. Clippy, formatting and whitespace
checks pass. No live account test, full application rerun, portable rebuild,
installation or release occurred. Main-window wiring, mount/trash/update pages,
window lifecycle, packaging and Python removal are still incomplete.

An isolation defect was found in the earlier headless/FUSE test scripts: they
set XDG_DATA_HOME although the core stores state under XDG_STATE_HOME. The tests
therefore were not fully state-isolated despite prior documentation. Two newly
created foreground caches were verified to contain only the known synthetic
root/data metadata and three-byte `abc` objects. They were moved, under exclusive
cache locks, to `/tmp/nd-isolation-test-cache-_92vvawy`. Existing service files'
modification times predated these tests; no deletion of those files was done.
All service/FUSE/QML-service checks now set XDG_STATE_HOME and assert their private
state paths. A corrected full rerun passed, and comparison confirmed user service
and foreground file lists, sizes and modification times stayed unchanged. See
`/tmp/nd-user-state-isolation-validation.json`. The migration goal remains active.

## Native mount and local-trash UI checkpoint

Rust now implements mount selection/options, all prior mount actions, recovery
status rendering, saved-configuration display and background status polling.
Snap/demo restrictions are preserved. Local-trash listing/restoration uses the
native journal with an explicit own-journal boundary; unsuccessful nested restore
results cannot become UI success. Receipt eligibility and post-success refresh
match the prior interface. The native core is unchanged in this checkpoint.

Verified: **30 desktop Rust tests and four QML checks**, plus the existing native
CLI and foreground-FUSE checks. The new QML mount test uses a private service and
synthetic SDK: mount, pin, managed eviction, re-pin, unmount, offline mount/pin and
unmount, followed by empty local-trash listing. It confirms exactly two synthetic
downloads, retained `abc` content, joined workers and no remaining kernel mount.
Its initial expectation of pinned-cache eviction failure was incorrect: the
existing manager deliberately unpins first. The test now verifies that behavior.

A real filesystem/journal test covers cancellation, destination collision without
overwrite, retained trash payload, recovery listing, successful retry and rejection
of a second restore. Demo/Snap restrictions, field propagation and UI restore
eligibility are also checked. Clippy, formatting and whitespace checks pass. A
post-run comparison found no changes to user service/foreground file lists, sizes
or mtimes. No live account, release, installation, full application-suite rerun or
portable rebuild occurred. Main-window wiring, updater/autostart/window lifecycle,
packaging and final Python removal remain, and the continuous goal stays active.


## Native autostart UI checkpoint

The native file controller now routes autostart enable/disable to Rust. Flatpak
requests the desktop portal and treats denial as failure. Snap uses its stable
launcher without retaining an SDK path from a previous revision. AppImages use
the persistent outer executable; an SDK inside APPDIR is rediscovered at login,
while a custom external SDK is retained. Native runs use the current executable.
Existing Rust managed-entry protection and Desktop Exec escaping remain in use.
The legacy Python launcher fallback is never called by this native controller.

Validation: 34 desktop Rust tests, four QML checks (including native autostart
enable/disable), Clippy with warnings denied, formatting, headless service checks
and online/offline synthetic FUSE checks pass. Autostart writes use an isolated
XDG_CONFIG_HOME, and all daemon/mount state uses isolated XDG_STATE_HOME. User
service/foreground file names, sizes and mtimes match the prior baseline.

The complete application migration remains active. Production Python frontend,
update controller, window lifecycle, main-window wiring, portable packaging and
final parity checks are still outstanding. No release, installation or live
account test was performed at this checkpoint.


## Native update-controller checkpoint

The Rust update model and Qt controller now handle saved automatic/download/preview
preferences, release-channel defaults, the 24-hour check cadence, manual checks,
verified downloads, release notices and explicit installation. The existing Rust
HTTPS/update runtime remains authoritative for selection, downloads and digest
verification; nested errors cannot become UI success. QSettings uses the existing
INI keys. Package format still accepts the installed payload directory from the
future main-window bootstrap.

Download results own their cleanup, including queued results discarded at close.
An explicit install re-verifies the package, reads its Package field with bounded,
cancellable dpkg-deb output and rejects anything other than neutrondrive before
invoking pkexec/apt-get. Once the privileged transaction starts it is not killed;
shutdown returns false while installation is active so the future window binding
can defer close. This must be honored when integrating the actual main window.
A running file operation prevents installation. Failure retains the download for
retry, while success removes it. Automatic download never starts installation.

Validation: 43 Rust desktop tests, five QML checks, Clippy with warnings denied,
format checks and the existing isolated service/online/offline FUSE tests pass.
A synthetic DEB is built and inspected without installing it. The QML test covers
translated status, saved/reloaded options, demo guards and repeated shutdown.
No live release request or privileged installation was performed. User service
and foreground-cache file names, sizes and mtimes remain unchanged.

Two compile issues (Rust pinned-borrow lifetimes and a generated binding type-name
collision) were corrected. A subsequent /tmp user-quota failure was resolved by
removing only regenerable task-owned Bun cache, a duplicate Qt SDK copy and a
failed linker temporary. The successful final log is
/tmp/nd-qt-updates-final-2.log. The Qt SDK still in use is /tmp/nd-qt-sdk.

The overall goal remains active: main-window/controller wiring, update timers,
window/single-instance lifecycle, Qt translations, installed resource discovery,
portable builds/package wiring, full parity tests and production Python removal
are still outstanding. No release or local installation was made.

## Shared main-window native checkpoint

`desktop/ui.qrc` embeds the unchanged existing Main.qml and DriveIcon.qml.
NativeShell forwards their existing controller interface to Rust files,
preferences, appearance and update objects. File dialogs, navigation, all six
pages, update timers/notices and manual theme/language preferences are connected.
Clipboard effects now call Qt's actual clipboard. Qt bindings also expose a
window snapshot for rendered verification.

Close policy is implemented in Rust: explicit installation defers closing; busy
file work asks for confirmation and cancels before closing; stale confirmation
tokens do nothing; a Flatpak daemon-status check hides the window while background
sync runs. Closing proceeds on the next event-loop cycle. The current native
window entrypoint is an isolated demo validation command; regular GUI startup,
resource discovery, Qt translations and single-instance activation still need
integration before production Python can be removed.

The first main-window run printed its UI marker but failed afterward in a QML
AOT call on the timeout path. Honoring the boolean shutdown return and deferring
the close callback removed the observed crash. The remaining timeout exposed an invisible
Window parented to the root Item, which had no window itself. Creating the view
as a top-level window fixed visibility and close events. The folder-dialog test
also now closes the actual dialog, and final close is deferred out of onClosing.
The initial internal-content-item screenshot approach was unsupported by Qt;
QQuickWindow::grabWindow now supplies the verification images.

Final evidence: 45 Rust tests, six QML checks, Clippy/format/whitespace checks,
headless-service and synthetic online/offline FUSE tests pass. The script requires
successful process exit after the main-window marker, and verifies both PNGs.
Light file-list and dark settings captures were visually inspected. Final log:
/tmp/nd-qt-main-window-final-2.log. User service/foreground file names, sizes and
mtimes remain unchanged. No live account, privileged installation, release,
portable rebuild or production frontend switch occurred. The full goal remains
active, not complete.


## Native GUI bootstrap and window-instance checkpoint

The native executable now accepts regular GUI startup, --demo and --cli PATH.
The headless modes still dispatch before constructing QGuiApplication. GUI
startup resolves installed-prefix or source-checkout catalogs, preserves explicit
SDK/environment/PATH selection, displays settings when the SDK is unavailable,
sets the existing desktop-file identity and retains Qt translators for qtbase
and qtdeclarative. Missing graphical sessions fail clearly with exit status 1.

A Rust window-instance owner retains the existing QLockFile format and per-scope
qt-window-{demo,flatpak,snap,native} socket paths. It rejects symlinked/non-private
directories and foreign socket-path objects; removes only an owned stale socket;
and checks socket identity before cleanup. Activation connects are nonblocking
with a bounded retry, and the Qt timer drains a bounded number of pending
connections to show/raise/activate the existing window. Socket cleanup precedes
lock release. Installed resource paths take precedence over source fallback.

Validation: 51 Rust tests, six QML checks and all existing native service/FUSE
checks pass. A separate process test starts the regular --demo entrypoint,
contacts it from a second process, kills only that synthetic demo owner to
simulate a crash, and verifies stale-lock recovery followed by clean window
shutdown. Symlinks, public directory permissions, regular-file socket collisions
and replacement-path preservation have dedicated tests. No account or installed
package was used. The user service/foreground state file list, sizes and mtimes
remain unchanged. Final log: /tmp/nd-qt-startup-resources-final.log.

Production/package launchers still use Python, and portable frontend/core builds,
package dependency changes, complete integration parity, localized launcher
errors and final Python removal remain outstanding. The overall goal is active.


## Python-free package payload preparation

The Qt extractor now has an explicit native mode: it stages Qt libraries,
plugins, QML and translations without PySide6/Shiboken bindings or Python files.
The original mode remains intact until package migration is complete. A native
shell launcher supplies relocatable Qt/SDK paths, preserves literal arguments
and executes the Rust binary. Native staging requires a fresh destination,
retains the locale payload identity used by update detection, and includes the
Rust/Unicode/Qt notices. Native sandbox staging deliberately fails until the
browser-portal helper is migrated, rather than silently shipping Python.

`packaging/native_frontend.py` verifies the actual ELF architecture, GLIBC and
GLIBCXX requirements, unwanted Python/separate-core dependencies, binary hash and
first-party source provenance before staging. The host development binary requires
GLIBC 2.39 and is correctly rejected for the current portable baseline. No
portable artifact or release has been produced.

Five new packaging tests and five existing AppImage-cache tests pass. A separate
host-only fixture built from the pinned Qt wheels contains no Python runtime
files or Python Qt bindings. Its native launcher successfully ran the full
main-window check with a PATH containing only readlink/dirname and the native
application directory; the process exited 0. The fixture is marked
HOST-VALIDATION-ONLY.json and is not a distributable package. Log:
/tmp/nd-native-payload-window.log.

Next requirements are the portable frontend build, Rust browser-portal and
AppImage cache/bootstrap helpers, architecture verification and the default
package/launcher/dependency switch. Production launchers and the installed app
remain unchanged. The overall migration goal remains active.


## Native browser-portal helper checkpoint

Rust now implements the desktop OpenURI request and the headless --open-uri URL
entrypoint. Only bounded valid HTTP(S) addresses are accepted; whitespace,
controls, backslashes and unsupported schemes are rejected. Fragments are
preserved. The client subscribes before issuing the request, checks the reply's
unique sender and returned request path, ignores unrelated signals, and treats
denial, invalid paths and disconnection as failure. The existing bounded runtime
limits waiting to 120 seconds and supports cancellation.

Native sandbox staging now selects xdg-open-portal-native instead of the Python
Gio helper. Its shell wrapper passes a single literal URL to the Rust executable,
sets the package Qt-library path and rejects wrong argument counts. Legacy
staging remains unchanged until the complete package switch.

Validation: 162 core tests, the isolated D-Bus suite (11 scenarios, including
browser success/spoof/denial/invalid-path/cancellation), 52 desktop Rust tests,
six QML checks, the existing service/FUSE/window-activation checks and six native
packaging tests pass. Core and desktop Clippy deny warnings. The browser helper
rejects a file URL without constructing QGuiApplication, tested with an invalid
Qt platform and no display. No actual browser/account request, release or
installation occurred. User service/foreground file names, sizes and mtimes are
unchanged. Logs: /tmp/nd-portal-rust-tests.log,
/tmp/nd-portal-core-clippy.log and /tmp/nd-native-browser-qt-tests.log.

Remaining requirements include the Rust AppImage stable cache/bootstrap,
portable/architecture builds, and package/launcher/dependency replacement.
Installed autostart must also resolve its Qt libraries without inheriting a
launcher environment (for example a verified relative ELF RUNPATH). The updated
core invalidates earlier portable-core provenance; rebuild it before final
package checks. The continuous goal remains active.

### Native AppImage bootstrap and direct ELF startup

The Rust core now materializes a private, content-addressed AppImage cache with
an exclusive no-follow lock, manifest checks, atomic staging, and refusal to
replace damaged caches. Five new unit tests cover reuse after source removal,
concurrent starts, symlinks, unsafe paths, permissions and failed publication.
The native frontend handles bootstrap before Qt or account/service access;
AppRun-native forwards original arguments into the cached runtime. The real
binary re-exec test also covers a symlinked XDG cache parent and corrupt-cache
rejection. The existing production AppImage builder has not switched yet.

The frontend now embeds `$ORIGIN/../qt/lib` as its ELF RUNPATH. A relocated
binary starts without LD_LIBRARY_PATH or Qt environment settings; package
provenance rejects binaries missing that relative path. The complete native
suite passed (52 desktop Rust tests, six QML checks, headless service/FUSE tests,
window activation/crash recovery and both new startup checks). Core Clippy
passed. Logs: /tmp/nd-native-appimage-final-tests.log,
/tmp/nd-appimage-rust-tests.log, /tmp/nd-appimage-core-clippy.log.

Portable Ubuntu 22.04 compilation is in progress in an isolated rootfs. Host Qt
code generators are separate from the target compiler/linker and runtime;
release ABI validation remains mandatory. Package defaults, Python removal and
full parity validation remain unfinished. No release or installation occurred.

The full core regression also passed: 167 unit tests, both desktop-bus
entries, both desktop-runtime entries, and 14 runtime integration tests.
Log: /tmp/nd-appimage-all-core-tests.log.

A final cache hardening uses nonblocking no-follow opens, so a cached file
replaced by a FIFO fails validation rather than hanging startup. The additional
regression passed; all 168 current core unit tests and Clippy passed, as did the
rebuilt complete native frontend suite. Latest logs:
/tmp/nd-appimage-hardened-core-unit.log,
/tmp/nd-appimage-hardened-core-clippy.log,
/tmp/nd-native-appimage-hardened-tests.log.

The initial isolated x86_64 release build succeeded and its real ELF requirements
were GLIBC 2.34 / GLIBCXX 3.4.11 with the expected relative RUNPATH. Its version
command also ran on Ubuntu 22.04 without Python or graphics. This initial binary
predates the FIFO-only change; the updated release build must finish before
recording/staging the final artifact. The temporary build helper is
/tmp/nd-build-native-jammy. scripts/build-native-qt provides the general offline
build/provenance entrypoint for an already prepared baseline environment and
rejects source changes during compilation.

The updated portable release build has now completed successfully. The verified
artifact is build/native-qt-jammy-x86_64/neutrondrive-qt with native-build.json
recording current source hashes, compiler and SDK lock. Actual requirements
remain GLIBC 2.34 / GLIBCXX 3.4.11. Both --version and the full native QML window
check exited successfully inside Ubuntu 22.04.5 with isolated preferences and
no Python on PATH. Logs: /tmp/nd-jammy-native-final-build.log,
/tmp/nd-jammy-native-final-start.log, /tmp/nd-jammy-native-final-window.log.
Only x86_64 has been checked; package defaults and Python runtime removal remain
open. The original installed application and release artifacts are unchanged.

The verified artifact was staged with the current-source-provenance-checked
computer-backup SDK into build/native-package-runtime-check/usr. The staged
runtime contains no .py/.pyc/.pyi or Python/PySide/Shiboken libraries. Its native
binary passes version and full window checks without LD_LIBRARY_PATH, Qt/QML
path overrides, or Python on PATH; bundled qt.conf/resources suffice. Reproduce:
`python3 desktop/tests/check-staged-runtime.py build/native-package-runtime-check/usr`
(the single-instance socket needs local socket permission). Log:
/tmp/nd-staged-native-runtime-check.log. The selected pre-existing user service
and foreground files still match their size/mtime baseline. This is a validation
payload, not an installed package or release. Continue with package defaults,
AppImage builder switch, other architectures, and legacy runtime removal.

### Package defaults switched to the native executable

Common staging now always uses the verified Rust/Qt frontend; requesting the
removed Python staging mode is rejected. It needs only the language catalogs
under src/neutrondrive and no Python entrypoint. Source-built SDK provenance is
included alongside the frontend report. DEB/RPM dependencies drop Python and
PyGObject. Actual Qt DT_NEEDED inspection also identified GSSAPI and Brotli;
these are now explicit DEB/RPM/Snap dependencies rather than incidental ones.

A real x86_64 DEB was built at
build/native-installer-check/neutrondrive_0.6.0~alpha6-1_amd64.deb. Streaming archive
verification checked all 915 regular entries, root ownership, absence of Python
runtime/bindings/shared core, and exact hashes of the frontend, SDK, launcher and
both build reports. Control dependencies contain no Python. Evidence:
/tmp/nd-native-deb-final-build.log and /tmp/nd-native-deb-verification.json.
Seven native packaging tests, the portable launcher test and version consistency
test pass. The version test now reads the Rust Cargo version, not the Python app.

Snap preparation and Flatpak recipes now pass a verified native frontend and
retain SDK provenance. Flatpak's smoke command uses the actual native window
check instead of importing PySide. These complete format builds are still
pending. The AppImage Python bootstrap was removed; AppRun invokes Rust and the
builder requires a current native DEB, verifies its frontend provenance, omits
Python/GI bundling, and preserves original Qt/SDK/frontend bytes after
linuxdeploy. --work-dir supports a local filesystem when the checkout lacks
symlink support. Full AppImage build verification is still pending.

Remaining work includes adapting CI to produce native frontend artifacts,
validating all package formats/architectures, updating the AUR release recipe
when appropriate, removing the legacy application Python sources/entrypoints,
and completing the function-by-function parity audit. Existing published
releases, installed software and account state were not changed.

The native Flatpak bundle also builds successfully. The bundled executable passed
--diagnose and the native QML window check inside the installed GNOME 50 runtime
with isolated state/config/cache and network disabled. Artifact:
build/native-installer-check/neutrondrive-0.6.0a6-x86_64.flatpak. Log:
/tmp/nd-native-flatpak-build.log. No application installation was performed.
The Snap project at build/native-snap-project-check verifies both frontend and
SDK provenance against the copied sources; Snapcraft is unavailable on this
host, so this is preparation validation, not a completed Snap build.

The native RPM builds inside the isolated Ubuntu rootfs. Its private Qt/ICU
libraries no longer advertise system-wide RPM capabilities or require system
copies; Secret Service uses the shared-library ABI capability across distro
package names. RPM digest validation, exact frontend/SDK/launcher/report hashes,
root ownership and absence of Python runtime/dependencies pass. Artifact:
build/native-installer-check/neutrondrive-0.6.0~alpha6-1.x86_64.rpm. Evidence:
/tmp/nd-native-rpm-verified-build.log and /tmp/nd-native-rpm-verification.json.
This verifies assembly/integrity, not installation on Fedora or openSUSE.

RAM-backed /tmp repeatedly reached its per-user quota during package assembly.
Further large builds should use the task-owned SSD directory
/var/tmp/neutrondrive-packaging.VaTl2Y (also recorded in
/tmp/nd-package-work-root), bound as /package-tmp in the isolated rootfs. It is
outside the default writable roots, so use an approved escalated build command.
Do not delete other users' temporary data. Existing rootfs/tools remain at
/tmp/nd-jammy-root; Python and RPM installed there are build tools only.

### Native source entrypoint and localized interface checks

`bin/neutrondrive` now starts the Rust executable directly with the bundled Qt
library/import paths. The setuptools application package and Python
`neutrondrive.__main__` entrypoint have been removed. Remaining compatibility
bindings also launch the native service rather than `python -m neutrondrive`;
the Rust desktop-bus launcher no longer injects PYTHONPATH. Its isolated bus and
runtime tests pass (`/tmp/nd-native-launcher-bus-tests.log`). Eight source/package
launcher tests and the Rust-based version consistency check pass.

Native help and no-display diagnostics use the existing language catalogs with
English fallback. `check-cli-localization.py` exercises all 25 languages and an
unsupported locale in separate native processes, with no Python on PATH and no
state/config/cache writes. `check-window-localization.py` checks the actual Qt
window's selected language and translated title. The former Python-based
`test_i18n_ui.py` now invokes this native process test rather than PySide.

The main-window smoke additionally sends actual Qt keyboard/pointer events and
checks focus, select-all actions, retained delegate identity and folder opening
by double-click. Pointer injection uses the exported Qt Gui entrypoint used by
Qt Test; directly sending QMouseEvent to the window did not exercise Qt Quick's
native pointer path. The focused corrected test passes. Full combined test
results are recorded separately after completion. See NATIVE-UI-PARITY.md for
remaining view-level assertions before retiring the old controller tests.

The debug target and isolated portable root/target have moved to the task-owned
SSD directory; /tmp paths are symlinks for existing helpers. This prevents the
observed per-user RAM quota failure. Portable binaries and installer artifacts
recorded at the previous checkpoint are now stale and require a fresh source
provenance check and rebuild. No account test, release or installation occurred.

The combined native suite now passes: 54 Rust tests, Clippy with warnings denied,
formatting, all 26 CLI locale cases, all 26 actual window locale cases including
keyboard/mouse checks, stable AppImage cache/direct loader, all six QML checks,
headless services, real FUSE, autostart, mount/trash cleanup, update controller,
and window activation/crash recovery. Log:
/tmp/nd-native-input-and-locale-final.log. Previously recorded user-state file
sizes/mtimes remain unchanged. This establishes the source-entrypoint checkpoint,
not completion of the remaining Python/controller/CI cleanup.

### Python application runtime retired

The old `app.py`, `appearance.py`, `update_controller.py`, `qt_instance.py` and
`demo.py` were deleted after replacement native tests passed. No Python module
or generated bytecode remains under `src/`. The former non-UI C ABI/DTO adapters
were explicitly relocated to `tests/core_contract` so existing fault-injection,
fixture and differential-policy tests remain available. This relocation is test
maintenance, not a claim that moving Python files implements Rust functionality:
the actual production implementations already live in `core/` and `desktop/`
and are exercised by native executable tests. No application entrypoint or UI
controller was retained in the test adapter package.

The opt-in live smoke tools now import that test-only API. Source launchers and
installed payloads do not import it. The language catalog coverage test scans
Rust translation calls instead of deleted Python modules. A packaging regression
rejects Python modules in application sources or a restored setuptools entrypoint.

Actual window checks now also verify scrollable background/online pages, live
system-theme fallback without losing selection, a prompt's literal original
filename, and Unicode/placeholder text passed through the real Rust worker to a
local SDK fixture. The share dialog's copy action is checked against the actual
Qt clipboard. The fixture has no network behavior, and update polling is disabled
in this test mode. These checks pass together with the full native suite,
including private-bus portal contracts, real FUSE and instance crash recovery.
Log: /tmp/nd-native-python-ui-retired.log.

The full retained contract suite is running against a freshly built native core,
with all Qt/D-Bus/FUSE opt-ins and isolated state/config/data/cache. Completion
must be recorded after it exits. CI still needs native frontend artifact jobs and
fresh package integration; previous portable artifacts remain stale. There was
no release, installation or live-account test.

The retained contract suite completed successfully: 341 tests, no failures or
skips, with Qt, D-Bus and FUSE enabled. Log:
/tmp/nd-contract-after-runtime-removal.log. The separately rerun native packaging
suite has nine passing tests, including the new source/runtime boundary check.
Previously recorded user-state file size/mtime metadata remains unchanged.
A new portable frontend build is running in the isolated Ubuntu 22.04 rootfs;
its source snapshot is /tmp/nd-native-current-expected.json and its log is
/tmp/nd-native-runtime-retired-portable-build.log. Verify source identity and
actual ABI/startup before recording or staging this artifact.

### Portable SDK and native package checkpoint

The current Rust frontend was rebuilt using the repository build script with
Qt 6.10.2 qmake/moc compiled inside Ubuntu 22.04. Both generators were verified
against the pinned source archive and binary hashes. The rebuilt frontend and
staged runtime pass version and actual-window checks on the baseline system.
The fresh DEB passes archive ownership, dependency and provenance checks, and
was installed and exercised inside an isolated Ubuntu 22.04 rootfs. The user's
installed application was not changed. Current local Arch recipe generation
also passes frontend and SDK provenance checks. Twelve focused packaging and
Qt generator tests pass.

CI now defines native frontend and current SDK artifacts for x86-64 and ARM64,
then DEB, RPM, Flatpak, Snap, AppImage and Arch package jobs. Actionlint passes;
remote CI execution and the remaining current-source package tests are still
pending. Older RPM/Flatpak checks do not validate the current source snapshot.
Python remains in build and regression-test tooling, not the application runtime.

Current-source RPM, Flatpak and AppImage validation is now available. RPM
archive hashes and private-library dependency isolation pass. The Flatpak
bundle was exported after diagnostics and actual QML checks inside GNOME 50.
The complete AppImage builder, extracted native runtime, and persistent-cache
window checks pass. Build-tool preflight now detects missing dependencies before
assembling an AppDir. The cached-window theme fixture is isolated from the host
D-Bus portal; otherwise the user's actual system theme correctly overrides its
synthetic fallback value. User-state metadata remains unchanged. ARM64, Snap
and Arch installation remain pending remote CI.
