# NeutronDrive SDK event bridge

This is a small additive entry point for the MIT-licensed [official Proton Drive
SDK and CLI](https://github.com/ProtonDriveApps/sdk). The exact upstream revision,
Bun compiler version, and honest third-party application identity are recorded
in `source-lock.json`. Upstream authentication, encrypted caches, OS keyring,
cryptography, transfer implementation, and existing CLI command classes are
retained. The bridge does not extract tokens, call private HTTP endpoints, or
implement its own encryption. Metrics and Sentry initialization are disabled in
this entry point. The small `version-branding.patch` suppresses the upstream
CLI update comparison only for NeutronDrive's application identity: replacing
this bridge with an official CLI release would remove event support. It applies
to direct commands and the interactive CLI. The source-built binary identifies as
`external-drive-neutrondrive@0.2.0-alpha`.

## Build

Run from the repository root with the exact Bun version in `source-lock.json`:

```sh
scripts/build-backend --bun /path/to/bun \
  --target bun-linux-x64-baseline --output build/backend/v0.2.0/proton-drive-linux-x64
scripts/build-backend --bun /path/to/bun \
  --target bun-linux-arm64 --output build/backend/v0.2.0/proton-drive-linux-arm64
```

The script fetches and verifies the pinned upstream Git commit, copies the
bridge source and test files into that isolated build checkout, applies the version
patch, installs the frozen Bun lockfile, runs event and branding tests plus the
complete TypeScript check, and invokes upstream's CLI build
script. Upstream's generated lockfile points at Proton's private package mirror;
only that URL prefix is replaced with the public npm registry. Exact locked
versions and SHA512 package integrity hashes are retained. Sibling source-package
imports resolve through the same locked CLI dependency graph via a root
`node_modules` link; they do not install an additional unlocked graph. The transformed lock
hash, bridge source hashes, compiler/SDK/source identity, target and output SHA512
are written beside each binary as `.provenance.json`. Compilation is an explicit
developer/release step; the application never downloads or compiles code.

The x64 target is Bun's baseline target, so AVX2 is not required. The arm64 output
is cross-compiled and must also be smoke-tested on real arm64 Linux before claiming
runtime validation. Both depend on the system glibc and Secret Service/libsecret
stack. Both built ELF targets require GLIBC symbols up to version 2.17.
Package-specific runtime checks remain required.

## Protocol 1

```sh
proton-drive events poll --json --cursor-json '{}' -- /my-files
proton-drive events poll --json --cursor-json '{"scope-id":"acknowledged-id"}' -- /my-files
proton-drive events watch --cursor-json '{}' -- /my-files
```

`poll` produces exactly one complete JSON batch on success:

```json
{"protocol":1,"type":"batch","roots":[{"path":"/my-files","uid":"root-uid","scopeId":"scope-id"}],"events":[{"type":"node_updated","treeEventScopeId":"scope-id","eventId":"event-id","nodeUid":"file-uid","parentNodeUid":"parent-uid","isTrashed":false,"isShared":false}],"cursors":{"scope-id":"event-id"},"resets":[]}
```

The command uses the public `ProtonDriveClient.iterateEvents(scope, cursor,
signal)` interface. It performs no recursive directory scan. Missing cursors
invoke the SDK's latest-event fast-forward and add that scope to `resets`; the
consumer must schedule an initial reconciliation. `tree_refresh` requests
reconciliation and `tree_remove` removes the invalid scope cursor. An exception
on any page rejects the entire batch and exits nonzero without a partial cursor
acknowledgement. Only documented event identifier/status fields are exported.

The v1 bridge accepts My Files folder roots. Events belong to the whole volume,
so consumers of a subfolder must use node/parent ancestry to scope changes; a
root UID does not mean that every event occurred inside that subfolder. Photos,
Shared with me and Computers automatic subscriptions are not implemented.

`watch` is a diagnostic NDJSON stream: each successful line is a batch; an error
line contains only `protocol`, `type: "error"`, generic `code`, and `retryMs`.
It polls every 30 seconds and backs off to 600 seconds on errors. Its in-memory
cursor indicates delivery, not durable consumer acknowledgement. Use `poll`
for the application scheduler.

## Python acknowledgement and cancellation

`neutrondrive.events.EventSource` serializes short `poll` subprocesses using the
existing backend process lock and its own cancellation token. It does not run
simultaneously with a transfer on another NeutronDrive backend instance. The
scheduler receives `RemoteEvent` callbacks and must durably persist pending work
before returning. Only after all callbacks succeed does EventSource atomically
write the cursors to a private 0600 file in a 0700 directory. Cancellation,
callback failure, invalid JSON/schema, or failed persistence preserve the old
cursor for at-least-once replay. Consumers must therefore handle duplicate
notifications idempotently.

The background poll cadence is at least 30 seconds with bounded failure backoff.
Backend busy responses wait without reporting a remote failure. Transient errors
produce a generic status callback without raw SDK errors, credentials, or URLs.
A normal empty event batch never schedules a directory reconciliation. Initial
startup, account/root changes, explicit SDK refresh/removal signals, and actual
file events can schedule reconciliation.

Run contract tests without an account:

```sh
bun test packaging/bridge/eventProtocol.test.ts
python3 -m unittest discover -s tests -p test_events.py
```

The upstream source and licensing are available at the pinned commit:
[MIT license](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/LICENSE.md),
[public event interface](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/client/js/src/interface/events.ts),
[event iteration](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/client/js/src/internal/events/index.ts).

## Computer commands (development)

The current source also adds `computers create --json -- NAME` (Linux device)
and `computers info --json -- UID`. They use `sdk.createDevice` and
`sdk.getDevice`, with duplicate-name rejection before creation. Event polling
accepts explicit `/devices/COMPUTER` roots as well as My files. The frozen SDK,
crypto and authentication behavior are retained. Computer command tests run
alongside protocol and branding tests during `scripts/build-backend`.
An explicit source-built backend can be passed to package builders with
`--backend`; its adjacent provenance must match this checkout and architecture.
