# Desktop features

The latest published package is alpha14; its changes are tracked in
[PR #11](https://github.com/SigmaCodez/NeutronDrive/pull/11). The sections below
identify when features were introduced.
Upgrade the complete application package; older bridges do not contain all
photo-library, album and account-storage commands.

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

The Qt tray provides window activation, direct access to Automatic sync and
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
always at 600 × 600 pixels. Photo-model changes are deferred during a flick,
then applied without resetting selection or scroll position. Rust avoids
copying retained base64 previews when requesting or updating a batch.
The visible range is checked for missing previews even if its first tile is
already cached. These changes reduce local rendering work; cloud response time
and the first download of a preview still depend on the connection.
