# Upstream evidence and provenance

Checked on 2026-10-20 against public sources and the downloaded release binary.

| Component | Version / identity | Source |
| --- | --- | --- |
| Official Drive CLI | 0.9.0, embedded build `75612299` | [Downloads](https://proton.me/download/drive/cli/index.html) |
| CLI release source | `cli/v0.9.0` → `df3a717bb8123791abf115a1dd171cb66813e9f4` | [GitHub tag](https://github.com/ProtonDriveApps/sdk/tree/cli/v0.9.0) |
| Bundled JS SDK | 0.22.2, embedded build `75612299` | CLI `version` output |
| SDK main inspected | `8a44654aece1609f0f7218be51c205103945abd8` | [SDK repository](https://github.com/ProtonDriveApps/sdk) |
| Windows reference | Public, GPL-3.0-or-later with separately licensed components | [windows-drive](https://github.com/ProtonDriveApps/windows-drive) |
| macOS reference | Public, GPL-3.0-or-later | [mac-drive](https://github.com/ProtonDriveApps/mac-drive) |

The release binary checksum, not its differing embedded/source commit strings,
defines the artifact bundled here. Exact per-architecture URL/SHA512 values live
in `packaging/backend-lock.json`. NeutronDrive does not claim to reproduce
Proton's binary from source. The upstream MIT license is shipped with packages.
No code was copied from the GPL desktop applications.

Contract references at the release tag:

- [CLI command registry](https://github.com/ProtonDriveApps/sdk/blob/df3a717bb8123791abf115a1dd171cb66813e9f4/cli/src/commands/registry.ts)
- [Remote path resolution](https://github.com/ProtonDriveApps/sdk/blob/df3a717bb8123791abf115a1dd171cb66813e9f4/cli/src/cli/paths.ts)
- [Node and revision JSON fields](https://github.com/ProtonDriveApps/sdk/blob/df3a717bb8123791abf115a1dd171cb66813e9f4/client/js/src/interface/nodes.ts)
- [Transfer summary](https://github.com/ProtonDriveApps/sdk/blob/df3a717bb8123791abf115a1dd171cb66813e9f4/cli/src/commands/fileSystem/transferSummary.ts)
- [Browser authentication](https://github.com/ProtonDriveApps/sdk/blob/df3a717bb8123791abf115a1dd171cb66813e9f4/cli/src/commands/auth/commandAuthLogin.ts)
- [Operational and product requirements](https://github.com/ProtonDriveApps/sdk#usage-guidelines-for-personal-projects)

The current upstream README describes third-party SDK use as evolving, with a
future cryptographic migration. It requires honest identification, official API
endpoints, suitable caching/backoff and distinguishable third-party branding.
The unmodified official CLI identifies itself as `cli-drive`; a future modified
source build must use a truthful third-party identifier such as
`external-drive-neutrondrive` and be validated separately.
