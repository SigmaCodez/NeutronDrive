# Native runtime integration

`core/src/bin/neutrondrive-runtime.rs` is a Rust executable for the versioned
runtime protocol. It uses the same implementation as `neutrondrive_backend_call`;
there is no Python subprocess or Python fallback. The Proton SDK remains an
external, pinned executable for authentication, cryptography and remote access.
The production desktop application, controllers, daemon, online filesystem and
mount manager now run in Rust, retaining Qt/QML and the pinned Proton SDK. The
source launcher is a shell wrapper; installed payloads contain no first-party
Python runtime. Python build tooling and regression adapters are not installed.

The API descriptions below include historical migration notes. References to
Python ownership, temporary bindings or remaining launcher work describe earlier
checkpoints, not the current production path. Retained compatibility adapters
now live exclusively under `tests/core_contract`. See
[Rust migration status](RUST-MIGRATION.md) and
[native UI regression coverage](NATIVE-UI-PARITY.md) for current validation,
including x86-64/ARM64 CI, installed packages and authorized live smoke tests.

Build and test from the `core` directory so Cargo uses the checked-in vendor
configuration:

```sh
cd core
cargo test --locked --offline
cargo build --locked --offline
./target/debug/neutrondrive-runtime --version
```

If `CARGO_TARGET_DIR` is set, the output is under that directory instead. The
existing package build enforces the supported GLIBC baseline. A successful local
build on a newer Linux distribution does not establish portable-package support.

The process reads one UTF-8 JSON request from stdin, with a maximum size of 64 MiB.
It writes newline-delimited JSON: zero or more `{"event": ...}` records followed
by one `{"ok": true, "result": ...}` or `{"ok": false, "error": ...}` reply.
An unsuccessful reply also exits nonzero. Login events require the caller to
present the verified URL to the user. SIGINT/SIGTERM cancel active operations;
the SDK process group is terminated and reaped before returning. Never archive
stdout as a diagnostic log: successful results can contain account/file metadata.

A read-only local validation example:

```sh
printf '%s\n' '{"version":1,"op":"validate_remote","path":"/my-files"}' \
  | ./target/debug/neutrondrive-runtime
```

A Drive operation has the following shape (the SDK executable must be installed):

```json
{
  "version": 1,
  "op": "drive",
  "executable": "/usr/lib/neutrondrive/bin/proton-drive",
  "operation": {"action": "list", "path": "/my-files"}
}
```

Operation fields are defined by `drive::Operation`; unknown fields are rejected.
SDK results are validated before exposing nodes. Restore re-lists the trash and
rejects ambiguous names. Upload/download conflict strategies retain their existing
allowlists. The native event transaction never acknowledges a rejected delivery.
The desktop binding propagates callback exceptions after native cleanup.

The executable is currently a development integration target. Package launchers,
Qt/PySide6, FUSE and daemon ownership have not yet switched to it. Removing Python
from a package now would break those features.

Descriptor-based operations (`filesystem_read`, `filesystem_entry`,
`filesystem_scan`, `trash_snapshot`, `stage_upload`, `install_download`) use file
descriptors in the calling process. C ABI callers keep those descriptors alive
until the synchronous call returns. The native executable can only use descriptors
explicitly inherited into its own process; numeric descriptors from another
process are not transferable handles. Poll callbacks provide cancellation points
inside hashing and copying. A `local_backup` callback records the recovery path
before installation proceeds. `computer_setup` performs the guarded registration
transaction and returns the persisted job; it does not delete remote objects when
interrupted.

`localization` accepts a typed `operation` for locale selection, bounded catalog
loading, placeholder validation, translation or diagnostic substitution. It never
formats captured values as executable templates. `control_command` accepts a Unix
socket `path`, `action`, nullable `job_id` and a finite timeout in seconds (greater
than zero and at most 300). It authenticates the peer UID and shares one deadline
across connect, send and receive; malformed/incomplete responses fail closed.
The native control client does not replace the daemon's Python lifecycle/server.

