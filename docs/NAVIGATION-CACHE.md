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
