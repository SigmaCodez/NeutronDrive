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

The current SDK bridge and application packages are published together in
`v0.6.0-alpha14`. Source builds are inspectable; bit-for-bit reproducibility
across arbitrary build hosts is not claimed.

## Requirements and coverage

| Format | Intended distributions | Build command |
| --- | --- | --- |
| DEB | Ubuntu 22.04+, Debian 12+, derivatives such as Mint/Pop!_OS | `python3 scripts/build-deb` |
| RPM | Modern Fedora and openSUSE with bundled Qt 6 | `python3 scripts/build-rpm` |
| Flatpak | Distributions with Flatpak, including Arch, immutable and KDE desktops | `python3 scripts/build-flatpak --install-deps` |
| Snap | Distributions with snapd and working desktop interfaces | `python3 scripts/build-snap` |

The target architectures are **x86_64** and **aarch64**. Native scripts also accept
`--arch amd64` / `--arch arm64`. There are no 32-bit, RISC-V or musl-native packages.
The x64 bridge uses Bun's baseline target without AVX2. Older CPUs still need
runtime testing. The bundled x64 ELF declares
GLIBC symbols through 2.17; the native GUI/Qt requirements are higher than the
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
(Ubuntu 24.04+, Debian 13+). The application does not require Python, PySide or PyGObject. Python is only a
build/test tool. Rust owns D-Bus/portal integration; the SDK uses Secret Service.
Flatpak and Snap retain their existing GNOME base runtimes for those adapters,
while displaying a Qt interface. No executable download occurs at app startup.

## Rust application runtime

The native Rust executable includes the core; no separate core shared library or
Python runtime is installed. Use the pinned Rust 1.98.0 toolchain and the native
build instructions below. Cargo uses committed lockfiles and vendored sources.
Set `NEUTRONDRIVE_NATIVE_BUILD` to the verified native frontend build directory.
Staging checks architecture, source hashes, binary hash and the recorded GLIBC
requirement. Rust dependency notices and build provenance accompany each package.

## Build dependencies

Run builds from the repository root. Native builds require Python 3.10+ and
`dpkg-deb` (DEB) or `rpmbuild` (RPM); root is not needed. Each packages a private
backend at `/usr/lib/neutrondrive/bin/proton-drive`, avoiding conflicts with an
independently installed official CLI. Artifacts are written to `dist/`.

RPM dependency names are selected for the build distribution: build Fedora RPMs
on Fedora and openSUSE RPMs on openSUSE. CI runs both variants. The kernel
adapter is built into the Rust core using pinned, vendored `fuser` sources.
Native packages require `fuse3` for the mount helper; pyfuse3 and Trio are no
longer runtime dependencies. An RPM made on a Debian/Ubuntu build host uses the
Fedora dependency names and is only a packaging smoke check, not a substitute
for the distribution's native build.

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
flatpak install --user dist/neutrondrive-0.6.0a8-x86_64.flatpak
flatpak run io.github.SigmaCodez.NeutronDrive
```

The manifest uses GNOME Platform/SDK 50 (GTK/Python/libsecret), packages the CLI
offline from checksum-verified architecture-specific sources and never strips its
appended Bun payload. The Rust browser adapter uses the desktop portal to open the browser. A working portal backend for your desktop is required.

For machines without enough space for the SDK, an equivalent payload can also be
assembled with just Python and Flatpak:

```sh
python3 scripts/build-flatpak-bundle --backend /path/to/proton-drive
```

This uses Flatpak's `build-finish`, `build-export` and `build-bundle` commands.
It reads permissions and the runtime identity from the same manifest, installs
the same prebuilt Rust frontend, and verifies frontend and backend provenance.
No native code is compiled in either packaging route. The GNOME 50 runtime is still required
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
sudo snap install --dangerous dist/neutrondrive_0.6.0-alpha14_amd64.snap
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

Native packages depend on the FUSE 3 mount helper and install a narrowly scoped
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

## AppImage and Arch formats

The [AppImage recipe](appimage/README.md) builds a portable x86_64 package from a
current provenance-verified native DEB on Ubuntu 22.04. It bundles the Rust/Qt
runtime and retains a private cache for background/autostart stability. Pass
`--deb PATH`; it no longer downloads the published Python-based alpha6 package.

The [Arch/AUR recipe](aur/README.md) repackages the pinned amd64/arm64 DEBs into
`neutrondrive-bin`. A separate Arch Linux CI job builds and installs x86_64 with
makepkg/pacman. An AUR account has not been configured; the recipe and generated
metadata are distributed on GitHub until actual AUR publication is possible.


## Native development artifacts

Build on Ubuntu 22.04 for x86_64, or Ubuntu 24.04 for aarch64. The latter matches
the pinned ARM64 Qt runtime's GLIBC 2.39 requirement. Install a C++ compiler,
CMake, Ninja, Perl, pkg-config, zlib development files and OpenGL/EGL development
headers, plus the runtime libraries listed in the main README. Use Rust 1.98.0.
Run on a local filesystem that supports symlinks:

```sh
python3 packaging/qt_runtime.py --dest build/qt --build-tools
python3 scripts/build-native-qt-tools --work-dir /tmp/nd-qt-tools-work --output /tmp/nd-qt-tools
python3 scripts/prepare-native-qt-dev --fetch --output /tmp/nd-qt-sdk --tools /tmp/nd-qt-tools
export QMAKE=/tmp/nd-qt-sdk/usr/lib/qt6/bin/qmake6
python3 scripts/build-native-qt --output build/native-frontend
export NEUTRONDRIVE_NATIVE_BUILD="$PWD/build/native-frontend"
python3 scripts/build-backend --output build/backend/proton-drive
python3 scripts/build-deb --backend build/backend/proton-drive
```

On ARM64, pass `--target bun-linux-arm64` to `build-backend`; the x86_64 SDK uses
`bun-linux-x64-baseline`. The generator build verifies the pinned Qt source
archive and links Qt statically into qmake/moc. Development archives are pinned
separately for both architectures. No development SDK or Python interpreter is
staged into application packages. All builds verify frontend source/binary
provenance; the SDK bridge has its own source/binary report.

Default staging now expects the locally built `build/backend/proton-drive`.
Flatpak-builder also consumes the current verified SDK and its report instead
of downloading the old bridge release. The reusable `native-build.yml` workflow
produces native frontend and SDK artifacts for the package jobs. CI execution,
not YAML parsing, is required to validate those jobs.

For a local Arch package from the same native DEB:

```sh
python3 scripts/prepare-native-arch --deb PATH_TO_NATIVE_DEB --output /tmp/nd-arch-package
# On Arch Linux, as a regular user:
cd /tmp/nd-arch-package
makepkg --syncdeps
```

This recipe has no Python runtime dependencies and verifies the original DEB's
frontend and SDK provenance before creating its local source/checksum entries.
The published `packaging/aur/neutrondrive-bin` recipe pins the alpha14 native
DEBs and uses the same runtime dependency set as the tested Arch package.
No AUR account creation or publication is performed by either workflow.
