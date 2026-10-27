# Snap Store publication

Stable and beta uploads use the GitHub release workflow. Store review and channel availability must be confirmed independently; this document is not proof of approval.

## Listing

Name: `neutrondrive` (registered by SigmaCodez)

Title: NeutronDrive

Summary: Unofficial Proton Drive client for Linux

Description:

NeutronDrive is an unofficial Proton Drive client for Linux, with a native
Rust and Qt Quick desktop interface. Browse, upload and download files,
share files and folders, browse photos and albums, and import photos and videos
from extracted Google Takeout exports.

Synchronize ordinary local folders in the background. The setup assistant
explains that this store edition downloads files instead of creating online
placeholders. Changes are synchronized in both directions; folder sync is not
a system-image backup. Light and dark themes follow the system or your choice.

This is independent community software, not affiliated with, endorsed by, or officially
supported by Proton AG. A Proton account and desktop keyring are required.
On-demand FUSE placeholders and native file-manager extensions are not available
inside this strict Snap. The native DEB/RPM editions provide those integrations.

Website: https://neutrondrive.sigmacodez.dev
Source and support: https://github.com/SigmaCodez/NeutronDrive
License: MIT (bundled components retain their respective licenses).

Screenshots: use the current English light/dark application captures. Do not use
native file-manager screenshots to advertise functionality of the Snap edition.
Recapture the setup assistant after the store-mode changes before upload.

## Publisher actions

1. Sign in to https://snapcraft.io using your own Ubuntu One account.
2. Register `neutrondrive`, if available. Do not enter credentials in chat.
3. Fill the listing using the text above and current screenshots.
4. Build the changed source and pass the native and strict-Snap CI jobs.
5. On a host with working snapd/AppArmor, install the resulting Snap locally and
   test sign-in, folder sync, pause/resume, restart, autostart and sharing.
6. Connect the keyring explicitly unless Store auto-connection has been approved:
   `sudo snap connect neutrondrive:password-manager-service`.
   Safe folder synchronization also requires:
   `sudo snap connect neutrondrive:mount-observe`.
   This read-only interface exposes mount-table/quota information, including mount
   arguments. It allows NeutronDrive to reject FUSE or nested mounts before sync;
   it does not permit mounting or unmounting filesystems. Without this connection,
   setup stops with “Linux mount information is unavailable” before adding a job.
   Connect `removable-media` only when needed for external media.
7. Publish the verified GitHub release with its Snap and SHA256SUMS assets. The
   `Publish Snap Store release` workflow validates them and targets `latest/stable`
   for normal version tags or `latest/beta` for beta tags. See
   [Store automation](../snap-store-automation.md).
8. Wait for automated/manual review and confirm the target channel before testing
   `sudo snap install neutrondrive --channel=latest/stable` on a clean host.

Keep `confinement: strict`. Stable release packages use `grade: stable`; beta
packages use `grade: devel`. No classic-confinement exception is requested.
Keyring and mount-observe auto-connection require separate Store review; neither
is assumed or requested automatically. Store
updates are delivered by snapd, not the application's native DEB/RPM updater.
The older `publish-snap-beta` helper is limited to beta uploads and must not be
used for stable publication. Neither workflow replaces runtime testing or name
registration.

Official instructions: https://snapcraft.io/docs/releasing-your-app/
