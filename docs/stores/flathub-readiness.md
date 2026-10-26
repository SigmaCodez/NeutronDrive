# Flathub readiness — not a submission

Checked against https://docs.flathub.org/docs/for-app-authors/requirements
on 2026-10-26. This is an internal engineering note, not PR text or a manifest
for submission. No Flathub submission has been made.

## Current blockers

- New applications must first be accepted in the stable repository. A first
  submission directly to flathub-beta is not accepted. NeutronDrive is still beta;
  do not relabel an unvalidated beta as stable merely to pass this requirement.
- Source-available applications and their bundled runtime dependencies must be
  built from source. Our local Flatpak recipe consumes prebuilt Rust/Qt binaries,
  a Bun-compiled SDK bridge and Qt libraries extracted from wheels. It is a local
  distribution recipe, not a compliant Flathub source-build manifest.
- The source build must work offline: pin the application source, Cargo dependency
  sources, Proton SDK/npm dependency sources, and the required toolchain inputs.
  Prefer the Qt provided by a supported KDE SDK/runtime instead of bundling wheels.
  Validate Qt ABI compatibility and compile the SDK bridge using available tooling.
- Flathub prohibits AI-generated/AI-assisted submission manifests and automated
  submission PRs, their descriptions and review interactions. A human maintainer
  must independently author the submission manifest and handle the submission.
  Known AI-generated application code, docs and other material must be disclosed
  honestly, including affected parts and approximate extent. This project has
  substantial AI-assisted implementation, packaging and documentation; its human
  maintainer must review the history and determine the actual extent.
- Flathub also reviews development history, real-world use, localization quality
  and maintenance. Machine-assisted translations need human review.
- Review the application ID casing against current Flathub requirements before
  submission; changes affect data paths, desktop IDs, D-Bus names and migration.

## Sandbox changes prepared in application source

The local Flatpak no longer requests access to the native NeutronDrive mount
service. Rust rejects sandbox mount operations before opening the session bus.
Status/restore return an explicit unavailable status without activating a helper.
The setup assistant uses ordinary downloaded folder synchronization for Flatpak
and Snap, explains disk usage and two-way behavior before confirmation, and
requests background/autostart consent before adding or starting a job.
Retries reuse an existing matching job; they do not register duplicates.
The native package retains its on-demand FUSE setup.

Do not promise online placeholders, native file-manager extensions, green offline
emblems, or bookmark customization for the standalone sandbox edition. Those are
native-package features. No host-package installation or sandbox escape is used.

## Permissions to justify and test

- network: Proton account access and file transfer;
- home filesystem: explicitly configured persistent folder synchronization;
  evaluate persistent document-portal directory grants before submission;
- Wayland/fallback X11, IPC, DRI: Qt rendering;
- Secret Service: encrypted account credentials;
- StatusNotifierWatcher: optional tray integration;
- Background portal: explicit background/autostart permission.

No host command execution, broad session-bus access, native service permission,
FUSE device access or host extension installation is requested.

## Validation still required before a human submission

Build with org.flatpak.Builder, run both manifest/repository linters and AppStream
validation, then test inside a clean sandbox with no native NeutronDrive package.
Test real sign-in, transfer, regular folder synchronization, denied background
permission, retries, restart/autostart, store upgrade and uninstall behavior.
Capture current English screenshots from that sandbox. The listing summary must
say “Unofficial Proton Drive client for Linux” and link to
https://neutrondrive.sigmacodez.dev with an explicit non-affiliation statement.
