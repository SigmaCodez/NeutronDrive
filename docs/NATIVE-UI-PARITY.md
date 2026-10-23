# Native Qt controller regression coverage

The source launcher and package staging run the Rust controller. This checklist
tracks retirement of the former PySide controller tests without silently losing
behavioral coverage. Qt/QML and the Proton SDK are retained. Test orchestration
may use Python, but the tested application runs with no Python on PATH.

| Former UI behavior | Native regression location |
| --- | --- |
| Saved language/theme, system fallback, invalid preferences | `desktop/src/preferences.rs`, `ControllerCheck.qml`, `AppearanceCheck.qml` |
| Update check/download options, cancellation and installer failure | `desktop/src/updates.rs`, `update_installer.rs`, `UpdateCheck.qml` |
| Explicit computer-backup setup confirmation | `files.rs::background_setup_captures_folder_and_remote_at_confirmation`, `BackgroundCheck.qml` |
| All pages and light/dark rendering | `NativeShell.qml` through `--check-window` |
| Every supported OS language and English fallback | `desktop/tests/check-window-localization.py`, `check-cli-localization.py` |
| Search, sorting and selection without resetting file delegates | `files.rs::sorting_search_and_selection_preserve_independent_file_model`, `ControllerCheck.qml` |
| Keyboard selection, select-all, mouse selection, delegate identity and folder double-click | `NativeShell.qml` actual Qt input events |
| Unsafe metadata and trash-only actions | `files.rs::trash_only_allows_restore_and_preserves_selected_targets` |
| Demo never starts jobs, mounts or reads personal trash | `files.rs::demo_never_creates_background_work`, `snap_and_demo_mount_guards_do_not_submit_workers` |
| Local-trash receipt validation and refresh after successful restore | `files.rs::trash_restoration_requires_recoverable_receipt_and_refreshes_only_after_success`, core trash transaction tests |
| Captured dialog targets, stale tokens, cancel and nonlocal picker URLs | `files.rs::dialog_token_and_captured_parent_survive_navigation`, `cancelled_dialog_and_remote_urls_never_create_transfers` |
| Busy owner, stale status and worker completion | `files.rs::stale_result_does_not_release_current_busy_operation`, `stale_background_status_cannot_overwrite_a_new_action`, `ControllerCheck.qml` |
| Mount status/options/recovery and restricted Snap mode | `files.rs::mount_options_and_recovery_status_are_preserved`, `MountCheck.qml` |
| Close confirmation and cancellation, install in progress, Flatpak background ownership | `files.rs::close_waits_for_confirmed_cancellation_and_rejects_stale_answers`, `close_preserves_active_install_and_flatpak_background_service` |
| Share revoke confirmation | `files.rs::sharing_revoke_requires_separate_confirmation` |
| Instance activation and crash recovery | `desktop/src/instance.rs`, `desktop/tests/check-window-instance.sh` |

The former PySide controller and its appearance, update, demo and instance
modules have now been removed. The former Python UI test entrypoints invoke the
native process checks instead. The actual view also verifies live theme changes
without losing selection, scrollable background/online pages, and literal prompt
and clipboard values. `check-window-fixture.sh` uses a local SDK fixture which
asserts the exact Unicode/placeholder filename received through the real native
worker. `scripts/check-native-qt` also runs the core private-bus portal contracts.

The former Python application controllers are retired; this table records the
UI portion of the completed runtime regression coverage.
Non-UI compatibility adapters are now test-only under `tests/core_contract`.
The 347 retained regression contracts passed with no skips in native CI on
both x86-64 and ARM64 (run 40034921192). All source-launcher and package
application logic is Rust. Arch package validation and the strict Snap X11 window check also pass.
The latter uses the normal Snap command-chain and confinement, isolating the
synthetic theme fixture only after Snap has created its process cgroup.

Run `scripts/check-native-qt` with the pinned Qt SDK and Rust toolchain. Package
validation additionally requires a fresh provenance-verified release build;
debug-window tests do not establish portable ABI compatibility.
