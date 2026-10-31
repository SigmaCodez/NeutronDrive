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

## Validation

Tests use isolated temporary profiles and synthetic SDK responses, never real account files. They cover old job-format compatibility, persisted deadlines, expiration boundaries, replacement deadlines, manual override, fixed-duration IPC commands, missing job identities, safe cancellation, daemon startup after expiry, and automatic resumption without a GUI.

Validation completed: 215 Rust core unit tests, 21 native integration tests and 137 desktop tests passed. The 364-case contract suite had 15 opt-in kernel-FUSE tests skipped; its three UI failures were caused by the test's popup lookup and all three passed after correcting that lookup. The final real-window checks cover all 25 locales plus English fallback, including the timed-pause menu and resume timestamp. A fresh-daemon expired-pause test and an English screenshot check also passed. No release or local installation was performed.
