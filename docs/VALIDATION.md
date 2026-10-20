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
- Official CLI 0.9.0 was executed: reports SDK 0.22.2. SHA512 matches the official
  release manifest. Real argument parsing, including `--`, was checked.

## Packaging and desktop checks

| Format / architecture | Evidence | Remaining validation |
| --- | --- | --- |
| DEB / x86_64 | Built, extracted; packaged launcher finds bundled CLI | Clean Ubuntu/Debian install matrices |
| RPM / x86_64 | Built with real rpmbuild; Fedora dependency variant | Fedora/openSUSE install and desktop session tests |
| Flatpak / x86_64 | Assembled and bundled; `--diagnose` passes in GNOME 50 runtime | Browser/keyring login in sandbox, GNOME/KDE portal matrix |
| Snap / amd64 | Recipe validated and source/backend staged | Actual Snapcraft build, installation and keyring connection |
| aarch64 | Checksum-pinned binary and build recipes | Native runtime tests on aarch64 hardware |

GTK 4 application was launched and visually inspected through GTK's Broadway
test display with explicitly labelled demo data. This proves UI construction and
basic display, not Wayland/X11 account flows on every desktop. The initial
light-theme text contrast issue was found and corrected.

The complete Flatpak SDK build could not run locally because its SDK exceeded
the temporary disk quota. The SDK-free assembly path packages the same Python
source and unmodified verified CLI, uses the manifest's runtime/permissions and
was tested against the installed runtime. CI retains the regular SDK build.

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
