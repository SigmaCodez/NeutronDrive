# Native file-manager integration

Alpha10 adds **Keep on device**, **Free up space** and **Share…** to
Linux file-manager context menus. These changes are included in the native alpha10 packages.

| File manager | Integration | Scope |
| --- | --- | --- |
| GNOME Files / Nautilus 43+ | Rust native extension, API 4 | Entries appear for selections inside the configured sync folder |
| Nautilus 42 / Ubuntu 22.04 | Separate Rust native extension, API 3 | Same actions, older callback ABI |
| Nemo / Cinnamon | Rust native menu provider | Same actions |
| Caja / MATE | Rust native menu provider | Same actions |
| Thunar / Xfce 4.16+ | Rust native menu provider | Same actions |
| Dolphin / KDE | Service menu | One local file/folder; the application checks membership before acting |
| PCManFM / PCManFM-Qt | File-manager action descriptors | One local file/folder; visibility predicate checks the configured folder; desktop GUI verification remains pending |

Select a file or folder **inside the active NeutronDrive sync folder**, such as
`~/ProtonDrive/Photos`. The native providers support up to 128 selected paths for
pinning or freeing space. Sharing is shown for a single object and opens the
existing NeutronDrive sharing dialog. A link is not created and an invitation is
not sent merely by opening that dialog. Email invitations still require confirmation.

Pinning a folder includes its descendants. Existing limitations for exclusions
inside pinned parent folders still apply. These actions do not automatically
upload arbitrary files from elsewhere on the computer.

Native DEB, RPM and Arch packages install host extensions. Restart the file
manager after installation so it loads the new module; an active desktop session
may require signing out and in. The package does not forcibly terminate file
managers or open transfers. Flatpak, Snap and portable AppImage cannot inject
extensions into host file managers: install the native host package for these
menus and FUSE integration. Strict Snap still cannot mount FUSE itself.

## Safety and architecture

The extensions are Rust shared libraries using the file managers' public GObject
interfaces. They do not embed Qt or Python, contact Proton, or hydrate files while
constructing a menu. Menu eligibility reads only a bounded local configuration
file and compares path components. The application revalidates the entire
selection against the live mount and rejects traversal, outside paths and symlinks
before changing availability. Interrupted multi-file operations report how many
items completed instead of claiming all succeeded.

Native provider activation passes literal argument vectors through GLib, without a shell.
Desktop-entry integrations use their file manager’s parameter escaping and the
same application-side path validation.
An already running NeutronDrive window receives a bounded request over its private
activation socket. Requests wait while a worker or dialog is active. No login
credentials or cloud links are placed in command-line arguments.

## Validation

`file-manager/tests/check-providers.py` loads the actual host libraries and checks
GObject provider registration, menu construction, single/multiple selection,
outside paths, nonlocal URIs, traversal and literal activation arguments.
Synthetic file-info objects and intercepted process spawning prevent changes to
real files or accounts. This validates the native ABI, not every desktop theme
or distributor's file-manager build. Ubuntu 22.04's Nautilus 42 library is checked
separately from the API 4 implementation.

Rust tests cover private-socket forwarding, partial messages without blocking the
GUI, invalid requests, cloud-path mapping and symlink rejection. Package provenance
includes both extension ABIs and the visibility helper, with architecture and
glibc baseline checks. The alpha11 availability integrations below extend these menus.

Public interface references:
[Nautilus](https://gnome.pages.gitlab.gnome.org/nautilus/iface.MenuProvider.html),
[Nemo](https://github.com/linuxmint/nemo/blob/master/libnemo-extension/nemo-menu-provider.h),
[Caja](https://github.com/mate-desktop/caja/blob/master/libcaja-extension/caja-menu-provider.h),
[Thunar](https://developer.xfce.org/thunar/thunarx-writing-extensions.html),
[Dolphin](https://develop.kde.org/docs/apps/dolphin/service-menus/),
[PCManFM-Qt](https://github.com/lxqt/pcmanfm-qt/wiki/custom_actions).

## Alpha11: availability and Places integration

These changes are included in alpha11 native packages. The Dolphin overlay addon
is supplied as source and must be built for the host KDE version.

| File manager | Availability check mark | Proton Drive favorite | Hide duplicate mount |
| --- | --- | --- | --- |
| Nautilus 42/43+, Nemo, Caja | Native Rust InfoProvider | GTK bookmark | Native GIO shadowing |
| Thunar | GVfs `metadata::emblems` | GTK bookmark | Native GIO shadowing |
| PCManFM, PCManFM-Qt | GVfs `metadata::emblems` | GTK bookmark | Not automatically supported |
| Dolphin 6 | Optional KIO addon | KIO addon | KIO addon |
| Dolphin 5 | Addon build target; runtime verification pending | Addon | Addon |

For Dolphin, build the host addon as described in
[the addon instructions](../file-manager/kde/README.md). Its source is included in
native packages. It must match the host KDE/Qt ABI, and is not silently loaded
against the application's private Qt runtime. The service-menu actions work
without this optional addon.

Thunar and PCManFM require GVfs metadata support for emblems. The helper preserves
other emblems and only adds/removes `neutrondrive-available`. Each metadata write
runs outside the file manager and is killed after two seconds if it stalls.
Unsupported metadata writes do not affect sync or file content. No global setting
hides other disks, and no system fstab changes are made.

A check mark means the current revision is locally cached. Pinned folders receive
it only when the complete known subtree is listed and cached; open writes do not
count as complete. Native providers read a bounded local status index, never
query the cloud. The background service retries failed GVfs decorations.

The FUSE service precaches directory metadata recursively without downloading
file contents. Cached lookups use a separate metadata snapshot even while an SDK
operation is blocked. Missing-name probes in already listed directories return
immediately; uncached read-only metadata requests have a bounded wait. Content
reads and writes retain normal hydration and acknowledgement semantics.

Validation includes real host-library menu and emblem callbacks, isolated GVfs
add/remove tests preserving user emblems, a KDE 6 plugin-loader test, and a real
FUSE test with a deliberately stalled SDK. Full graphical sessions on every
listed desktop/distribution have not been tested.