Server-side `control_receive` and `control_respond` borrow an accepted socket
`fd` for one synchronous operation. They verify AF_UNIX/SOCK_STREAM and the peer
UID, duplicate the descriptor with CLOEXEC, and preserve the caller's descriptor
and status flags. Receive accepts one newline-framed, typed request of at most
32 KiB. Send bounds replies to 8 MiB and suppresses SIGPIPE. Nonblocking syscalls
and polling enforce a total deadline for each receive/send, including slow peers.
The Python service still owns accept, dispatch and lifecycle; JSON framing and
socket I/O no longer run in Python. Unknown request fields and non-string action
or job identifiers are rejected before dispatch.

`echo_matches` accepts an absolute `root`, a safe `relative` path and the settled
`snapshot` map. It returns a boolean using stat-only observations, without hashing
or modifying files. File identities, sizes, mtime and ctime must match; folder
subtrees must contain exactly the expected non-reserved entries. Symlinks,
unsafe mount boundaries, traversal errors, excessive nesting and changed root/parent identities
return false so the caller schedules reconciliation rather than hiding an edit.
This is an event-suppression check, not an atomic guarantee against future edits.

`trash_journal` exposes typed `lock`, `unlock`, `read`, `names` and `save`
operations. A successful lock returns an opaque process-local handle, not a file
descriptor. Always unlock it in a caller-side finally/RAII scope. Active operations
retain native ownership through concurrent handle removal. The lock wait is
bounded and cancellable; a timeout of zero attempts one immediate acquisition.
Save uses exclusive creation for initial receipts (false means already present)
and atomic replacement for updates, fsyncing file and directory. Before and after
I/O, pinned journal directory identities are checked against the current paths.
Journal reads require private regular files owned by the current user with a
single hard link. These operations manage recovery metadata; they never unlink
user payloads. The single-request development executable cannot retain a handle
across separate process invocations; use the library API for transactions.

`trash_restore` accepts journal `state`, canonical receipt `identifier` and a
nullable absolute `destination`. It owns the journal transaction for the entire
restore and returns a result containing `ok`, the receipt `record`, and optional
`error`/`cancelled` fields. It emits a `trash_receipt` event so host callback
exceptions can retain recovery metadata. The binding must process that event
even after a poll callback fails. No payload is overwritten or deleted: restore
uses same-filesystem RENAME_NOREPLACE and preserves conflicting destinations.
An interrupted rename/commit is reconciled from the recorded parent identity and
full content proof before reporting restoration as complete.

`trash_list` accepts journal `state` and returns registered receipts. It owns and
releases its lock, including on cancellation, and classifies payload availability
without modifying receipts. Unrelated journal filenames are ignored.

`trash_location` accepts absolute `root` and `data_home`, the source `device`, and
`desktop` preference. It returns `path`, `backend` and nullable mount `top` for
trashinfo path encoding. It creates private same-filesystem directories with
parent fsync and never follows symlinks or falls back to copy-delete.

`trash_init` accepts an absolute journal `state` and prepares private state and
entries directories. `trash_move` accepts a typed `request` containing `state`,
`data_home`, `desktop`, borrowed `root_fd`, absolute `root`, safe `relative`,
`expected`, nullable complete `expected_tree` and nullable canonical `identifier`.
The native transaction owns the journal lock and descriptor duplicates. It emits
`trash_validate` after the final content hash, before cheap identity checks and
rename; the host must synchronously perform its final remote/account proof and
return failure on any uncertainty. `trash_receipt` delivers recovery metadata.
The result follows `trash_restore`'s `{ok, record, error?, cancelled?}` contract.
No hash runs between successful remote validation and the atomic move. Errors
after the move trigger no-overwrite rollback into the still-verified parent or
retain the payload with recovery evidence. Failed final persistence leaves the
previously durable prepared receipt available for reconciliation.

