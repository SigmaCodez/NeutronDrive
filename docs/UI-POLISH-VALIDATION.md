# Interface refinement and download emblems

Unreleased changes based on v1.0.0. No package was installed or published for this change.

## Behavior

- Neutral light/dark surfaces, a separate rounded content area, larger navigation targets and shared button styling.
- File actions, including version history, share one wrapping toolbar. Compact windows use smaller margins and omit the decorative eyebrow; controls retain their labels and commands.
- Settings pages reserve space for the scrollbar. Existing prompts, selection, keyboard navigation and account controls remain wired to the same Rust controllers.
- Native Keep on device operations publish blue download status before network work, including incomplete descendants of a selected folder. Completed items use the green availability emblem.
- Cancellation and failures clear transient status. Progress is bound to the originating PID and Linux process start time, so a dead or reused process cannot leave a stale blue indicator.
- Nautilus/Nemo/Caja and Dolphin use the local status API. GVfs-based managers use the background metadata helper. No cloud requests or FUSE file reads are added to file-manager UI callbacks.
- Snap/Flatpak restrictions on native extensions remain unchanged. Emblems are static symbols; file managers control their rendering and refresh timing.

## Validation

- Core: 190 unit tests plus integration tests passed; Clippy with warnings denied passed.
- Desktop: 89 Rust tests, Clippy, actual Qt/QML window checks, all supported languages and English fallback, setup, compact layout, selection, photo paging/viewer, sharing fixture, clipboard, preferences, updater guards, background lifecycle and window recovery passed.
- Regression suite: 349 passed; the 14 opt-in kernel FUSE and private D-Bus tests were then enabled and all passed (363 total).
- Real Nautilus, Nemo, Caja and Thunar provider libraries passed. InfoProvider tests cover green -> no emblem -> blue -> green and invalidation.
- Isolated GVfs tests cover blue -> green -> blue -> cleared while preserving unrelated user emblems.
- Nautilus and Nemo sidebar integration tests passed with isolated profiles and a temporary X server. Missing host test libraries were extracted into /tmp; no system packages were installed.
- New core tests inspect the status file during a synthetic transfer and verify successful completion, cancellation and recursive folder status. File-manager tests reject stale process identity.
- Package staging verifies both green and blue SVG icons are included.
- Actual English GUI captures were inspected in light, dark and compact modes. See previews/design-polish/capture.json.

## Limits

No new real-account upload/download, installer or store publication was performed. Dolphin's KIO adapter was updated but not built or exercised in a KDE desktop here. Synthetic fixtures and local kernel tests do not establish complete Windows/macOS feature parity or prove all real-account scenarios.
