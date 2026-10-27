# Photo viewer and navigation

Open **Photos**, then click an image to open its preview. Use **Select photos**
when you want to select images for album actions; checkboxes stay hidden otherwise.

## Preview controls

| Key | Action |
| --- | --- |
| Left / Right | Previous / next loaded photo |
| Home / End | First / last loaded photo |
| + or = / - | Zoom in / out, from 100% to 400% |
| 0 | Reset zoom to 100% |
| Escape | Close the preview and return keyboard focus to the current photo |

Arrow navigation also works when a preview toolbar button has focus. Already
loaded photos remain browsable during a background request. Moving to another
photo resets zoom and pan so the new image opens in a predictable position.
The preview uses cached images; download the original for full image quality.
Home and End refer to the loaded portion of the library, not an additional cloud
request. Scrolling the timeline continues to load subsequent pages automatically.

## Stable browsing

Appending pages and updating thumbnails preserve the timeline's scroll position
and selection. If an inventory refresh removes or reorders photos, selection is
cleared and an open preview closes to avoid applying an action to a different
photo at the same position. Missing metadata entries are skipped without blocking
the next page. Returned metadata is displayed in inventory order; duplicate or
foreign identities are rejected before changing the library.

Available in NeutronDrive 1.0.2.

## Validation

Checked locally on 2026-10-27 with 93 passing desktop Rust tests, Clippy with
warnings denied, and the real offscreen Qt window in all 25 catalog languages
plus the English fallback. Tests cover a 10,000-entry Rust inventory, a
2,049-entry QML gallery, incremental updates during scrolling, reordered
identities, toolbar-focused keyboard navigation, zoom bounds and focus return.
The tests use synthetic data; they do not certify live Proton response latency
or a complete account synchronization.