`sync_remote` accepts an `operation` (`entry`, `node_entry`, `root`, `scan` or
`lookup`) and an optional SDK `executable`. With an executable it performs SDK
requests entirely in Rust. The caller must already own serialized SDK lifecycle
coordination; this adapter does not share the Python backend mutex or caches.
Without an executable, the request emits synchronous `host_request` events with
`id`, `method` (`info`, `list_nodes`, `list_nodes_fresh`) and `arguments.path`.
The host answers by a nested `host_reply` call containing that `id` and normalized
node `value`. Replies are single-use, scoped to the current callback and OS thread,
and rejected after the callback returns. Exceptions must return nonzero; missing
responses fail closed. `poll` events preserve cancellation. This host adapter is
transitional; native SDK services already implement the same interface.

`sync_state` accepts typed operations: `prepare {state}`, `key {root,uid}`,
`load {state,key,root,uid}` and `save {state,key,value}`. Keys are lowercase
64-character SHA-256 names; arbitrary file paths are not accepted. Load returns
`entries`, raw-byte `digest`, `pending` and nullable root `identity`. A missing
file returns empty entries/digest/pending and null identity; malformed or unsafe
files fail without changing bytes. v1 identity entries are retained as legacy
identity-version 1 in memory. Save validates v1/v2 schema and the 32-MiB limit
before the private atomic-write implementation fsyncs and renames the state file.
The sync execution owner must retain its process lock around state mutations.

`sync_proof` accepts `roots`, `remote` and `preflight` operations with a plan
identity containing `local_root`, `remote_root`, `remote_root_uid`,
`root_identity` and `baseline`. Preflight borrows `root_fd` and accepts a safe
`relative` plus `direction` (`trash_local` or `trash_remote`). It returns the
verified target subtree and performs no mutation. `remote` confirms trash,
unchanged UID/revision/content and original parent location; absence alone is
insufficient. Folder deletion remains blocked.

`sync_plan` accepts `state`, absolute local `root`, `remote_path` and optional
SDK `executable`, returning the complete preview data. Both commands support the
same direct SDK or synchronous host-services boundary as `sync_remote`.
`try_info` is an additional host method: return `{node: ...}` on success or
`{unavailable: true}` on metadata lookup failure, while cancellation must still
abort the callback. It lets native proofs distinguish missing metadata from
confirmed trash without treating an unavailable object as deleted.

`sync_execute` accepts `state`, the complete `plan` returned by `sync_plan`,
`trash_state`, `trash_data`, `trash_desktop` and optional SDK `executable`. It owns
its process lock for the full execution and returns result lists plus final local
and remote snapshots. The caller must enforce preview ownership and single use.
In callback mode it emits `sync_started` only after obtaining the process lock,
so contention leaves the preview reusable. Direct SDK mode performs all operations
without Python. The Python adapter retains backend serialization/cache invalidation
while GUI/daemon callers migrate.

Host services now also support `mkdir`, `trash`, `upload` and `download` with the
same argument fields as their typed Drive operations. `host_failure` answers an
outstanding request with `id`, `message` and `cancelled`; it has the same thread,
single-use and lifetime restrictions as `host_reply`. Recoverable SDK failures
become per-file errors, while cancellation aborts execution with durable state
preserved. A host interpreter exception outside ordinary `Exception` still
aborts the callback and is rethrown after native cleanup.

`metadata_cache` exposes `create`, `release`, `info`, `list`, `invalidate`, `event`,
`upload`, `mkdir` and `trash` operations. `create` returns an opaque process-local
handle; all other operations except create take `id`. The owner must release it.
A list operation accepts `path` and `fresh`; events take `roots`, nullable
`node_uid`/`parent_uid` and `reset`. SDK services are invoked outside cache locks.
Generation changes prevent an in-flight old response from becoming a cached
listing. Active calls retain ownership after release; new calls reject expired
handles. Mutation failures still trigger invalidation. This cache stores only
in-memory metadata and does not persist account information.

