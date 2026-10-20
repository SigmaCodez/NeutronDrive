# Validation record

Date: 2026-10-20. This file records evidence, not universal compatibility.

## Automated local checks

- 68 standard-library adapter/sync tests pass on Python 3.14.4 / Ubuntu 26.04 x86_64.
- Adapter tests cover JSON contracts, partial failures, login streaming and URL
  validation, process locks, cancellation, timeouts, output bounds, endpoint/cache
  protections, option injection, symlinks and glob behavior.
- Sync tests cover both directions, nested/empty folders, stale plans, root/account
  changes, concurrent writes, conflicts, local backups, failed-install recovery,
  symlink/FIFO rejection, checksum failures, cancellation and mtime preservation.
- Python sources compile. Desktop entry and AppStream metadata validate.
- Five additional opt-in native GTK tests pass on a Broadway display, covering
  page construction, navigation, filtering, unsafe-entry action controls and busy
  sync state. The normal test run reports 68 passed and five UI tests skipped;
  explicitly enabling the UI suite exercises all 73 checks.
- Official CLI 0.9.0 was executed: reports SDK 0.22.2. SHA512 matches the official
  release manifest. Real argument parsing, including `--`, was checked.

## Packaging and desktop checks

| Format / architecture | Evidence | Remaining validation |
| --- | --- | --- |
| DEB / x86_64 | Built on Ubuntu 24.04; packaged launcher and CLI executed; GTK tests pass | Clean Debian/Mint/Pop!_OS install matrices |
| DEB / aarch64 | Built on native Ubuntu 24.04 ARM64 runner; bundled CLI, diagnose and GTK tests pass | Account transfers on ARM64 desktop hardware |
| RPM / x86_64 | Both Fedora 44 and openSUSE Tumbleweed builds pass; RPM metadata inspected | Installed-package account/desktop session tests |
| Flatpak / x86_64 | Full SDK build; actual final bundle installed in isolated user installation; bundled backend diagnose and mapped GTK window pass | Repeat account transfer/restart tests in installed package, GNOME/KDE portal matrix |
| Snap / amd64 | Actual strict Snapcraft build passes; application bus slots and payload verified | Installed-package login, interface connections and desktop session |
| Other aarch64 formats | Checksum-pinned binary and build recipes | Actual RPM/Flatpak/Snap builds and runtime tests |

All eleven jobs in [GitHub Actions run 39485194520](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39485194520)
passed at source commit `7528cfab4dfa7f29cf50901d9bfc743fc9c0969c`.
The matrix includes Python 3.10–3.13 adapter/sync tests, five GUI tests against
GTK 4.6 on Ubuntu 22.04, and GUI/packaged-backend checks on both Ubuntu 24.04
architectures. Release packages come from this run, not the earlier local builds.

GTK 4 application was launched and visually inspected through GTK's Broadway
test display with explicitly labelled demo data. This proves UI construction and
basic display, not Wayland/X11 account flows on every desktop. The initial
light-theme text contrast issue was found and corrected.

The complete Flatpak SDK build could not run locally because its SDK exceeded
the temporary disk quota. The SDK-free assembly path packages the same Python
source and unmodified verified CLI, uses the manifest's runtime/permissions and
was tested against the installed runtime. The regular SDK build subsequently
completed successfully in GitHub Actions and supplies the release Flatpak.

## Flatpak runtime and credentials

The final CI bundle was installed using a temporary `FLATPAK_USER_DIR`, with the
existing system GNOME 50 runtime. The packaged `--diagnose` completed successfully
and an actual GTK window mapped successfully. A read-only account query correctly
reported no authentication in the new application store. No additional login was
started for that installation. Only test-created app data and the temporary
installation were removed; existing installations and the native session were
preserved. The bundle SHA256 matches the release checksum file.

A preceding assembly sandbox using the same manifest permissions completed an
official browser login followed by a successful root metadata query in a separate
CLI process. This tested portal browser launch and credential retrieval between
processes in that sandbox, not restart/transfer behavior of the installed release.
Its temporary credential store was removed after testing. It contained no copied
native credentials and never used plaintext credential mode.

GNOME 50 libsecret 0.21.8.2 selects an encrypted file backend when the desktop
Secret portal is supported; this store can be separate from native Secret
Service credentials. This explains why the native login is not automatically
present in a fresh Flatpak installation. Source references:
[backend selection](https://github.com/GNOME/libsecret/blob/0.21.8.2/libsecret/secret-backend.c),
[portal-backed file store](https://github.com/GNOME/libsecret/blob/0.21.8.2/libsecret/secret-file-backend.c).
The preliminary `flatpak build` probe needed explicit bus-proxy flags and removal
of build-mode host filesystem access; the final installed-package test used
normal `flatpak run` permissions without those build-mode adjustments.

## Real-account testing

An owner-authorized test account completed official browser authentication and a
root metadata query. One browser displayed “Invalid code” after the single-use
login link was opened twice; the CLI had already completed successfully and the
authenticated session was independently verified.

The synthetic live contract test in `scripts/smoke-live` completed successfully:

- Folder creation, Unicode upload/listing and SHA256-verified download round trip.
- Rename and move, public read-only link creation and revocation, trash and restore.
- Manual sync of new local files to the cloud and new remote files to local storage.
- Changed local content uploaded as a new revision.
- Changed remote content downloaded with the previous local version retained.
- Simultaneous local/remote edits detected as conflicts without overwriting them.

The first live run exposed `sharing remove-url` printing literal `undefined` on
successful removal of the final share. The adapter now accepts this only for the
two optional sharing-result commands, with dedicated regression tests; the full
live run then passed. Both uniquely named synthetic test folders were moved to
the account's trash. Nothing was permanently deleted. No credentials, account
identity, remote node IDs or public share URLs are included in the repository.

## Release limitations

No claim of error-free operation, full Windows/macOS parity, security audit,
all-distribution compatibility or store publication is made. GitHub Actions
results must be read per job; source recipes alone are not built packages.
