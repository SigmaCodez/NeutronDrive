# Computer folder synchronization (development)

The new **Automatic sync → Computer backup** card registers a Linux device
through the pinned official Proton SDK and adds selected local folders under
**Computers** in Proton Drive. Device registration and all encryption use the
SDK; no private API or reimplemented cryptography is involved.

Select a local folder, choose **Add folder to computer backup**, and confirm.
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

Manage running jobs using the existing pause/resume/remove controls. Removing a
job stops synchronization; it does not delete the computer's cloud files.
Different accounts or replaced/renamed computers require explicit review and
are blocked. No automatic reassociation is performed. Folder deletion remains
blocked when the SDK cannot prove all trashed children; individual-file trash
propagation is supported. Computer events use their own SDK scopes and durable
cursor files, refreshed as job configuration changes.

## Development runtime

This feature requires both the new Rust core and the new SDK bridge from this
source tree. The previously published alpha6 bridge does not have the computer
registration commands. Build with `scripts/build-rust-core` on the supported
Ubuntu 22.04 baseline and `scripts/build-backend` with pinned Bun 1.4.2, then run
with `--cli /absolute/path/to/the/new/proton-drive`. The package builders accept
an explicit `--backend` whose adjacent provenance matches the current source.
Default release pins still refer to alpha6's old bridge and must be updated as
part of a future release; no new release or local installation is implied here.

The current native development build on Ubuntu 26.04 requires GLIBC 2.39 and is
for local testing only. It must not be published as the baseline Linux build.
The new setup labels currently have German/English text; other locales fall back
to English for these labels.