`event_ancestry` takes event `root_uid`, `node_uid`, `parent_uid` and job snapshots
(`id`, `paused`, `remote_uid`, `account_root_uid`, `known_uids`). It uses the same
SDK host-service callback mechanism and returns `affected` job IDs, nullable
`ancestor`, and `cancelled`. It does not acknowledge cursors or persist/schedule
work itself; the daemon must do those atomically before acknowledging delivery.

`job_roots` takes `jobs` and `selected`, and calls synchronous host `info`
services to prove the complete active set before returning canonicalized `jobs`.
Unsafe identity/ancestry returns `blocked`; absent/paused selection returns
`cancelled`; SDK failures retain normal error/cancellation handling. This
operation does not persist partially validated identities.

`service_controller` takes `state` and an `operation`: `status`, `stop`, or
`start` with an argv array and optional environment overrides. No shell parsing
is performed. The executable is supplied by the caller so native entrypoints
can replace the current compatibility daemon launcher. Startup checks use the
authenticated local control socket; failures terminate the owned child and
report whether termination was confirmed. Status without a reachable service
reads the private saved status and always marks it stopped.

`daemon_transition` takes a serializable job `runtime`, an `outcome` and a
`context` containing monotonic/wall clocks, pause/stop state, local root and retry
settings. Outcomes are `begin`, `complete`, `retry`, `blocked`, `cancelled` and
`finalize`. Rust owns result accounting, retry/backoff, echo snapshot construction
and stat-only filtering of writes received during an active sync. Unknown or
remote-origin follow-up work is retained. The result is the complete updated
runtime value; no partial state is returned if validation fails.

`control_server` owns daemon lifetime resources through opaque handles. `create`
takes `state` and returns a handle. `serve` takes `id` and synchronously emits
`host_request` messages with method `daemon_command`; the caller replies through
`host_reply`/`host_failure`. Native code validates requests, authenticates peers,
and bounds receiving and sending. `stop` ends the accept loop; `release` removes
the handle and also requests stop. An active serve operation retains ownership
through the current reply. Socket/PID cleanup and lock release occur only when
the final operation releases the server. Replaced socket/PID files are preserved.
The command handler and main daemon scheduler remain compatibility bindings at
this checkpoint, rather than an independent native daemon executable.

`daemon_control` takes a typed `action` (`status`, `stop`, `reload`, `pause`,
`resume`, `sync`), optional `job_id`, and `reload_only` for initial configuration
loading without a status response. Rust owns command selection, configuration
updates, runtime reconciliation, watcher restart decisions, cancellation of
removed/paused active jobs and persistence before acknowledgement. Its
synchronous `daemon_state` host requests are primitive state/storage/watcher
operations. The caller must hold its daemon coordination lock for the complete
operation. The Python binding currently supplies those primitives; this does
not yet replace the shared-state owner or scheduler with a native daemon.

`daemon` runs the complete native background service until a control `stop` or
callback cancellation. Its `config` contains `state`, `executable` and optional
`mount_paths`. It owns the shared job state and condition variable, authenticated
control thread, inotify threads, SDK serialization, event polling/durable cursor
acknowledgement, unknown-event resolution and sync worker scheduling. SDK locks
are released before event delivery. While a sync/ancestry worker runs, the main
thread continues polling cancellation so SIGTERM can terminate an owned SDK
process promptly. The standalone `neutrondrive-runtime` accepts this operation
without Python; the existing application entrypoint is temporarily a thin
signal/FFI binding.

`ondemand_io` provides native private-directory, regular-file, stable SHA1,
JSON-journal, legacy-compatible cache-key, verified-open and hydration operations.
`hydrate` checks the dynamic mount root and immutable file revision, downloads
into a private staging directory, verifies ownership/type/link count/content,
rechecks the cloud revision and the staged inode, then durably installs the
object. Filesystem errors are returned as `errno`/`message` so a FUSE binding can
preserve POSIX errors. `regular`, `verified` and `hydrate` transfer an owned file
descriptor to an in-process caller; that caller must close it. `digest` duplicates
its borrowed descriptor and does not close or change the caller's descriptor.
The optional `executable` selects direct native SDK services instead of callbacks.

