# Desktop feature development

These changes are unreleased and are not present in alpha7 installations. Build
both the native frontend and the SDK bridge from this checkout; the alpha7 bridge
does not contain the new photo-page and revision commands.

## Files and sync

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
was added. Snap and every desktop/environment combination have not been retested
as installed packages for these development changes.

## Photos

Photos loads the actual Proton Photos timeline in pages of 48, preserving server
order. Available SDK JPEG thumbnails are held in memory and are not written to a
persistent thumbnail cache. Missing thumbnails show an icon. Download uses the
photo's identity and the official SDK with rename-on-conflict, so existing local
files are not overwritten. Refresh reloads the current page. Concurrent cloud
changes may move photos between pages; this is not a frozen timeline snapshot.

Albums, map browsing, favorite editing, RAW grouping, Google Takeout import and
photo uploads are outside this change. The file-grid view uses file icons, not
photo thumbnails. English documentation previews contain synthetic demo data.

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

## Validation

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

Full new release-package and installed-distribution validation remains a release
step; the previous alpha7 package results do not certify these changes.
