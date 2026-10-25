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


## Alpha12: content probes and the Nautilus favorite icon

GLib may open files with `O_NOATIME` to guess their content type while listing a
folder. Previously this optional operation could wait behind cloud requests,
download a placeholder, or hash a large cached file. For known GIO clients
(Nautilus, Nemo, Caja, Thunar, PCManFM and `gio`), the FUSE adapter now declines
this read-only probe immediately. GLib retains its filename-based MIME guess.
Normal file opening, copying, hydration and integrity verification are unchanged;
`O_NOATIME` reads from other applications continue to work. This deliberately
bounded compatibility rule does not suppress all thumbnail generation or claim
that every file manager's content-inspection strategy is supported.

The existing Neutron application icon decorates the configured Proton Drive
favorite in Nautilus 50. Nautilus has no public bookmark-icon extension API, so
this optional adapter checks the GTK4 row type and GObject property types at
runtime. Unknown layouts are left unchanged. Only the exact configured local
folder is matched. Other favorites and remote URIs are untouched; saved icons
are restored when the configured folder changes or the extension shuts down.
The adapter uses the host GTK/GIO libraries and local configuration only, with
bounded traversal on the existing two-second main-loop timer. It does not load
Qt, make network requests or access cloud file contents in Nautilus.

`check-cached-browsing.py` includes a real GIO MIME query while a synthetic SDK is
stalled, followed by ordinary `O_NOATIME` reads and `gio copy` content checks.
`check-sidebar.py` uses real GTK4/GObject with synthetic sidebar rows to check
scope, later windows, icon resets, configuration changes and shutdown restoration.
This is an adapter test, not visual verification on every Nautilus version.


### Favorite icons on other desktops

| File manager | Neutron icon integration | Validation |
| --- | --- | --- |
| Nautilus 50 | Guarded GTK4 sidebar adapter | Real GTK4 with synthetic rows; local graphical check required after install |
| Nemo 6.x | Guarded GTK3 `NemoShortcutsModel` adapter | Real GTK3 tree/filter model, resets, root changes and deleted rows |
| Caja | GVfs `metadata::custom-icon` on the sync root | Isolated real GVfs metadata tests; visual session not tested |
| Thunar (custom-folder-icon capable versions) | GVfs `metadata::thunar-custom-icon-name` | Isolated real GVfs metadata tests; visual session not tested |
| Dolphin | Existing optional KIO addon now creates/updates its Proton Drive Places icon | Host KDE 6 addon integration test |
| PCManFM / PCManFM-Qt | Standard icon retained | No custom favorite-icon adapter provided |

The Nemo adapter validates the model name, column count and URI/icon column types
before accessing the GTK model. It never accesses internal C struct layouts and
restores saved icons when applicable. It is optional: changed/unsupported models
are left untouched. Both sidebar adapters have bounded main-loop work.
Caja/Thunar icon metadata is written by a separate helper with a two-second
execution deadline. Existing user-chosen folder icons are preserved. Metadata is
local to the desktop, not uploaded to Proton Drive. The installed SVG path is
resolved from the helper's installation prefix.

PCManFM-Qt reads bookmark icons from a `.directory` file inside the target folder;
this integration deliberately does not create one in the synced cloud folder.
There is no equivalent implementation for PCManFM's libfm bookmark icons here.
No claim of visual testing across all desktop versions is made.

Implementation references: [Nemo sidebar](https://github.com/linuxmint/nemo/blob/6.4.5/src/nemo-places-sidebar.c),
[Caja bookmark](https://github.com/mate-desktop/caja/blob/master/libcaja-private/caja-bookmark.c),
[Thunar folder metadata](https://github.com/xfce-mirror/thunar/blob/master/thunar/thunar-file.c),
[PCManFM-Qt bookmarks](https://github.com/lxqt/libfm-qt/blob/master/src/core/bookmarks.cpp),
[KDE Places model](https://api.kde.org/kfileplacesmodel.html).
