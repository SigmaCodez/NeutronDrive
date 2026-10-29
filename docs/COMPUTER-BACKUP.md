# Computer folder synchronization

Open **Computers → Configure computer backup** in the **Back up this computer**
card. The separate **Saved computers** list browses existing cloud devices.
The configuration page registers a Linux device
through the pinned official Proton SDK and adds selected local folders under
**Computers** in Proton Drive. Device registration and all encryption use the
SDK; no private API or reimplemented cryptography is involved.

Choose **Add folder…**, select a local folder and confirm.
The confirmation explains that this is bidirectional folder synchronization,
including guarded trash propagation after a common baseline. It is not a system
image or an immutable backup. This matches the scope of Proton's Windows folder
sync: https://proton.me/support/proton-drive-windows-sync-folder

A unique suffix distinguishes this installation's computer registration. The
private `computer.json` record pins account, device and root identities. Existing
cloud folders with the same name are rejected rather than merged. Overlapping
local sync roots are rejected before remote registration. Jobs are first saved
paused, then pinned to their remote/account identities, then enabled. If setup
fails after a remote folder was created, that folder is retained for review;
setup never deletes cloud data to roll back an error.

Manage running jobs using pause/resume and these distinct removal actions:

- **Stop syncing…** removes the local job and keeps both local and cloud files.
- **Remove backup…** stops the job and moves its verified cloud folder to the
  Proton trash. Local original files are retained.
- **Remove computer…** removes the selected device through Proton's SDK after
  confirming its device/root identity. This may permanently remove its cloud
  backup; the confirmation explicitly warns about this. Local files are retained.

An active transfer must finish before cloud removal can proceed. Failed cloud
removal retains the paused job for review or retry.
Different accounts or replaced/renamed computers require explicit review and
are blocked. No automatic reassociation is performed. Folder deletion remains
blocked when the SDK cannot prove all trashed children; individual-file trash
propagation is supported. Computer events use their own SDK scopes and durable
cursor files, refreshed as job configuration changes.

## Runtime and packages

Alpha7 packages include the native Rust frontend and current SDK bridge with
computer registration commands. The old alpha6 bridge does not contain these
commands. Source builds use `scripts/build-native-qt` and `scripts/build-backend`;
see the packaging guide for the pinned toolchain and baseline instructions.
Package staging verifies the SDK's source/binary provenance and retains the
report alongside the native frontend report. Release SDK assets and their
reports are pinned in `packaging/backend-lock.json`.

The current portable x86-64 Rust frontend is built on Ubuntu 22.04 and has a
verified maximum GLIBC requirement of 2.34. ARM64 uses the Ubuntu 24.04 baseline
and the pinned Qt runtime requires GLIBC 2.39. Both architectures have passed
native CI tests; host-built artifacts still require provenance and ABI checks
before packaging.
The interface provides 25 language catalogs, with English fallback for unavailable
translations. Initial machine-assisted translations still need native-speaker review.
