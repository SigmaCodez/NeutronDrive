# Desktop features

The latest published package is alpha16. The sections below identify when features
were introduced.
Upgrade the complete application package; older bridges do not contain all
photo-library, album and account-storage commands.

## Alpha16: Takeout, Computer and donations

**Photos → Google Takeout import** accepts an extracted Google Photos export.
Download every archive part from Google Takeout and extract/merge them into one
folder before choosing it. ZIP/TGZ archives are not extracted by NeutronDrive.
The Rust planner scans media files and associated JSON, then asks for confirmation
with counts for media, videos, duplicates, ignored files and metadata warnings.
Only confirmation starts uploads into Proton Photos, not My files.

Matching `.json` and `.supplemental-metadata.json` files supply `photoTakenTime`
and valid GPS coordinates. Exact sidecar names take precedence; a unique title
match handles truncated metadata filenames. Missing or ambiguous metadata uses
the SDK's embedded metadata fallback and is counted as a warning. Original media
bytes and local JSON files are never rewritten. Albums, descriptions, comments
and edited/original relationships are not reconstructed. See Google's
[export metadata explanation](https://support.google.com/photos/answer/3024190?hl=EN).

Identical content within the selected export is skipped. The public Photos SDK
checks existing uploads by name/content before uploading, allowing the same
export to be retried after cancellation or a lost reply. This is not a global
cross-name duplicate finder. The source hash is checked again before each upload;
a changed source or account stops the operation. Upload failures stop the batch
and show its partial counts and error. Refresh Photos to view new items.

Imports run in the existing cancellable worker. Closing to the tray keeps the
app available; explicitly quitting can cancel the import. There is no separate
scheduled import daemon or automatic restart resume. Supported extensions cover
common image/video formats; decoding and preview support depend on the pinned
Proton SDK. Limits: 100,000 unique media files, 500,000 directory entries,
64 directory levels, 1 MiB per JSON and 64 MiB of JSON per directory. Split larger
exports into batches. Symbolic links and special files are ignored.

**Computer** now separates a prominent **Back up this computer** setup card from
**Saved computers**. Generic upload/new-folder controls are hidden at the device
root. The setup page explains that this is two-way folder synchronization, not a
system image or independent recovery archive, and is separate from `~/ProtonDrive`.

An optional **Support on Ko-fi** button sits at the bottom of the sidebar. It opens
<https://ko-fi.com/D1D11J5U3D> in the browser. No Ko-fi JavaScript or remote widget is
loaded inside the application. New fixed UI strings are included in all 25
catalogs; German was reviewed manually and other new translations are machine drafts.

These features require alpha16 or newer, including its updated SDK bridge.

## Alpha16: update restart

The built-in DEB/RPM updater now coordinates the application, synchronization daemon,
online-file service and its emblem helper. It refuses an active sync and uses a
normal unmount that refuses open files; it never forces an unmount or kills a
transfer. Mount configuration, offline availability and paused jobs are retained.
The final idle check is made under the daemon's job-selection lock.

After successful installation, a helper from the newly installed executable
restores services that were running, then the UI releases its single-instance
lock and replaces itself with the new executable. A failed/cancelled installer
restores the previous services and keeps the UI open. If service restoration
fails after installation, **Restart app and services** retries the restart without
installing the package again. New actions are blocked during this handoff.

These changes are included starting with alpha16. They apply to updates
initiated by the built-in DEB/RPM updater once this implementation is installed;
external package-manager upgrades and other package formats retain their existing
update flow. Native Fedora RPMs use DNF5 or DNF; openSUSE RPMs use Zypper. The
package records its build family so the updater selects only the matching release
asset and architecture. A missing asset never falls back to a different family.
Existing RPM installations need one manual upgrade to the version introducing
this support and its package marker. Atomic/OSTree and transactional-update hosts
keep their system-managed update workflow.

Downloads require the GitHub release SHA-256 digest and are verified again before
installation. The RPM name must be `neutrondrive`; administrator authorization is
requested through `pkexec`. Missing tools are detected before stopping services.
Zypper accepts the verified local unsigned RPM while keeping repository signature
verification enabled. DNF retains the host's signature-checking policy; if that
policy requires signed local RPMs, the current unsigned release is rejected.
The command flows follow the [DNF](https://dnf.readthedocs.io/en/stable/command_ref.html),
[DNF5](https://dnf5.readthedocs.io/en/latest/commands/install.8.html) and
[Zypper](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html) manuals.

File-manager extension libraries already loaded inside Nautilus or
another file manager are refreshed when that file manager next restarts.

## Alpha15 changes

- **Photos:** the next page and its previews load automatically near the bottom;
  there is no “Load more photos” button. Appending continues during scrolling.
  JPEG thumbnail payloads are cached under `$XDG_CACHE_HOME/neutrondrive/photo-thumbnails-v1`
  (normally `~/.cache/neutrondrive/photo-thumbnails-v1`). The private cache is
  limited to 512 MiB and entries expire after 30 days. Account identity and the
  current photo revision determine the cache key. Authenticated metadata is still
  fetched before reuse; this is not an offline photo-library mode. Missing,
  corrupt or unwritable cache entries fall back to fetching previews. Signing
  out clears this cache. Originals are not downloaded for timeline browsing.
  The updated SDK bridge is required to expose revision IDs; older bridges work
  without persistent thumbnail caching.
- **Sharing:** file-manager sharing opens its own themed window, without raising
  the main browser. Link copying, link creation/revocation and email invitations
  retain the selected file/folder. Invitations and revocation require confirmation.
  Canceling confirmation returns to sharing; successful changes reload its status.
- **Computer backup:** choose Computers → Computer backup to add and manage
  backup folders. Other folder-pair jobs remain under Settings → Automatic sync.
- **Navigation:** the Automatic sync sidebar entry is removed. Settings contains
  a dedicated Automatic sync tab with configuration and service start/stop/restart.
  Restart refuses a reported active synchronization and only starts a replacement
  after the previous daemon has confirmed stopping. It does not unmount online files.

Older navigation labels in historical sections refer to the version in which
the feature was introduced. In alpha15, synchronization is configured in Settings.

## Files and sync

### Account menu (alpha13)

The lower-left account button displays the signed-in user's display name and,
when available, email address. Its themed menu opens Settings, the Proton
subscription dashboard, Proton Drive online, or the existing guarded sign-out
flow. This requires the updated SDK bridge as well as the desktop executable.
Only display identity and quota fields are exposed to the UI, never user keys.

List/grid switching shares selection and actions. The view choice lasts for the
window session. The automatic-sync page shows current totals for folders,
active transfers, paused jobs and jobs requiring attention. Detailed job rows
retain errors, conflicts and per-job pause/resume/retry controls.

## Tray

The Qt tray provides window activation, direct access to Settings and
Online drive, and explicit sync-service start/stop. Closing an idle window hides
it only when a tray is available. Closing during an active transfer keeps the
existing cancellation confirmation. If the tray host disappears, a hidden window
is shown again. Starting NeutronDrive again also activates its existing window.

Native “Quit window and tray” leaves the independently managed sync daemon
running. Use “Stop sync service” to stop that daemon. Flatpak keeps its existing
background-process rules and labels the menu action “Close window”. GNOME needs
a working StatusNotifier/AppIndicator host for a tray; a plain GNOME session may
not provide one. A tray is never required to access the app. Flatpak grants only
access to `org.kde.StatusNotifierWatcher`; no unrestricted session-bus permission
was added. Package validation is recorded in the release TEST_RESULTS.json. This does not
certify tray support in every desktop/environment combination.

## Photos and albums (alpha9)

Photos inventories the timeline once and displays the total count. Scrolling
loads metadata and JPEG previews in batches of up to 48. The alpha13 scrolling
update preserves grid delegates and scroll position, separates thumbnail updates
from service/status updates, and requests the next page before reaching the end.
Page loading retains up to 96 thumbnail payloads; preview refresh retains a
nearby window of up to 144 entries. Qt also caches decoded images. Older previews
can reload when revisited. Originals are
not downloaded by browsing. Missing previews remain downloadable by photo ID.
Refresh rebuilds the inventory; cloud changes during browsing are not a frozen
snapshot. The current limits are 500,000 timeline entries and 10,000 albums.

Albums can be created, renamed, opened and deleted. Select up to 48 photos to
add/remove them from an album or select one photo as its cover. Album deletion
is explicitly confirmed and requests `saveToTimeline: true`; force deletion is
never used. Partial SDK results are reported as errors, not complete success.

Photos with valid encrypted GPS metadata offer **Map**, opening their coordinates
on OpenStreetMap in the browser. This is not an embedded map or geolocation
inference. No map request occurs until the user chooses that action. Photos
without GPS data do not receive an invented location. Photo uploads, favorite
editing, RAW grouping and import remain outside this change.

## Guided placeholder sync (alpha9)

Alpha10 uses a themed two-step assistant: sign in to Proton
first, then choose the sync folder. Continue is enabled after authentication;
Back preserves the folder choice. Cards, buttons and inputs follow the main
window’s light/dark appearance.

The first-run assistant defaults to `~/ProtonDrive`. Choose an empty real folder,
sign in, then finish. Setup starts the existing native FUSE service and enables
login autostart only after a successful mount. Existing nonempty directories are
rejected rather than moved or hidden. An autostart failure leaves the live mount
intact and setup incomplete, allowing a retry without interrupting open files.

Files initially appear from cloud metadata. Opening a file downloads and verifies
its contents. **Keep on device** can pin an individual file or recursively pin a
folder; subsequent cloud events refresh pinned revisions and include new child
files. An offline remount can read previously verified pinned content. Existing
computer-backup jobs remain available, and overlapping eager-sync/FUSE roots are
rejected. The advanced online-drive mode is available from sync settings and
retains its reboot restore behavior.

A pinned parent folder applies to its descendants. Individual exclusions inside
a pinned folder are not supported: a later refresh can pin a released child again.
For selective offline content, release the parent and pin only the desired items.

The app's context menu offers availability actions. Native packages also contain
a [Dolphin service menu](https://develop.kde.org/docs/apps/dolphin/service-menus/)
for one selected file/folder. Absolute local paths must belong to the currently
active mount. GNOME/Nautilus context extensions and file-manager status overlays
are not included. Flatpak needs the native host helper; strict Snap does not
support FUSE placeholders. Those limitations are not hidden by the assistant.

## Sharing and account storage (alpha9)

The sidebar includes **Shared by me** as well as **Shared with me**. The sharing
dialog retains public links and adds email invitations with Viewer or Editor
access. A confirmation captures the selected node, recipient and role before
sending. No filename or custom message is included in invitation email options.
The sidebar replaces sign-in after successful authenticated browsing with account
storage usage, free bytes and percentage. It uses upstream account quota fields;
unknown quota is shown as unavailable rather than as zero usage.

## Version history

Select one safe file outside Trash, then choose Version history. The dialog lists
server creation timestamps and available sizes. Restore captures the selected
file, historical revision and current revision before opening a confirmation.
The SDK bridge verifies membership and checks the current revision twice before
sending the restore. A changed file is rejected and requires reloading history.

The SDK cannot atomically lock out another edit during restore, and documents
that server application can be delayed. The UI therefore reports a restoration
request and asks the user to refresh, rather than claiming a verified final state.
No permanent deletion of old versions is offered.

## Alpha9 validation

The [successful release build](https://github.com/SigmaCodez/NeutronDrive/actions/runs/40138612701)
tested both native architectures and all eight installation packages. Local
checks passed 63 frontend unit tests, 172 core unit tests, 347 retained regression
tests without skips, 22 SDK bridge tests, TypeScript checking and Rust Clippy.
The actual Qt interface passed all 25 locales plus English fallback, and the
setup assistant passed a real writable FUSE/autostart integration test.

Authorized live checks covered account storage, photo inventory, outgoing shares,
synthetic album creation/renaming/deletion and real-kernel FUSE online/offline
reads, editing/uploads, create/rename/trash and cache eviction. No invitation
email was sent. See [the validation record](VALIDATION.md) and the alpha9 release
BUILD_INFO.json and TEST_RESULTS.json for the exact scope and remaining limits.

## Historical alpha8 validation

- Rust frontend: 58 tests, including captured restore targets, photo destination
  validation, tray close behavior and background status updates on other pages.
- Rust core: 168 unit tests.
- SDK bridge: 16 tests plus full TypeScript checking, including stale/foreign
  revisions, the final current-revision check and bounded photo loading.
- Native Qt GUI tests cover list/grid navigation, Photos JPEG decoding and version
  history in all 25 languages plus unsupported-locale English fallback.
- An isolated X11/D-Bus StatusNotifier host checks actual tray registration,
  nonempty icon, translated menu and activation (`desktop/tests/check-tray.py`).
- Authorized test-account proof on 2026-10-24: actual photo timeline page loaded;
  synthetic file uploaded twice; version history read; stale restore rejected;
  historical content restored and downloaded byte-for-byte; synthetic test folder
  moved to the cloud trash. No existing user file was modified.

See the alpha8 release BUILD_INFO.json and TEST_RESULTS.json for the exact
build commit, GitHub test run and installed-package validation.

## Gallery and desktop refinements (alpha10)

The photo grid now shows clean image tiles. **Select photos** enters selection
mode; only then do checkboxes appear in the top-left corner of images. **Done**
clears the selection. Tile clicks normally open an in-app preview with previous /
next navigation, keyboard arrows, zoom, date information and download / map actions.
The preview uses the existing bounded thumbnail cache, not an automatic original
file download. Unsupported photo/video previews offer an original download.

Thumbnail size is adjustable. Selection actions support clearing or selecting up
to 48 loaded photos and the existing album operations. This is a bounded batch,
not a claim that all photos in a large library are selected. Continuous loading,
total counts and album operations remain. Video playback, favorites filters,
photo import and complete Proton Photos web parity are not added by this UI change.
New strings have English/German text; other locales use English fallback for these
new labels while retaining their existing translations.

The guided assistant now separates Proton sign-in from folder selection, with
Next / Back navigation and shared light/dark theme styling. Native desktop menus
are described in [file-manager integration](FILE-MANAGER-INTEGRATION.md).

### Alpha11 tray and cached browsing

Graphical-session `--background` startup also launches the single-instance tray
window using `--tray`. A duplicate tray launch does not reveal an existing hidden
window. Closing the window hides it while a tray host is available; the explicit
Quit action still exits the window and tray. If no tray host becomes available,
the hidden startup window is revealed after 15 seconds so it remains accessible.
Hiding keeps active transfers running; explicit exit retains cancellation confirmation.
The sync daemon has its own lifetime.

Metadata prefetch, local availability emblems and file-manager Places integration
are described in the [support matrix](FILE-MANAGER-INTEGRATION.md). See the alpha11 release validation for package and installation evidence.

### Unified interface (alpha14)

Shared buttons, selectors, text fields, checkboxes, menus and scrollbars now
carry the same light/dark styling through setup, settings, sharing, version
history and Photos. Native OS file pickers and tray menus follow the desktop.
See [design conventions](DESIGN.md).

Photo thumbnails decode at tile resolution and screen density rather than
always at 600 × 600 pixels. Incremental photo-model changes are deferred during a flick,
then applied without resetting selection or scroll position. Rust avoids
copying retained base64 previews when requesting or updating a batch.
The visible range is checked for missing previews even if its first tile is
already cached. These changes reduce local rendering work; cloud response time
and the first download of a preview still depend on the connection.
