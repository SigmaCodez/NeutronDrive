# Sharing improvements and remaining product gaps

Source review: 2026-10-28, Proton SDK commit
`8a44654aece1609f0f7218be51c205103945abd8` (the packaged SDK, not an unpinned latest version).

## Implemented in 1.0.5

The separate sharing window supports public-link viewer/editor permissions,
password protection, expiry dates, and whether editors may reshare. Existing
settings are loaded before editing. Dates selected in the UI expire at the end
of that day in UTC; an unchanged existing date retains its exact original time.
Clearing password or date removes that restriction after confirmation. Link
copying, revocation and email invitations continue to work.

Passwords are masked in Qt and sent to the SDK over a bounded stdin pipe,
never as command-line arguments or through temporary files. Saving requires
confirmation and compares the current SDK sharing state with the state captured
when the dialog opened. A changed state requires a refresh. This is a preflight
check, not an atomic server-side transaction; concurrent external edits can
still race after the check. Refresh after any failed or interrupted save because
Proton may already have applied part of a settings change.

## Not solved by this change

- **Multiple accounts:** the current runtime has one active credential and sync
  context. A complete implementation must isolate encrypted credential-store
  keys, photo/metadata caches, job baselines, mounts, service endpoints and update
  handoffs. Merely adding a chooser would risk mixing accounts. No account chooser
  is shipped in this change.
- **Proton Docs/Sheets native export:** the pinned CLI explicitly excludes Docs
  and Sheets from its account export. Downloading their placeholders is not a
  document export. Use the web editor's export/download menu until a supported
  integration is implemented; see [Proton Sheets export instructions](https://proton.me/support/sheets-import-export).
- **Automatic whole-folder deletion:** the existing guard remains. Safe
  propagation needs complete, fresh identity/content proof of the entire subtree,
  including trashed descendants and concurrent additions or edits. A missing
  directory listing alone is not deletion evidence.
- **Simultaneous file edits:** the existing revision/identity checks and
  recoverable local trash remain. The CLI does not expose an atomic conditional
  revision write, so it is not possible to claim conflict-free sync across
  independent clients. Keep an independent backup.

These are explicit remaining limitations, not features hidden behind a setting.
These changes are included in release 1.0.5. Installing a release remains a separate action.
