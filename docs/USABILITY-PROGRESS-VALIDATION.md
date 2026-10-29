# NeutronDrive 1.0.8 usability changes

## Behavior

- File-manager deletion acknowledges a private, durable local journal before
  returning to the caller. The entry disappears from cached directory listings
  without waiting for the network. An idle worker checks account/root, node and
  revision before sending the cloud entry to trash. Offline failures retry;
  changed cloud entries become visible conflicts. Open or locally edited files
  are not silently discarded. Directory removal only proceeds after its known
  children have been removed and a fresh cloud listing confirms it is empty.
- Activity shows the active sync job, file path, completed item count, percentage,
  retry time, skipped items, cache downloads and queued cloud deletions. Operations
  without a measured total show an indeterminate bar; percentages are item-based,
  not invented byte-transfer estimates.
- Computer backup starts with one folder picker and a confirmation. Stop syncing,
  remove cloud folder backup and remove whole computer are separate, confirmed
  operations. Local source folders are never deleted by these controls.
- Shared folders receive a separate chain emblem, alongside offline/sync status.
  This uses cached sharing metadata, so drawing emblems does not query the cloud.
  Host file managers need their existing integration enabled; sandbox package
  limitations remain unchanged.
- Buttons and the account/storage card use the same updated light/dark palette.
  Ko-fi is only in Settings, at the bottom of the About section.

## Validation scope

Tests use synthetic local files, fake SDK responses and isolated configuration.
No real user files or cloud backups are deleted. New messages are translated in
German and English; other catalogs retain English fallback for these additions.
A live multi-distribution installation and live cloud device deletion are not
claimed by these development tests.

## Results (2026-10-29)

- Core: 209 unit tests and 21 integration tests passed; full native runtime build
  passed after clearing stale shared build artifacts.
- Desktop: 128 Rust tests passed; Clippy passed for core, desktop and all
  file-manager features with warnings denied.
- Contract suite: 364 tests completed, 19 environment-specific tests skipped.
- SDK bridge: TypeScript checks and 27 Bun tests (93 assertions) passed.
- Native provider hosts: Nautilus, Nemo, Caja and Thunar passed. Combined
  available/shared emblems were exercised; the KDE overlay source was updated
  but a real Dolphin session was not tested locally.
- Actual Qt window smoke passed for light/dark themes, compact layout,
  navigation, photos, settings, dialogs and tray behavior. The activity fixture
  renders measured progress and queued deletion; Ko-fi is only reachable through
  Settings.
- Real isolated FUSE mount: cached browsing stayed responsive during a five-second
  fake SDK delay. Multi-file and folder local deletion completed in 0.002 seconds
  while cloud trash was stalled, then all cloud trash operations completed.
  This is a synthetic regression measurement, not a production latency guarantee.

## Follow-up layout fixes

Compact windows now use tighter sidebar spacing and a smaller account card so
all eight navigation entries, including Settings, remain visible at 900 × 640.
Section headings wrap and settings action groups flow onto another row.
The footer keeps cancellation beside a single-line status and gives progress its
own full-width row; Activity does not repeat its detailed bars in the footer.
Empty error/conflict fields no longer leave dangling status separators.

The 128 desktop tests and full offscreen Qt smoke passed after these changes.
Additional assertions verify Settings remains in the navigation viewport and
long progress text cannot push cancellation outside the compact window. Light,
dark and compact screenshots were inspected. No local installation was changed.
