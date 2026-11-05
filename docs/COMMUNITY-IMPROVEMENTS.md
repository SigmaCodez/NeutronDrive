# Community-informed improvements

Latest review: **5 November 2026**. This is a qualitative sample of public
feedback, not a survey or vote-based ranking. Priorities below are maintainer
recommendations based on usefulness and implementation dependencies.

## Request inventory and implementation assessment

The repository issue search returned only [#19, Hello from Syncative](https://github.com/SigmaCodez/NeutronDrive/issues/19),
an exchange between developers rather than a feature request. Its follow-up
suggests comparing related Linux clients; it does not request Android or
multi-account support for NeutronDrive. Repository metadata reports zero open
issues and GitHub Discussions disabled at the time of review. The requests below
come from the **broader Proton Drive community**, not a verified backlog of
NeutronDrive users.

| Request | Evidence | Current NeutronDrive status | Suggested next step |
| --- | --- | --- | --- |
| Exclude files/subfolders, such as node_modules and build output | [Subfolder exclusions](https://www.reddit.com/r/ProtonDrive/comments/1wzc527/add_ability_to_exclude_specific_subfolders_from/), [file-level exclusions](https://www.reddit.com/r/ProtonDrive/comments/1rpszqw/proton_drive_needs_filelevel_sync_exclusions_is/) | Open: no configurable per-job ignore rules found. Existing reserved-name and unsupported-file safeguards are not user-defined exclusions. | First feature priority; implement consistently across local scan, remote scan, reconciliation and saved baselines. |
| Limit upload/download bandwidth | [Rate-limit request](https://www.reddit.com/r/ProtonDrive/comments/1543xp2/proton_drive_needs_a_configurable_ratelimit_option/) | Open: no user-configurable transfer bandwidth limit found. Timed pauses do not provide throttling. | Assess SDK/transfer enforcement before adding controls; include concurrent jobs and on-demand reads. |
| Run synchronization at chosen times | [Scheduling requested alongside bandwidth limits](https://www.reddit.com/r/ProtonDrive/comments/1543xp2/proton_drive_needs_a_configurable_ratelimit_option/) | Open: automatic resume after a pause exists, but is not a recurring calendar schedule. | Define timezone, daylight-saving, missed-run and manual-pause behavior before implementation. |
| Pause temporarily and resume automatically | [Timed pause feedback](https://www.reddit.com/r/ProtonDrive/comments/1ulfq23/windows_pause_sync_for_a_limited_time/) | Already implemented in the project: 1, 4 or 24 hours, or manual resume; persisted per-job deadlines. | Retain existing expiration/restart regression coverage; no duplicate feature needed. |
| Reliable Linux sync with understandable failures | [Linux day-to-day reliability discussion](https://www.reddit.com/r/ProtonDrive/comments/1v0nch0/anyone_here_using_proton_drive_as_their_primary/) | Partly addressed: retry, last successful sync, retry/resume times, expandable issue details and private diagnostic summaries exist. Reliability is an ongoing requirement, not a completed checkbox. | Continue reproducible error/recovery tests and UX fixes. |
| Independent security review and public findings | [Linux client feedback](https://www.reddit.com/r/ProtonDrive/comments/1t047ri/i_built_an_opensource_proton_drive_sync_client/) | External review request concerning another client, relevant as a trust requirement for this category. This review establishes no independent NeutronDrive audit. | Separate externally conducted review; internal tests must not be described as an independent audit. |

### Proposed acceptance criteria for exclusions

These are a proposed design, **not implemented functionality**:

- Store explicit rules per sync job and explain their scope before saving them.
  Start with relative paths/subtrees; define glob syntax separately rather than
  implying full gitignore compatibility.
- Treat an excluded subtree as outside reconciliation on both sides. Adding a
  rule must not delete local or cloud content, including files synced previously.
- Preserve sufficient baseline information so that removing a rule does not
  silently overwrite changes made while the path was excluded. Apply normal
  conflict checks when bringing it back into scope.
- Apply rules consistently to scans, event-triggered work, pending operations and
  restart recovery. Reject absolute paths and traversal outside the sync root.
- Test nested paths, rules changed during a job, restart persistence, existing
  cloud files, changed files on both sides and rule removal with synthetic data.
- Explain that exclusions affect synchronization, not necessarily visibility in
  the cloud browser or on-demand mount. Do not advertise disk cleanup as a side
  effect of excluding a path.

### Outcome of this review

The source-backed request inventory and implementation boundaries were updated.
No new runtime feature, release or local installation was made in this review.
The open requests need coordinated engine work or an external review; none was
represented as implemented merely by adding documentation or a UI control.
The earlier implementation and validation records below are retained as history.


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