`ondemand_write` accepts the borrowed edit descriptor, recovery paths, current and
baseline nodes, parent/root identity and replacement intent. It validates the
recovery file identity, performs pre-upload revision checks, durably records the
upload, verifies a staged copy, and checks the acknowledged remote revision and
content. Verified cache installation uses a separate inode and descriptor-relative
rename. The result always carries `after` when the remote commit was confirmed,
even if later local persistence failed; callers must retain that new baseline.
Errors include POSIX `errno` or a `backend_error`/`cancelled` pair. Local data and
a recovery journal remain on ambiguous or failed writes. The Python filesystem
still owns inode/handle tables and persists its directory graph at this checkpoint.

`ondemand_tree` owns online-drive directory metadata validation, persistence and
reconciliation. Its typed `load`, `save` and `reconcile` operations accept or return
a tree snapshot (entries, child-name maps, pinned/listed inode sets and the next
inode). The transport uses decimal JSON object keys for inode-indexed maps;
protocol tests cover deserialization through the native executable. `name` and
`node` use the same filesystem validation as reconciliation. Filesystem failures
return `{errno,message}`; backend validation failures keep the normal error
contract.

Reconciliation preserves inode identities and open snapshots, retains pinned
content revisions while updating their names/paths, filters ambiguous names and
unverifiable nodes, and keeps pending local creates visible on collisions. Saved
v1 metadata remains compatible; unreachable and local entries are excluded.
Loading rejects duplicate inodes/names, missing parents and disconnected graphs
without modifying the original cache. Python currently supplies snapshots from
its remaining inode/handle adapter; this API is also callable directly from Rust.

`ondemand_handles` owns the open file descriptors in an in-process native handle
registry. `adopt` duplicates a verified borrowed read descriptor; `edit` creates
an exclusive private recovery file and optionally copies and verifies a cached
source. The result includes an opaque `id`, a borrowed `fd` for the temporary
writeback adapter, and optional work/journal paths. Callers must not close that
borrowed descriptor. `release` removes the registry entry and closes the native
file even if recovery persistence fails; unresolved edits are retained on disk.
`abort` is reserved for failed initialization before the handle is exposed.

`journal` records write intent; `write` and `truncate` require and durably repeat
that intent before modifying bytes. Native append handling, file sync and
`settled`/release cleanup preserve successful writeback semantics. `read`/`write`
use hexadecimal bytes at the temporary JSON boundary, with POSIX short I/O at
8 MiB per call. Direct native integration can use the owned file instead of this
binding transport. Read handles never delete cache objects. Dirty release never
starts a cloud upload and never removes unconfirmed edits. Python still holds
inode/handle DTOs and coordinates cloud-operation state; descriptor ownership and
local edit lifecycle no longer depend on Python file objects.

`ondemand_replace` owns the editor temporary-file replacement transaction. It
accepts the mount tree, source/target inode, previously observed target revision,
open-inode set and private cache directories, and can use either the direct
native SDK transport or the transitional host-service binding. It verifies the
account/root and target baseline, rejects pending target recovery, hydrates the
verified source, creates an independent private edit, and performs native
writeback. Target metadata is persisted before the source-trash step. A durable
replacement intent precedes source revision validation and trashing; the updated
tree is then persisted before clean release. Errors return the latest `tree`
alongside `errno/message` or `backend_error/cancelled`, so a confirmed remote
revision is never replaced with an outdated local baseline.

The native `ondemand_handles` `recovery` operation lists only inactive unclean
journals, validates private regular data files and fixed hexadecimal basenames,
and rejects malformed journal shapes and traversal paths without touching the
retained data. Neither operation automatically retries an uncertain upload or
replays a recovery journal. Python now only supplies/applies its temporary tree
DTOs for replacement; remaining namespace and mount state still need migration.

