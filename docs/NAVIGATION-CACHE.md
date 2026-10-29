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
