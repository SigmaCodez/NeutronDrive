# Snap Store beta submission

Status: preparation in progress; this document is not proof of Store approval or publication.

## Listing

Name: `neutrondrive` (availability must be checked when registering)

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

This is beta community software, not affiliated with, endorsed by, or officially
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
   Connect `removable-media` only when needed for external media.
7. After signing in with `snapcraft login`, run:
   `python3 scripts/publish-snap-beta PATH_TO_TESTED.snap`.
8. Wait for automated/manual review. Then verify installation on a clean host:
   `sudo snap install neutrondrive --beta`.

Keep `confinement: strict` and `grade: devel`. No classic-confinement exception is
requested. Keyring auto-connection may require a separate Store review. Store
updates are delivered by snapd, not the application's native DEB/RPM updater.
The publication helper validates package metadata and only targets beta; it does
not replace runtime testing or reserve the name.

Official instructions: https://snapcraft.io/docs/releasing-your-app/
