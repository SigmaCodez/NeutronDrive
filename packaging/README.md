# Linux packaging

These recipes bundle the NeutronDrive event bridge built from Proton's public
SDK/CLI. `backend-lock.json` pins each release executable by SHA512.
`bridge/source-lock.json`, the additive patches and `scripts/build-backend`
define the source recipe; per-target provenance accompanies the bridge release.
Dependencies retain the upstream lockfile versions and SHA512 integrity. Only
its unavailable mirror URL prefix is rewritten to the public npm registry.
The SDK/CLI MIT notice is included unchanged. Nothing downloads executable code
at application startup.

Build the backend (Bun 1.4.2 and network access required):

```sh
python3 scripts/build-backend --help
```

The published dependency release is `bridge-v0.2.0-alpha1`; application packages
are in `v0.6.0-alpha6`. Source builds are inspectable; bit-for-bit reproducibility
across arbitrary build hosts is not claimed.

## Requirements and coverage

| Format | Intended distributions | Build command |
| --- | --- | --- |
| DEB | Ubuntu 22.04+, Debian 12+, derivatives such as Mint/Pop!_OS | `python3 scripts/build-deb` |
| RPM | Modern Fedora and openSUSE with Python 3.10+ / bundled Qt 6 | `python3 scripts/build-rpm` |
| Flatpak | Distributions with Flatpak, including Arch, immutable and KDE desktops | `python3 scripts/build-flatpak --install-deps` |
| Snap | Distributions with snapd and working desktop interfaces | `python3 scripts/build-snap` |

The target architectures are **x86_64** and **aarch64**. Native scripts also accept
`--arch amd64` / `--arch arm64`. There are no 32-bit, RISC-V or musl-native packages.
The x64 bridge uses Bun's baseline target without AVX2. Older CPUs still need
runtime testing. The bundled x64 ELF declares
GLIBC symbols through 2.17; the GUI's Python/Qt requirements are higher than the
minimum libc requirement. Building for an architecture does not test that CPU.

Build recipes and CI are not a certification that every distro/desktop works.
Account login, transfer round trips, interruption recovery, sandbox keyring access
and long-running synchronization must be checked on real target installations.
No account credentials are used by CI. CI uploads build artifacts; it does not
publish releases, sign packages or submit to Flathub/Snap Store.

## Qt runtime

`qt_runtime.py` downloads the official PySide6 Essentials/Shiboken 6.10.2 wheels
pinned by SHA256 in `qt-lock.json`. Only the required Qt Quick/QML, Quick Controls Basic/Fusion, dialogs, network, test,
SVG and Wayland/X11 desktop runtime is selected; developer tools and unrelated
bindings are excluded. The included binaries remain unmodified and replaceable.
`packaging/licenses/qt` contains upstream notices from the source archives in
`qt-sources-lock.json`. See [QT-NOTICES.md](QT-NOTICES.md).

Native x86_64 requires GLIBC 2.34; the official ARM64 wheels require GLIBC 2.39
(Ubuntu 24.04+, Debian 13+). Python must be 3.10–3.14. Native packages declare
these bounds. GTK is no longer the UI dependency; Gio remains for D-Bus/portals.
Flatpak and Snap retain their existing GNOME base runtimes for those adapters,
while displaying a Qt interface. No executable download occurs at app startup.

## Rust core

Install the pinned Rust 1.98.0 toolchain for native source builds, then run
`python3 scripts/build-rust-core`. Cargo uses the committed lockfile and vendored
sources with `--locked --offline`. End users need no Rust compiler.

For container/sandbox builds, set `NEUTRONDRIVE_CORE_BUILD` to a directory with
`libneutrondrive_core.so` and `core-build.json` from a matching source build.
Staging verifies architecture, ABI, source hashes, binary hash and the recorded
GLIBC requirement. CI builds the shared x86_64 payload on Ubuntu 22.04; ARM64
DEB builds natively. The core requires at most GLIBC 2.35. Rust dependency and
standard-library notices, Cargo.lock and build provenance accompany every package.

## Build dependencies

Run builds from the repository root. Native builds require Python 3.10+ and
`dpkg-deb` (DEB) or `rpmbuild` (RPM); root is not needed. Each packages a private
backend at `/usr/lib/neutrondrive/bin/proton-drive`, avoiding conflicts with an
independently installed official CLI. Artifacts are written to `dist/`.

RPM dependency names are selected for the build distribution: build Fedora RPMs
on Fedora and openSUSE RPMs on openSUSE. CI runs both variants. Fedora does not
provide pyfuse3, so its build compiles the unmodified, SHA256-pinned 3.4.2 source
privately against the distribution's Python ABI. The RPM includes the complete
source archive, LGPL license and replacement/rebuild instructions. Trio remains
a distribution dependency. openSUSE uses its packaged pyfuse3. Build dependencies
and exact pin are in the CI workflow and `rpm/pyfuse3-lock.json`; no pip command
runs on application startup or installation. An RPM made on a
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
flatpak install --user dist/neutrondrive-0.6.0a6-x86_64.flatpak
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
sudo snap install --dangerous dist/neutrondrive_0.6.0-alpha6_amd64.snap
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

## Background and on-demand integration

Native packages depend on FUSE 3, pyfuse3 and Trio and install a narrowly scoped
D-Bus service activation file plus an optional user systemd unit. Enable
`neutrondrive-background.service` only if you prefer systemd to GUI-managed XDG
autostart; do not enable both. The GUI can generate an XDG autostart entry.

Flatpak uses the Background portal for session persistence/autostart. Mounting
online files requires the native NeutronDrive helper on the host. Its manifest
grants only the application-specific D-Bus name, not general Flatpak host-spawn
access. Host helper and sandbox can require separate Proton browser sessions.

Snap autostart is declared for snap-userd and uses Snap-specific arguments. Its
helper has a separate D-Bus name and cannot accidentally use a native instance.
Host D-Bus activation files and systemd units are excluded from Snap staging.
Strict Snap's FUSE interface does not supply the unprivileged mount helper needed
here, so online mounts are explicitly disabled. Browsing and background folder
sync remain available. Use the native package for on-demand files.

Kernel tests in CI exercise native FUSE; they are not proof of every installed
Snap/Flatpak portal flow.
