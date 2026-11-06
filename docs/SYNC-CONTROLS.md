# Sync scope, schedules and bandwidth

Unreleased implementation, 5 November 2026. These controls are not yet installed
or published. They require the matching native application and SDK bridge.

## Per-job settings

Open **Sync settings** on a folder-sync or computer-backup card. Saving pauses
that job. If a transfer is still stopping, wait for it to finish and save again.
Review the result and choose **Resume** when ready. Cancelling the settings dialog
makes no changes.

Invalid rules are rejected before pausing. The dialog retains the entered paths
and schedule so they can be corrected without entering everything again.

Enter one relative path per line, for example:

```text
node_modules
Project/build
Documents/private-notes.txt
```

Paths are relative to that job's local/cloud root and include the named subtree.
`node_modules` does not match `Project/node_modules`: list that path separately.
Matching is case-sensitive. Wildcards, absolute paths, dot/parent components and
empty path components are rejected. Up to 256 paths of at most 4096 bytes each
are accepted. An empty list restores the full job scope.

An exclusion does not delete files, free disk space or hide files in the cloud
browser/on-demand mount. It skips local contents and remote traversal for that
path, blocks reconciliation there, and preserves the existing baseline. Removing
an exclusion restores ordinary conflict/deletion checks; it does not grant
permission to overwrite newer changes. Existing unsupported-file and symlink
safeguards still apply.

Excluded directories are also omitted from recursive filesystem watches, avoiding
unnecessary inotify registrations for large build or dependency trees.

## Daily schedule

Enable **Sync daily at (UTC)** and enter a time. The timezone is explicitly UTC,
not local time; daylight-saving changes do not move this clock time. Disabling
the schedule restores continuous event-triggered sync.

- Saving sets the next strictly future occurrence; no immediate catch-up is
  caused by creating a new schedule.
- A saved deadline missed while the service was stopped runs once when the
  service is available and the job is resumed. It does not replay every missed day.
- A completed run advances the persisted deadline to the next future occurrence.
  Failed work keeps the existing retry/backoff behavior while the deadline is due.
- **Sync now** and **Retry now** can run before the next daily deadline. They do
  not override a manual pause. A successful manual run also recalculates the next
  future occurrence at the configured time.
- Closing the UI does not stop an enabled background service. A stopped service
  or powered-off computer cannot run a job; network failures can delay it.

## Shared bandwidth settings

Open **Transfer limits** in Settings. Upload and download are independently
specified in KiB/s (1024 bytes/s); 0 means unlimited. Values up to 1048576 KiB/s
are accepted. Changes apply to the next SDK command, not a transfer already in
progress. Restarting the service is not required.

The bridge paces request and response streams, including encrypted file data,
small-file uploads and SDK metadata. Parallel requests within one command share
a budget for each direction. The native core serializes ordinary SDK commands
across foreground, background and mounted-file transfers. Browser traffic and
application-update downloads are outside this setting.

This is application stream pacing, not kernel traffic shaping: socket/network
buffers can produce short bursts. A deliberately slow operation remains
cancellable and has a maximum command duration of 24 hours. The bridge disables
its short HTTP-request timer when limits are active so pacing does not itself
trigger a short-request timeout.

The core sends an explicit capability flag when a nonzero limit is configured.
Older bridges reject it rather than silently transferring without limits. Use
the matching bridge candidate for development testing; the published 1.1.1
backend manifest remains a record of the already published artifacts.

New controls are provided in German and English. Other locale catalogs currently
use English fallback for these new labels; existing translations are preserved.
