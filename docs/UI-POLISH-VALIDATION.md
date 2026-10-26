# Interface refinement and download emblems

Prepared for v1.0.1. The design/emblem build was installed locally on Ubuntu 26.04.1; release packages and Ubuntu 26.10 are verified separately by CI.

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
- Desktop: 90 Rust tests, Clippy, actual Qt/QML window checks, all supported languages and English fallback, setup, compact layout, selection, photo paging/viewer, sharing fixture, clipboard, preferences, updater guards, background lifecycle and window recovery passed.
- Regression suite: 349 passed; the 14 opt-in kernel FUSE and private D-Bus tests were then enabled and all passed (363 total).
- Real Nautilus, Nemo, Caja and Thunar provider libraries passed. InfoProvider tests cover green -> no emblem -> blue -> green and invalidation.
- Isolated GVfs tests cover blue -> green -> blue -> cleared while preserving unrelated user emblems.
- Nautilus and Nemo sidebar integration tests passed with isolated profiles and a temporary X server. Missing host test libraries were extracted into /tmp; no system packages were installed.
- New core tests inspect the status file during a synthetic transfer and verify successful completion, cancellation and recursive folder status. File-manager tests reject stale process identity.
- Package staging verifies both green and blue SVG icons are included.
- Actual English GUI captures were inspected in light, dark and compact modes. See previews/design-polish/capture.json.

## Real-account test on 2026-10-27

Three synthetic 8 MiB files were uploaded to a uniquely named test folder.
The installed native status provider reported `0 -> 2 -> 1` (online-only,
downloading, available) for both an individual file and a selected subfolder.
SHA-256 checks of all three files read through the mounted folder matched the
originals. Including metadata and content verification, these operations took
about 83 and 108 seconds. A preceding user-initiated folder download reported a
cloud-availability warning; the controlled test succeeded. This does not certify
that every item in the user's earlier selection completed.

The Qt regression verifies pin and evict do not reveal a hidden main window;
existing sharing-window and instance-forwarding tests also pass.

## Limits

The real-account test inspected provider status and file contents; it did not capture the live painted icons. No v1.0.1 store publication is claimed until its workflow completes. Dolphin's KIO adapter was updated but not built or exercised in a KDE desktop here. Synthetic fixtures and local kernel tests do not establish complete Windows/macOS feature parity or prove all real-account scenarios.