`ondemand_namespace` owns mkdir, rename/move, replacement dispatch and trash
transactions. It uses native directory reconciliation and durable tree saves,
checks account/root identity and open inodes, preserves prior target observations
across refreshes, and verifies the source revision immediately before moving or
renaming. Replies contain the latest tree, refreshed inode set and an optional
confirmed replacement `committed` node; listing-only changes never count as a
local write or advance the last-read conflict baseline. Both synchronous host
services and the direct native SDK support rename/move.

Folder removal checks the raw cloud listing count before filtering unsupported or
ambiguous entries. Such entries still make the folder nonempty, even if the
mounted filesystem cannot display them. Errors preserve updated metadata in the
reply without reporting an unconfirmed operation as successful. Python currently
only looks up source inodes, supplies mount DTOs and adapts the native outcome.

`ondemand_fs` owns complete mount state behind opaque native IDs. `create` takes
private cache, remote root, writable/offline flags and metadata TTL; `call` routes
all filesystem operations to a serialized native mount; `close` removes the
mount, preserves unflushed edits, persists metadata and releases the cache flock.
The registry lock is released before taking a mount lock or invoking SDK
services. Native file handles, inode tree, listing lifetimes, account blocking,
last-read observations and writeback state are no longer Python-owned.

Responses carry `value` or the existing errno/backend failure envelope plus a
small status snapshot. `snapshot` is an explicit inspection operation for legacy
DTO consumers; ordinary read/write operations do not serialize the inode tree.
Both direct native SDK services and the transitional host binding are supported.
The Python module adapts DTOs/bytes and keeps a bridge lock for the remaining
desktop status interface. It implements no mount/filesystem transactions.


## Native kernel adapter

`fuse_session` takes `operation: {id, mountpoint, cache, writable}` for an existing
owned `ondemand_fs` mount and optional `executable` for the direct SDK transport.
It blocks until external unmount or callback cancellation. `sync_started` signals
mount readiness, `fuse_status` carries updated filesystem status, and ordinary
poll/host-service events remain synchronous on the invoking thread. Kernel worker
threads own only Rust channels, file/directory handles and reply objects; they
never retain host callback pointers. The adapter uses private default permissions,
no writeback caching, direct I/O and atomic open/truncate. Unacknowledged writes
remain recoverable through the existing coordinator.

`ondemand_fs` also accepts `try_recovery {id}`: it returns `busy: true` without
waiting for an in-flight transfer, or the normal recovery/status envelope.
The desktop mount manager is now native; its remaining Python module only adapts
calls and owns the temporary host callback thread.


## Owned mount manager

`mount_manager` accepts a tagged `operation` and an optional SDK `executable`.
`create {state_dir}` returns an opaque manager ID. `prepare {id, mountpoint,
remote_root, writable, offline}` validates private cache/mount separation, refuses
symlink/nonempty/already-mounted paths, reserves the single native session and
creates the owned filesystem. `run {id}` blocks on the scoped FUSE service pump;
`wait {id}` waits for readiness. The Python thread binding remains temporary.

`status`, `unmount`, `abort`, `forget`, `refresh`, `pin {path,pinned}`,
`evict {path}` and `invalidate {root_uid,kind}` all carry `id`. Relative commands
cannot escape the mount. Account-root changes and tree removal latch read-only
blocking. Status uses nonblocking recovery inspection and remains available
during SDK work and after filesystem closure. Stopped mounts ignore late events.

Rust owns the readiness/termination condition variable, process-wide session
reservation, retained mountpoint directory identity and mountinfo identity.
Startup revalidates the prepared directory; rejected startup releases the cache
and reservation. Normal unmount invokes the mount helper with a deadline and
refuses busy filesystems. The mountinfo check avoids a stale unmount command
when the path already belongs to a replacement mount. `forget` rejects active or
busy managers and invalidates already-retained references before removal.
Error responses retain the existing errno/backend envelope.


