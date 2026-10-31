# Community-informed improvements

Reviewed on 31 October 2026. This is a qualitative sample of public Reddit discussions, not a survey of all Linux users.

## Implemented: timed background-sync pauses

Folder-sync and computer-backup jobs can now pause for **1 hour, 4 hours or 24 hours**, or until manually resumed. A saved UTC deadline survives closing the UI and restarting the daemon or PC. Expired pauses resume automatically when the daemon runs again. Manual resume or an indefinite pause cancels the old deadline; changing a timer replaces it atomically. Existing job identities and conflict safeguards remain intact.

These controls apply to the selected background job, not to on-demand reads through the FUSE mount. They do not impose bandwidth limits. The service must be running for transfers to resume. Other active work and cloud retries can delay the actual transfer; an elapsed pause makes the job eligible to run.

The job cards and Activity page show the last completed sync, the next retry when offline, and the automatic-resume time. A job without a successful sync explicitly says so. The six new labels are included in all 25 language catalogs.

### Why these changes

- [Linux users discussing day-to-day reliability and automatic sync](https://www.reddit.com/r/ProtonDrive/comments/1v0nch0/anyone_here_using_proton_drive_as_their_primary/) (July 2026) describe inconsistent third-party sync and slow large operations.
- [Limited-time sync pauses](https://www.reddit.com/r/ProtonDrive/comments/1ulfq23/windows_pause_sync_for_a_limited_time/) (July 2026) addresses forgetting to resume after reducing network or CPU load. The thread also includes requests for Linux support.
- [Linux GUI usability](https://www.reddit.com/r/ProtonDrive/comments/1rkiqny/proton_drive_app_for_linux/) (March 2026) emphasizes a straightforward GUI and responsive file browsing.

### Other recurring requests, not implemented in this change

- [File and folder exclusions](https://www.reddit.com/r/ProtonDrive/comments/1wzc527/add_ability_to_exclude_specific_subfolders_from/) and [development-folder exclusions](https://www.reddit.com/r/ProtonDrive/comments/1gosqmw/node_modules_exclusion/): need explicit semantics for existing cloud files, deletion baselines and nested paths before adding a safe ignore system.
- [Bandwidth limits](https://www.reddit.com/r/ProtonDrive/comments/1543xp2/proton_drive_needs_a_configurable_ratelimit_option/): require transfer-layer enforcement across the SDK, upload, download and mounted-file paths. A pause is not a rate limiter.

## Implemented: actionable sync troubleshooting

- Activity now offers **Retry now** for failed, offline, blocked and conflicted jobs. This rebuilds failed directory watches and schedules a fresh sync with the existing identity and conflict checks. A stale click never resumes a manually paused job or interrupts an active transfer; a missing job identity is rejected. Retrying does not choose which side of a conflict wins.
- All reported errors, conflicts and skipped items can be expanded in a scrollable list, rather than relying on the five-conflict status preview. Long lists use virtual delegates. Unchanged status polls preserve the activity cards, expanded details and scroll position.
- **Copy diagnostics** copies a deliberately allowlisted JSON summary: app version, OS/architecture, service state, counts of jobs by state, errors, conflicts, skipped items, timed pauses, downloads and pending deletions. It excludes job IDs, file names, paths, account information, timestamps and raw errors. Nothing is uploaded or sent automatically. It remains available during transfers.
- All seven new UI messages are included in the 25 language catalogs.

These changes address the reliability and understandable-status requests in the Linux discussions linked above. Exclusion rules and transfer rate limiting remain separate work.

## Validation

Tests use isolated temporary profiles and synthetic SDK responses, never real account files. They cover old job-format compatibility, persisted deadlines, expiration boundaries, replacement deadlines, manual override, fixed-duration IPC commands, missing job identities, safe cancellation, daemon startup after expiry, and automatic resumption without a GUI.

Validation completed: 217 Rust core unit tests, 21 native integration tests, 140 desktop tests and 349 contract/UI tests passed. The contract suite skipped 15 opt-in kernel-FUSE tests; this change does not modify the FUSE integration. Real-window checks cover all 25 locales plus English fallback, activity retry controls, complete issue lists, preserved delegates and scroll position after unchanged status refreshes, and clipboard diagnostics. Privacy tests deliberately inject sensitive strings into fields and verify that none enter the report. Native-daemon tests verify per-job retry and protection of manual pauses. The final English preview was visually checked and had no QML binding warnings. No release or local installation was performed.
