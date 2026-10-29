# Responsive navigation and session cache

Previously, leaving a slow listing requested cancellation but kept the UI busy until the SDK returned. Opening a section cleared its list, reloaded the photo inventory and recreated the gallery. The existing disk thumbnail cache could not prevent those extra reads or gallery reconstruction.

The desktop now discards ownership of superseded read results immediately. The controller retains at most one running SDK operation and one latest pending operation, without joining a live reader on the Qt thread. Mutations retain their existing cancellation and completion safeguards. Late results cannot restore an old page or clear a newer operation's busy state.

Visited folders have an in-memory LRU cache, limited to 32 folders and 8 MiB of serialized metadata. Entries younger than 30 seconds are displayed without another request. Older entries remain visible during refresh, and cached subfolders can be opened while a read is running. Explicit Refresh still fetches the cloud list. Mutations invalidate folder snapshots before execution, including failed or partially completed mutations; logout clears them. This cache is session-only, not a persistent copy of the entire account tree. First visits still need a cloud response.

The photo page remains alive between sections, preserving its loaded pages, scroll position and decoded previews. Its loading timers stop while hidden. Returning within 30 seconds does not rebuild the inventory. After that, the inventory is checked while existing photos remain visible; an unchanged automatic inventory check preserves the gallery. Explicit Refresh still reloads metadata. Logout destroys retained gallery state. The existing private, account/revision-scoped disk thumbnail cache remains in use.

## Verification

- 108 Rust desktop tests, including stale results, immediate navigation, folder-cache bounds/expiry, mutation/logout invalidation, cached subfolder navigation and automatic versus explicit photo refresh.
- Three core thumbnail-cache tests: persistence across reopening, account/revision separation, corrupt/expired entries and denied access.
- Clippy with warnings denied.
- Real Qt controller test with an SDK fixture that stalls for 60 seconds: three menu changes must return within 250 ms, Settings must be usable immediately, and shutdown must reap all SDK children.
- Real Qt main-window tests, including retained gallery identity across Files/Photos navigation, image decoding, photo selection/viewer, paging and tray behavior, with isolated synthetic data.

These tests do not establish real-account network latency. No live-account cloud writes are performed for this change.

## Follow-up regression fixes

A subsequent review reproduced three additional failures before fixing them:

- Logout left the previous folder, album pagination and version-history state behind. Successful logout now returns navigation to My files and resets those personal view states without submitting a new cloud request.
- A preview refresh replaced only the thumbnail, leaving old names, revisions and locations in the tile. It now replaces the returned metadata. Items missing from a fresh response lose their old thumbnail/location and are disabled for downloads.
- Successful album membership changes refreshed only the album list. Changes to the currently open album now invalidate its photo inventory and reload it after the album list. Deleting that album returns to the timeline. Cancelling a follow-up cannot restore removed photos; changes to another album preserve the current gallery.

Validation after these fixes: 113 desktop Rust tests, Clippy with warnings denied, real Qt main-window and slow-controller checks. The three original reproductions failed before their fixes; two additional tests cover cancelled follow-ups and unrelated album changes. No real cloud files were changed.

## Refresh and dialog races

Two further reproductions failed before correction:

- An unchanged folder refresh rebuilt the model and cleared selection/focus. Identical sorted metadata now only renews the cache; changed metadata still rebuilds the model and invalidates index-based selection. The first empty response still renders the empty state.
- A confirmed dialog could be consumed while an unrelated navigation read was busy, causing its action to be lost. A valid confirmation now cancels that read and submits the action with the originally captured destination. Stale tokens and rejected dialogs cannot cancel the read, and explicit close confirmation retains its shutdown behavior.

Validation: 117 desktop tests, Clippy with warnings denied, full Qt window checks (including file-delegate identity and selection across Refresh), and the slow-backend controller check with child-process cleanup. No live-account writes were used.

## Photo loading retries and empty pages

Thumbnail and viewer-preview failures previously suppressed all further attempts at the same position until another interaction reset the attempt marker. They now retry after a 15-second cooldown. Small scroll movements no longer reset request markers and bypass failure throttling. Returning to the gallery permits an immediate retry after a navigation cancellation; hidden galleries do not request data.

Automatic pagination now keys its attempt to the inventory offset, not just the number of displayed tiles. A successful batch containing only missing/inaccessible photos advances directly to the next batch instead of unnecessarily waiting 15 seconds for an unchanged tile count.

Validation: the complete real Qt window suite passes, including new checks for empty-page continuation, paging cooldown, thumbnail retries after cooldown, throttling across small scroll movements, and viewer-preview retries. The large synthetic gallery, selection/viewer, retained-page, navigation and tray checks also pass. No live cloud data was changed.

## Avoid unchanged photo payloads and stale selection

Preview responses and empty metadata pages previously incremented the photo revision even when the displayed photo data was unchanged. This forced serialization and delivery of the entire loaded gallery, including cached thumbnails. The Rust controller now tracks actual metadata, thumbnail and cache-window changes; unchanged data keeps its revision while pagination offsets still advance. Real changes continue to notify the view.

The gallery also prunes selected photos that become inaccessible. Such tiles are disabled immediately during scrolling, while ordinary metadata updates retain their existing deferred rendering behavior and accessible selections remain intact.

Validation: the two no-op revision regressions fail before correction and pass afterwards; 119 Rust desktop tests and Clippy pass. The complete real Qt window suite passes, including an access-change test during a simulated flick with a 2,049-photo fixture. No real-account changes were performed.

## Local search without unnecessary row rebuilds

Different search strings can produce exactly the same visible rows. Rendering now compares full visible node metadata before reformatting dates/sizes, serializing the file model and resetting index selection. Matching rows keep their delegates and selection; changed rows still invalidate selection. Counts and empty-state labels are updated independently, including changes to entries outside the active filter.

Local filtering and Ctrl+F remain available during cancellable cloud reads using the already loaded nodes. Filtering does not cancel the cloud request, and the latest filter remains applied when its response arrives. Mutation-time editing guards remain unchanged.

Validation: both regressions failed before correction; 122 Rust desktop tests pass, including a 10,000-entry folder and repeated refined searches. Clippy and the real Qt window checks pass, including the enabled search input during a folder refresh. This demonstrates avoided model rebuilds, not a claimed real-account latency measurement.

## Retry notices and malformed list responses

A failed attempt could leave its error banner visible after a later successful retry. Accepted new work now clears an existing error notice while preserving informational transfer summaries; the failed attempt remains in activity history.

File and local-trash list envelopes are validated before recording completion. A malformed response now records a failed operation and preserves existing entries instead of presenting a misleading successful or empty result. A valid empty array still represents an empty listing.

Validation: both regression tests fail before correction; 124 Rust desktop tests, Clippy with warnings denied, the complete Qt main-window suite and the slow-controller/cancellation check pass. No live-account files were modified.

## Metadata invalidation and renamed emblems

Single-node responses check the cache generation before updating listings, aliases and UID paths. A response started before invalidation cannot overwrite a newer cached listing. File-manager providers update tracked paths after a rename and remove tracking when an entry leaves the configured sync root. Regression tests cover rename completion and moving outside the root.
