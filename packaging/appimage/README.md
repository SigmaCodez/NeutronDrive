# Portable AppImage (x86_64)

The package wraps a native DEB built from this checkout. Its application runtime
is Rust plus the pinned Qt/QML libraries and Proton SDK; Python is used only by
build tooling and is not bundled. All 25 language catalogs are retained.

Build on Ubuntu 22.04 using `scripts/build-appimage --deb PATH_TO_NATIVE_DEB`.
The builder checks the DEB's native frontend source hashes, ELF compatibility and
binary hash before assembling the AppImage. linuxdeploy, appimagetool and the
type-2 runtime are SHA256-pinned in `tools-lock.json`; changed assets fail closed.
The Rust executable owns cache materialization and verification before starting
Qt or a background service.
The host supplies GLIBC (2.35+), graphical drivers, a browser and a Secret Service.
No musl/Alpine or 32-bit support is claimed. ARM64 users can use the native package.

## Run

```sh
chmod +x NeutronDrive-0.6.0-alpha16-x86_64.AppImage
./NeutronDrive-0.6.0-alpha16-x86_64.AppImage
```

If the system cannot mount AppImages, extract and run without FUSE:

```sh
./NeutronDrive-0.6.0-alpha16-x86_64.AppImage --appimage-extract
./squashfs-root/AppRun
```

On first launch, bundled files are copied to a private, content-addressed directory
under `$XDG_CACHE_HOME/neutrondrive-appimage` (normally `~/.cache/neutrondrive-appimage`).
This uses additional disk space comparable to the uncompressed package. Files are
verified against the embedded manifest before use. A stable runtime path is needed
because the background daemon and desktop autostart can outlive the AppImage mount.
Nothing is downloaded or installed into system directories at startup.

Keep this cache while using background sync/autostart. To remove it, first disable
autostart, stop background synchronization with the app's `--stop-background` flag,
and close the app. Removing the cache does not remove account credentials or sync
state. Relaunching the AppImage recreates it; re-enable autostart afterward.

Online-drive mounts still require the separately installed native D-Bus helper.
AppImage browsing, transfers and automatic folder synchronization do not require
that helper. A running desktop keyring and `/usr/bin/xdg-open` are required for login.
The application is not sandboxed by AppImage.

## Sources and notices

The application, bridge and pinned Qt source archives are provided by the alpha16
release. Ubuntu copyright notices and exact binary/source package versions are
included in `usr/share/doc/ubuntu-runtime` and
`usr/share/doc/neutrondrive/ubuntu-runtime-packages.tsv`. This manifest records the full Ubuntu build environment; not every listed package
is bundled. Distribution runtime sources are available from Ubuntu's public package archive. On Ubuntu 22.04 with
`deb-src` enabled, use `apt-get source SOURCE_PACKAGE=SOURCE_VERSION` with columns
three and four of that manifest. Standard distribution libraries remain shared
libraries and may be replaced in an extracted AppDir; regenerate its manifest
before using a locally modified package. No executable signature check restricts
user replacement. AppImage tool sources: https://github.com/linuxdeploy/linuxdeploy,
https://github.com/AppImage/appimagetool and https://github.com/AppImage/type2-runtime.