## Owned desktop event source

`event_source` accepts an `operation`, optional SDK `executable`, and optional
`host_delivery` (default false). `create {state_dir, roots, interval, dynamic_roots}`
validates configuration and returns an opaque source ID. `prepare`, `load`,
`snapshot`, `poll`, `start`, `run`, `stop {timeout}`, `abort_start` and `forget`
carry that ID. `start` reserves a new run and loads durable state; `run` blocks on
the native scheduler. Stop has independent native cancellation, interrupts the
condition-variable wait and cancels/reaps its SDK child. It is safe within a
consumer callback. Active sources cannot be forgotten.

Production queries use the native SDK transport. Direct native consumers receive
`remote_event` notifications; callbacks remain scoped to the invoking thread.
The temporary binding selects `host_delivery=true`, with synchronous host methods
`event_accept`, optional `event_roots`, and `events_query` only for injected SDK
fixtures. Host failures propagate through the scoped reply protocol; a consumer
exception never becomes an acknowledgement. Error reporting by the scheduler is
redacted. Dynamic-root state is installed only after the new cursor file loads
successfully, and delivered cursors become owned state only after durable commit.

Desktop and daemon cursor filenames use the same historical Python-compatible
ASCII JSON encoding. Snapshot exposes status, roots, file, cursors and running
state without waiting for remote I/O. `events.py` retains only the temporary
DTO/callback/thread binding until the native desktop controller takes over.

## Native HTTPS update transport

`update_runtime` accepts a tagged `operation`: `install_format {payload}`,
`check {current, preview, format}`, `download {release, destination}`, or
`verify {release, path}`. Release records retain `tag`, `url`, `asset_url`,
`digest` and `size`. Results use `{value: ...}` or `{error_kind, message}`;
error kinds are `validation`, `io`, `timeout` and `cancelled`. Diagnostics do
not include raw network responses or credentials. Poll callbacks permit
cancellation before network operations and between chunks.

HTTPS uses pinned, vendored ureq/rustls and the system certificate verifier,
HTTPS-only redirects, bounded connection/read/write/global timeouts, and
proxy discovery from the environment. Release JSON is limited to 4 MiB;
packages are limited to 512 MiB and the selected asset's exact size. Selection
continues to use the native update policy. Downloads require the repository's
release-asset URL prefix and a SHA-256 digest from release metadata.

Rust creates a random private staging directory and exclusive no-follow file,
streams and verifies the digest, fsyncs data/directories and rechecks stable
file and path identities. Failures and cancellation remove owned staging
entries without deleting replaced files. Reverification rejects symlinks,
hardlinks, content changes and parent-path replacement. The fixture CA/key
under `core/tests/fixtures/update_tls` is synthetic test material; production
cannot select it through this API. Installation authorization, timer/settings
and Qt notification orchestration remain in the temporary desktop controller.

## Native desktop autostart files

`autostart` accepts an `operation` with `path`, `command {arguments,
source_launcher, interpreter}`, `quote {argument}`, `read {path}`,
`enabled {path}`, or `set {path, enabled, command}`. Rust chooses the XDG/Snap
configuration path and installed launcher, applies both Desktop Entry escaping
layers, validates owned regular bounded files and rejects symlink ancestors,
hardlinks and foreign entries. Writes use exclusive private random staging,
fsync and atomic rename; creation refuses an entry appearing concurrently.
Flatpak file writes are rejected because its autostart must use the Background
portal. Snap keeps its single wrapper argv[0]. Source-launcher context remains
explicit in the temporary Python binding until the native launcher replaces it.

## Native desktop bus client and portals

