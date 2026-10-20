# Upstream evidence and provenance

Checked on 2026-10-20 against the public source checkout and built executables.
NeutronDrive 0.2 uses a source-built, additive event bridge over Proton's MIT
Drive SDK/CLI. It does not redistribute an unmodified official CLI under an
unrelated version number, copy the GPL desktop clients, or implement its own
Proton authentication or cryptography.

| Component | Exact identity | Source of truth |
| --- | --- | --- |
| SDK/CLI source | `8a44654aece1609f0f7218be51c205103945abd8` | [Pinned upstream checkout](https://github.com/ProtonDriveApps/sdk/tree/8a44654aece1609f0f7218be51c205103945abd8) |
| NeutronDrive bridge application identity | `external-drive-neutrondrive@0.2.0-alpha` | `packaging/bridge/source-lock.json` |
| Embedded SDK identity | `js@0.22.2+8a44654aece1` | Source snapshot label set by the build; not a separate official SDK release |
| Compiler | Bun `1.4.2` | `packaging/bridge/source-lock.json` and upstream CLI engine requirement |
| Event protocol | `1` | `packaging/bridge/eventProtocol.ts` and `src/neutrondrive/events.py` |
| Binary targets | `bun-linux-x64-baseline`, `bun-linux-arm64` | Per-target `.provenance.json` |
| Packaged artifacts | Per-architecture URL and SHA512 | `packaging/backend-lock.json` |

The release build produces `proton-drive-linux-x64` and
`proton-drive-linux-arm64`, each with an adjacent `.provenance.json`. Provenance
records the source commit, compiler, app/SDK identity, target, output SHA512,
bridge/test source hashes, build-script hash, source-lock hash, version-patch
hash and transformed dependency-lock hash. The dependency release and ordinary
application packages have separate release identities; package recipes verify
the exact locked executable checksum. Nothing builds or downloads code at app
startup. Bit-for-bit equivalence across arbitrary build hosts is not claimed.

## Auditable source changes

The three bridge source files introduce an alternative entry point and two
commands: `events poll` and `events watch`. They import the upstream command
registry and public SDK interfaces, retaining upstream login/logout, encrypted
cache, OS keyring integration, transfer and file-operation implementations.
The bridge does not initialize Sentry or metrics. Its own event commands disable
the legacy CLI event manager so consumer-owned cursors are not consumed by a
second listener. Ordinary CLI commands keep their upstream event/cache behavior.

`version-branding.patch` adds one application-name guard to the upstream version
printer. NeutronDrive's version is not compared with the unrelated official CLI
release train, which would otherwise recommend installing a CLI without the
bridge commands. The two version-identification lines remain available in direct
and interactive mode; a regression test verifies that this path never fetches
the official update manifest.

The upstream Bun lockfile contains tarball URLs under
`https://pkg.protontech.ch/npm/npm-public/`. The public build replaces only that
prefix with `https://registry.npmjs.org/`, retaining every locked package version
and SHA512 integrity value. Sibling source packages resolve dependencies through
the same installed CLI graph, rather than adding independent unlocked installs.
See [the build recipe](../scripts/build-backend) and
[bridge protocol/build documentation](../packaging/bridge/README.md).

## Public event path and contract

The extension resolves a My Files folder through the upstream path resolver,
then calls the public `ProtonDriveClient.iterateEvents(treeEventScopeId,
lastEventId, signal)` method. No raw HTTP API, copied bearer token, browser
session extraction, or reverse engineering is involved.

- [Public client event method](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/client/js/src/protonDriveClient.ts)
- [Public event types](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/client/js/src/interface/events.ts)
- [SDK iteration and cache listeners](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/client/js/src/internal/events/index.ts)
- [Volume pagination, refresh and removal signals](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/client/js/src/internal/events/volumeEventManager.ts)
- [SDK foreground cadence](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/client/js/src/internal/events/eventScheduler.ts)

`poll` emits a complete JSON batch. Missing cursors produce the SDK's latest-event
fast-forward and require an initial reconciliation. Refresh/removal events
request recovery. A failed page produces no partial cursor acknowledgement.
Python persists a cursor only after every consumer callback durably accepts the
work. Event queries run at least 30 seconds apart, with bounded error backoff;
they are not periodic recursive directory listings. `watch` is diagnostic
NDJSON with delivery cursors, not durable application acknowledgements.

Other contract references at the same pinned commit:

- [CLI command registry](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/cli/src/commands/registry.ts)
- [Remote path resolution](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/cli/src/cli/paths.ts)
- [Node/revision JSON fields](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/client/js/src/interface/nodes.ts)
- [Browser authentication](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/cli/src/commands/auth/commandAuthLogin.ts)
- [MIT license](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/LICENSE.md)
- [Third-party operational and branding requirements](https://github.com/ProtonDriveApps/sdk/blob/8a44654aece1609f0f7218be51c205103945abd8/README.md)

## Historical reference and limits

Version 0.1 used the official CLI 0.9.0 release, whose executable identified as
`cli-drive@0.9.0+75612299` and SDK `js@0.22.2+75612299`. The public release tag
`cli/v0.9.0` resolves to `df3a717bb8123791abf115a1dd171cb66813e9f4`; the different
embedded build identifier was not a claim that the public checkout reproduced
Proton's release binary. That official CLI did not expose the event command used
by NeutronDrive 0.2.

The public [Windows](https://github.com/ProtonDriveApps/windows-drive) and
[macOS](https://github.com/ProtonDriveApps/mac-drive) repositories were references
only. Their GPL desktop code and platform APIs are not included. The upstream
SDK/CLI MIT notice is included unchanged in packages. NeutronDrive is a distinct
community application with its own name, icon, interface and application ID;
it is neither an official Proton product nor an exact desktop feature clone.

The x64 backend uses Bun's baseline target without an AVX2 requirement. Both
built ELF targets require GLIBC symbols up to 2.17, but Python/GTK, keyring and
sandbox requirements still determine practical distribution support. The arm64
binary is cross-compiled; local x64 probes do not establish arm64 runtime support.
Automatic subscriptions currently cover My Files only. Hosted-service
requirements and SDK contracts can evolve, so future updates require another
source review and release validation.
