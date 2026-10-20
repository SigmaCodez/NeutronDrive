# NeutronDrive

An independent, open-source **Linux desktop client for Proton Drive**, with a
native GTK 4 interface and Proton's official CLI as its encryption and account
backend. The interface is currently in German.

**Status: 0.1.0 alpha.** This is an implemented client, not a feature-complete
replacement for Proton's Windows/macOS apps. In particular, there is no automatic
background synchronization or online-only filesystem. Do not treat an alpha
client as your only backup. See the actual [validation record](docs/VALIDATION.md).

NeutronDrive is not affiliated with, endorsed by, or officially supported by
Proton AG. Proton names are used only to identify the compatible service. The
interface and icon are original; no Windows/macOS application code is copied.

![NeutronDrive's native GTK interface with explicitly labelled demo data](docs/demo.png)

## Features

- Browser-based sign-in on `account.proton.me`; no password field in NeutronDrive.
- Session storage in Linux Secret Service (GNOME Keyring/KWallet), or `pass`.
- Browse My files, shared files, computer backups and trash; search the open folder.
- Upload/download files and folders, create folders, rename and move items.
- Restore unambiguous trash entries; move files to trash without permanent deletion.
- Create, copy and revoke public read-only links.
- Manual two-way synchronization with a preview, SHA1 content comparison,
  conflict detection, a persistent baseline and retained local backup versions.
- Cancellation, timeouts, serialized backend calls and in-session activity history.
- DEB, RPM, Flatpak and Snap build recipes with a checksum-pinned bundled backend.

The app delegates encryption and service requests to the unmodified official
Proton Drive CLI **0.9.0**, using Drive SDK **0.22.2**. No custom cryptography or
raw Drive API implementation is included. [Sources and compatibility](docs/UPSTREAM.md).

## Install

Use a package from this repository's [releases](https://github.com/SigmaCodez/NeutronDrive/releases)
or build it using [packaging instructions](packaging/README.md). GitHub Actions also
provides build artifacts when the corresponding job succeeds. Unsigned alpha
packages are not official Proton packages, Flathub listings or Snap Store releases.

| Format | Intended systems | Notes |
| --- | --- | --- |
| DEB | Ubuntu 22.04+, Debian 12+, Mint, Pop!_OS | Python 3.10+, GTK 4.6+, libsecret |
| RPM | Current Fedora, openSUSE | Distribution-specific dependency names |
| Flatpak | Flatpak-enabled distributions, including Arch/Manjaro and immutable desktops | GNOME 50 runtime; desktop portal and host keyring |
| Snap | snapd-enabled distributions | `password-manager-service` must be connected |

Both x86_64 and aarch64 are represented in build recipes. Artifact availability
and actual validation differ by architecture and format; see the validation
record. The current upstream x86_64 binary may require AVX2. There is no claim
that every distribution, CPU, desktop or sandbox has been tested.

After downloading a matching package:

```sh
# Debian/Ubuntu
sudo apt install ./neutrondrive_0.1.0~alpha1-1_amd64.deb

# Fedora (use the Fedora artifact)
sudo dnf install ./neutrondrive-0.1.0~alpha1-1.x86_64.rpm

# Flatpak
flatpak install --user ./neutrondrive-0.1.0a1-x86_64.flatpak
flatpak run io.github.SigmaCodez.NeutronDrive

# Snap, after building a local unsigned artifact
sudo snap install --dangerous ./neutrondrive_0.1.0-alpha1_amd64.snap
sudo snap connect neutrondrive:password-manager-service
```

Launch **NeutronDrive** from your application menu. Unlock the desktop keyring,
select **Bei Proton anmelden** and finish in the browser. The CLI opens the
browser automatically; the app's fallback login button is needed only if it did
not open. A sign-in link is single-use: opening it again after completion can
produce “Invalid code”, even though the app session is already authenticated.
Refresh the file list to check the session before starting another login.

The official CLI and NeutronDrive share the same keyring session. Signing out in
one signs out that CLI session. Do not run a separate CLI process concurrently
with NeutronDrive operations against the same cache.

## Run from source

Install Python 3.10+ and GTK 4/PyGObject through your distribution. For Debian or Ubuntu:

```sh
sudo apt install python3 python3-gi gir1.2-gtk-4.0 libsecret-1-0 gnome-keyring xdg-utils
python3 packaging/fetch-backend.py build/backend/proton-drive
NEUTRONDRIVE_CLI="$PWD/build/backend/proton-drive" python3 bin/neutrondrive
```

The fetcher downloads only the version and SHA512-pinned binary in
[`packaging/backend-lock.json`](packaging/backend-lock.json). Nothing downloads
or installs executable code when the app starts. Use `--cli /absolute/path` to
select an independently installed compatible official CLI.

```sh
python3 bin/neutrondrive --demo      # clearly labelled read-only UI examples
python3 bin/neutrondrive --diagnose --cli "$PWD/build/backend/proton-drive"
python3 -m unittest discover -s tests -v
```

PyGObject is provided by the operating system, not silently installed with pip.
Core adapter and sync tests use only the Python standard library. Building and
running this project does not require a Proton password in an environment file.

## Manual synchronization

1. Open **Synchronisierung**, select an existing local folder and enter an
   existing remote folder such as `/my-files/Documents`.
2. Select **Vorschau erstellen**. Review every action, conflict and skipped file.
3. Select **Geprüften Plan ausführen** to apply the approved plan.

New files and empty folders can travel in either direction. When an established
baseline shows that only one file changed, its update can travel to the other
side. Remote updates are downloaded into private staging, checked and installed
without blindly replacing a concurrently created file. A replaced local file is
retained beside it as `.neutrondrive-backup-<random-id>`; these files are excluded
from sync. They may contain sensitive plaintext and need the same protection as
the original local folder.

Different contents on both sides, ambiguous names and deletions require manual
resolution. **Deletions are not propagated.** Renames are not tracked as moves.
There is no recurring recursive remote scan. The CLI does not expose event
notifications or conditional revision writes. A recheck reduces write races but
cannot make remote updates atomic; uploads of an existing file create a new
remote revision rather than discarding its history.

State is stored under `$XDG_STATE_HOME/neutrondrive` or
`~/.local/state/neutrondrive` with directory permissions `0700`. Sync baselines
contain relative names and hashes, not account credentials. Resetting a baseline
changes conflict decisions; do not delete it to force an overwrite.

## Known limits

- No background service, tray synchronization, offline placeholders/FUSE mount,
  automatic deletion propagation, multi-account UI or automatic updater.
- No integrated Photos timeline/albums, Proton Docs/Sheets export, historical
  revision browser, member invitations or advanced link settings.
- Native Proton documents cannot be downloaded as regular files by this CLI.
  Transfer summaries report skipped items; sync reports unsupported files.
- Duplicate names in trash cannot be resolved by the CLI's name-only restore
  interface and are blocked. Use the Proton web app for those entries.
- Local upload paths containing `*`, `?`, `[` or `{` are rejected because the
  upstream CLI expands them as globs. Sync excludes such names. Symlinks and
  special files are excluded rather than followed.
- Regular upload/download activity shows the operation and final counts, not
  invented percentage progress; the JSON CLI has no progress event stream.
- Proton may change SDK/API compatibility, including the announced cryptographic
  migration. Compatibility must be retested when the pinned backend changes.

See [architecture](docs/ARCHITECTURE.md), [security](SECURITY.md) and
[contributing](CONTRIBUTING.md). The NeutronDrive code is [MIT licensed](LICENSE).
The bundled CLI retains its [upstream MIT notice](packaging/licenses/proton-drive-MIT.txt).
