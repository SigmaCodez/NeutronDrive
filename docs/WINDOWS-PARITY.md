# Windows feature comparison — 2026-10-23

Reference scope: the official [Windows desktop guide](https://proton.me/support/proton-drive-windows-app),
[folder synchronization](https://proton.me/support/proton-drive-windows-sync-folder)
and [Shared with me](https://proton.me/support/shared-with-me-windows).
This is a tracked comparison, not a claim of complete parity.

| Area | NeutronDrive status |
| --- | --- |
| Browser login and encrypted transfers | Official SDK backend; existing implementation |
| Browse/upload/download/create/rename/move/trash/restore | Existing Qt interface and SDK adapters |
| Automatic bidirectional folder sync | Existing guarded engine, Rust planning/scheduling/state runtime |
| Computer folders | New development implementation using actual SDK Linux devices; see COMPUTER-BACKUP.md |
| Individual-file deletion propagation | Guarded baseline and trash support |
| Whole-folder deletion propagation | Blocked when the public SDK cannot prove trashed children; not parity |
| Online-only files and keep-offline/evict | Linux FUSE implementation; requires native helper, strict Snap limitations remain |
| File-manager overlays/context menus | No complete Nautilus/Dolphin integration comparable to Explorer |
| Shared with me | Browsing/transfers exist; dedicated automatic shared-folder synchronization is not complete |
| Version history and restore UI | Not implemented in the desktop interface |
| Proton Docs/Sheets desktop integration | Not complete |
| Multiple account switching workflow | Not complete; pinned jobs reject account changes |
| Bandwidth limits/exclusions | No full Windows-equivalent settings workflow |
| Updates | Checks and DEB download/install path; other formats use package manager/release page |
| Language/theme | OS default and manual override; newer feature strings still fall back to English in some locales |

Rust now additionally owns computer path/identity validation and update version/
release-asset selection. Qt controller code, transfer orchestration and parts of
FUSE/trash remain Python; the Proton SDK is TypeScript. The project is not a
complete Rust rewrite. Security-sensitive SDK crypto/authentication is retained.

Closing the remaining gaps requires additional implementation, package-specific
validation and, for guarded folder deletion, a reliable SDK contract for trashed
subtrees. Passing tests cannot establish absence of all bugs.