`desktop_bus` accepts `operation: {action: "request", request, launcher?}`.
`request` is the existing desktop-service JSON object. An optional `launcher`
contains `argv` and the absolute source directory used by the temporary source
launcher. Rust owns session connection, request/reply validation and limits,
non-activating status queries, timeout/cancellation, package-specific service
names, source-helper startup and eventual process reaping. Only `ServiceUnknown`
and `NameHasNoOwner` allow a retry after starting the helper; standard typed FDO
errors and method-error names are both recognized. NoReply, disconnection,
authorization failure and timeouts never replay mutations. Flatpak cannot start
a host source helper; Snap uses its separate `.SnapService` name. The service's
request dispatcher and mount/event orchestration are still being migrated.

`portal` accepts `background {enabled}` and `appearance`. The Background operation
returns a boolean only after a matching portal Response, with a 120-second total
deadline; cancellation uses the ordinary callback contract. Subscription precedes
the request, so early responses and a changed returned request path are handled.
Response sender identity must match the method reply's unique sender. A forged
signal from another peer cannot grant consent. Errors/timeouts return false;
explicit callback cancellation remains a cancelled native result.

Appearance subscribes before its initial Read, follows owner-verified signals,
and emits `appearance_changed {scheme}` callbacks on the invoking thread.
A valid change received while the initial read is pending wins over that stale
reply. Legacy Settings.Read's double variant layer is accepted; unknown unsigned
schemes fall back to zero, as specified by the
[Settings portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html).
The temporary Qt binding only owns a callback thread and queues the property
notification to the GUI thread. Close cancels native waits; thread-start failure
retains the Qt palette fallback. The Background callback binding similarly
reports no consent if its thread cannot start.

The D-Bus implementation uses pinned vendored zbus 6.1.0 and zruntime 0.2.1.
The manifest's minimum compiler is now Rust 1.89; the project's pinned build
compiler remains 1.98.0. No additional system D-Bus C library is required by
these native operations. Python/Gio remain for the pending desktop server and
temporary main-context callback binding; this is not yet a Python-free app.

### Online-drive service core

`desktop_service` owns a registry of service instances and the mount operation
policy. `create` takes a private `state_dir` and returns an instance ID;
`request`, `status`, `configuration`, `save`, `current` and `forget` use that ID.
The native core serializes mutations independently of status queries, clears
operation state on every exit, validates persisted restore flags, checks overlap
with native sync jobs, rolls back mounts after configuration-write failure, and
keeps event workers reachable when shutdown has not completed. Configuration
uses the same guarded native storage as other service data.

The generic core can use scoped host services for test worker bindings:
`manager_status`, `manager_mount`, `manager_unmount`, `manager_pin`,
`manager_evict`, `manager_refresh`, `events_snapshot`, `events_start`,
`events_stop`, `events_drop`, `save`, `reset_cancel` and `login`.
`save` re-enters the native storage operation; the hook also allows deterministic
write-failure tests. No host callback is retained after a request returns.
The production D-Bus server uses the native implementation described below.
The remaining Python application launcher is still being migrated.

### Native desktop service process

`desktop_runtime` with `config: { state_dir, executable }` runs the production
online-drive service directly in Rust. The session-bus interface and JSON Request
method remain compatible with existing clients. Native and Snap use separate
names; registration neither replaces an existing owner nor permits replacement.
Each request checks the sender's Unix UID. Inputs are limited to 16 KiB and
responses to 8 MiB. A bounded queue serializes mutations while status and cancel
remain independently callable.

The service owns the FUSE session thread, SDK operations, independent event
source, durable configuration and shutdown. Event delivery invalidates the
native mount before acknowledging its cursor. Cancellation of a foreground
service action does not cancel the FUSE session or event watcher. Shutdown closes
the bus, cancels/reaps an in-flight SDK action, stops event polling and releases
the mount. The caller's callback only receives polling notifications on its own
thread and is never retained by worker threads.

The application `desktop_service.py` now only provides launch/client bindings.
The injectable `ServiceCore` adapter and reference Gio server were moved to
`tests/reference_desktop_service.py`; they are not shipped. Production service
control calls the Rust core with a native host implementation. The generic
`desktop_service` callback interface remains available for isolated fault tests.
