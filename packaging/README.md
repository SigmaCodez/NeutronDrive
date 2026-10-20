# Linux packaging

These recipes package the native Python/GTK application together with Proton's
unmodified standalone CLI **0.9.0**, pinned by SHA-512 in
[`backend-lock.json`](backend-lock.json). Builds refuse an incorrect or corrupt
backend. No package downloads executable code on application startup.

The CLI download and its published checksum come from
[Proton's download page](https://proton.me/download/drive/cli/index.html).
Its [source tag](https://github.com/ProtonDriveApps/sdk/tree/cli/v0.9.0) resolves to
`df3a717bb8123791abf115a1dd171cb66813e9f4`; the upstream executable reports
`cli-drive@0.9.0+75612299` and SDK `js@0.22.2+75612299`. The tag commit and embedded
build identifier differ upstream. The download checksum is the package's binary
identity; these recipes do not claim a reproducible build of Proton's binary.
The SDK/CLI MIT license is included unchanged in each package.

## Requirements and coverage

| Format | Intended distributions | Build command |
| --- | --- | --- |
| DEB | Ubuntu 22.04+, Debian 12+, derivatives such as Mint/Pop!_OS | `python3 scripts/build-deb` |
| RPM | Modern Fedora and openSUSE with Python 3.10+ / GTK 4.6+ | `python3 scripts/build-rpm` |
| Flatpak | Distributions with Flatpak, including Arch, immutable and KDE desktops | `python3 scripts/build-flatpak --install-deps` |
| Snap | Distributions with snapd and working desktop interfaces | `python3 scripts/build-snap` |

The target architectures are **x86_64** and **aarch64**. Native scripts also accept
`--arch amd64` / `--arch arm64`. There are no 32-bit, RISC-V or musl-native packages.
Proton's current x64 binary may need AVX2; the 0.9.0 release manifest has no
baseline variant. Older CPUs are not certified. The downloaded x64 ELF declares
GLIBC symbols through 2.17; the GUI's Python/GTK requirements are higher than the
minimum libc requirement. Building for an architecture does not test that CPU.

Build recipes and CI are not a certification that every distro/desktop works.
Account login, transfer round trips, interruption recovery, sandbox keyring access
and long-running synchronization must be checked on real target installations.
No account credentials are used by CI. CI uploads build artifacts; it does not
publish releases, sign packages or submit to Flathub/Snap Store.

## Build dependencies

Run builds from the repository root. Native builds require Python 3.10+ and
`dpkg-deb` (DEB) or `rpmbuild` (RPM); root is not needed. Each packages a private
backend at `/usr/lib/neutrondrive/bin/proton-drive`, avoiding conflicts with an
independently installed official CLI. Artifacts are written to `dist/`.

RPM dependency names are selected for the build distribution: build Fedora RPMs
on Fedora and openSUSE RPMs on openSUSE. CI runs both variants. An RPM made on a
Debian/Ubuntu build host uses the Fedora dependency names and is only a packaging
smoke check, not a substitute for the distribution's native build.

An already downloaded binary can be used with:

```sh
python3 scripts/build-deb --backend /path/to/proton-drive
python3 scripts/build-rpm --backend /path/to/proton-drive
```

The same SHA-512 check still applies. To prefetch the dependency independently:

```sh
python3 packaging/fetch-backend.py build/backend/x86_64/proton-drive --arch x86_64
```

File timestamps in staged application payloads use `SOURCE_DATE_EPOCH` (default:
2026-10-20 00:00 UTC). Build tool versions, runtime revisions and repositories must
also be pinned externally for fully reproducible package files.

## Flatpak

Install `flatpak`, `flatpak-builder`, and configure the Flathub user remote:

```sh
flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
python3 scripts/build-flatpak --install-deps
flatpak install --user dist/neutrondrive-0.1.0a1-x86_64.flatpak
flatpak run io.github.SigmaCodez.NeutronDrive
```

The manifest uses GNOME Platform/SDK 50 (GTK/Python/libsecret), packages the CLI
offline from checksum-verified architecture-specific sources and never strips its
appended Bun payload. The bundled `xdg-open` adapter uses Gio and the desktop
portal to open the browser. A working portal backend for your desktop is required.

For machines without enough space for the SDK, an equivalent payload can also be
assembled with just Python and Flatpak:

```sh
python3 scripts/build-flatpak-bundle --backend /path/to/proton-drive
```

This uses Flatpak's `build-finish`, `build-export` and `build-bundle` commands.
It reads permissions and the runtime identity from the same manifest, installs
the same Python sources, and verifies the same backend checksum. No native code
is compiled in either packaging route. The GNOME 50 runtime is still required
when installing/running the resulting application.

Home-directory access permits transfers and manually started folder backups. Access to
`org.freedesktop.secrets` permits Proton's CLI to use the host keyring. GNOME 50
libsecret can instead select its encrypted file backend when the desktop Secret
portal is available; the portal supplies the encryption key and the app needs a
separate browser sign-in. The declared Secret Service permission still exposes
the session's Secret Service. The application never selects the CLI's plaintext
secret mode.
For an additional backup directory outside your home, explicitly grant it, e.g.:

```sh
flatpak override --user --filesystem=/mnt/my-backups io.github.SigmaCodez.NeutronDrive
```

## Snap

The recipe uses `core24`, the GNOME extension and strict confinement. Snapcraft
normally builds inside a managed VM/container. Use `--destructive-mode` only on
an expendable Ubuntu 24.04 build environment; CI uses a disposable runner.

```sh
python3 scripts/build-snap
sudo snap install --dangerous dist/neutrondrive_0.1.0-alpha1_amd64.snap
sudo snap connect neutrondrive:password-manager-service
neutrondrive
```

`--dangerous` here permits installing an unsigned local build. The
[`password-manager-service` interface](https://snapcraft.io/docs/reference/interfaces/password-manager-service-interface/)
does not automatically connect. Without that explicit connection, persistent
login cannot work. A running unlocked GNOME Keyring or compatible Secret Service
is also required. For external disks, connect the optional `removable-media`
interface. Snap's `home` interface excludes hidden files; select supported paths
or use a native package for unrestricted backup roots.

## Release smoke checks

Before calling a distribution supported, install its artifact in a clean VM and
verify these user-visible flows:

1. Launch from the desktop menu on Wayland and X11.
2. Complete browser login, restart, and confirm the session is retrieved from the
   OS secret store; log out and confirm the session is removed.
3. Create a folder, upload/download a test file, verify its checksum and test a
   Unicode filename. Verify rename/move/trash/restore if exposed by the UI.
4. Test manually started backups with network loss, an interrupted transfer, and a
   remote conflict; verify the advertised conflict policy preserves data.
5. Confirm sandbox file selection, keyring access, external paths and portal
   browser launch on both GNOME and KDE.

Retain the distro/version, desktop, architecture, package hash and results with
the release. A green packaging job alone does not complete this checklist.
