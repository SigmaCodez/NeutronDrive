# Validation record

## Published 0.6.0 alpha9

Package source commit: `69b16e9dd4bacc96113eb14f001812d260563c50`.
All required jobs passed in the [release build](https://github.com/SigmaCodez/NeutronDrive/actions/runs/40138612701).
Feature changes are tracked in [PR #8](https://github.com/SigmaCodez/NeutronDrive/pull/8).
Local validation has passed 172 Rust core unit tests and 347 retained regression
tests without skips, including real kernel FUSE, isolated D-Bus and all 25 UI
languages plus the unsupported-locale English fallback. The SDK bridge passed
22 tests and TypeScript checking. Native frontend unit tests and the actual Qt
window/service checks also passed. The expanded first-run setup integration
test passed with an actual writable FUSE mount, persisted setup completion,
autostart creation and cleanup. It waits for startup requests to settle before
unmounting; immediate unmount attempts can report a busy mount. Package
CI passed for both native architectures and all eight installation packages.
See release `BUILD_INFO.json`, `TEST_RESULTS.json`, provenance reports and
`SHA256SUMS`. English previews were captured from the unmodified release DEB
with isolated synthetic data; [capture metadata](GUI-PREVIEWS.json) records hashes.

Authorized live tests used a separate test account. Read-only quota, Photos and
outgoing-share requests passed. A synthetic empty album was created, renamed,
verified and removed. A synthetic cloud folder passed real FUSE placeholder
stat without hydration, verified download, pinned offline access, edit/upload,
editor replacement, create/rename/trash and cache eviction. The synthetic test
folder was subsequently moved to cloud trash. No invitation email was sent.
These checks do not certify every Linux distribution, every album operation
against a live account, or absence of all defects.

## Published 0.6.0 alpha8

Package source commit: `360e4713f31967cff337e2e9a9d02b5fa6d1e0cc`.
All required jobs passed in the [full release build](https://github.com/SigmaCodez/NeutronDrive/actions/runs/40094128233).
The optional artifact-reuse job was skipped because both architectures were built
and tested from source in this run.

Release packages: DEB amd64/arm64, Fedora/openSUSE RPM x86_64, Flatpak x86_64,
Snap amd64, AppImage x86_64 and Arch x86_64. The downloadable AUR recipe supports
x86_64/aarch64 DEBs; it has not been published to the AUR.

The pipeline checks Rust unit tests and Clippy, retained differential/regression
contracts, all 25 UI languages plus unsupported-locale English fallback, actual
Qt window input, SDK bridge tests/typechecking, and installed package startup.
See release `TEST_RESULTS.json`, `BUILD_INFO.json`, binary provenance reports and
`SHA256SUMS` for the run, artifacts and checksums. Passing these checks does not
certify every Linux distribution or establish absence of all bugs.

The application runtime is Rust with Qt Quick/QML and the official Proton SDK.
No Python/PySide application runtime is shipped. Python remains in build and
regression tools. The [alpha8 preview record](https://github.com/SigmaCodez/NeutronDrive/blob/v0.6.0-alpha8/docs/GUI-PREVIEWS.json)
documents captures from that release executable with isolated synthetic data.

[Desktop feature validation](DESKTOP-FEATURES.md) records the separate authorized
live test of the Photos timeline and synthetic version restoration. Existing
personal files were not modified. [Windows parity](WINDOWS-PARITY.md) lists the
remaining functional limits, including guarded whole-folder deletion.

## Historical alpha6 record

The following evidence applies to alpha6 only and does not describe the current
Rust runtime or certify alpha8 packages.


Previous runtime and live-account evidence: [alpha5](VALIDATION-0.6-alpha5.md).

Release build source: `e7d30322d0f5d66e268216a97441d305f8a95c8b`.
All **15 CI jobs passed**: [final build](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39658476667).
Packages: DEB amd64/arm64, Fedora/openSUSE RPM x86_64, Flatpak x86_64 and Snap amd64.

All **340 local Python tests passed without skips** using the final alpha6 Rust
artifact, Qt, isolated D-Bus and actual kernel FUSE. 339 tests used POSIX temporary
storage; package staging used the project filesystem to avoid temporary storage
quotas. No security checks were relaxed. Rust tests and Clippy passed in CI.

All 25 catalogs contain 579 messages. Locale resolution, English fallback,
placeholder integrity and unchanged filenames/protocol identifiers were checked.
The UI opened all six pages for 25 supported locales plus unsupported Japanese
(English fallback). See [localization details](LOCALIZATION.md).

The final installed Flatpak passed the same 26-locale window test, six pages,
light/dark appearance, synthetic local trash restoration, native backend process
execution, persistent job storage and real inotify. All application files,
including 25 catalogs, the Rust core and backend matched the build source/pins;
all 425 bundled Qt files matched pinned wheels. Its isolated installation and
profile were removed; no personal account was used.

All six packages passed source/provenance verification. DEB, RPM and Snap payloads
were inspected without installing them; Flatpak was installed and exercised in
an isolated profile. CI validates both native architectures. A successful build
is not a claim that every distribution or desktop was tested.

This release changes localization and retains the alpha5 runtime. No new
live-account test or replacement of the user's installed native package was
performed. Prior live-account evidence is explicitly attributed to alpha5.
Python remains for transfer/trash/FUSE orchestration and the Qt controller.
Whole-folder deletion remains a protected conflict. Flatpak online files need
the native helper; strict Snap cannot provide FUSE mounts. ARM64 Qt requires
GLIBC 2.39. Long-duration workloads and independent auditing remain outstanding.

Initial machine-assisted translations include selected manual corrections;
full native-speaker review remains pending. Technical completeness is not
linguistic certification.

The [published alpha6 release](https://github.com/SigmaCodez/NeutronDrive/releases/tag/v0.6.0-alpha6)
is a prerelease with all 16 expected assets. Every published asset size and GitHub
SHA256 digest matches the local release files. The tag resolves to the exact tested
build source above; later commits only update English documentation and evidence.

## Additional AppImage and Arch packages

Application version remains alpha6, with the unchanged released DEB as input.
Packaging source: `8dec8f13ee268a0455b1350d70b911e4ef34ad46`.
Both [portable package CI jobs](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39677466662)
and all [15 regression CI jobs](https://github.com/SigmaCodez/NeutronDrive/actions/runs/39677466663) passed.

The x86_64 AppImage passed real AppImage startup on Ubuntu 26.04.1, persistent
launcher execution after the original mount exited, all 26 locale configurations,
22 Qt UI tests and 44 background tests. All 471 application files, including the
25 catalogs, Qt, Rust core and backend, match the original alpha6 DEB exactly.
Five cache/launcher safeguard tests passed. The private test runtime and profile
were removed. Initial local testing exposed a host-coreutils/bundled-library
conflict; the final launcher uses shell built-ins and includes a regression test.

The Arch x86_64 package was built as an unprivileged user, installed with pacman,
and tested for Python/Qt/Rust and all 26 locale configurations. Its 471 application
files also match the original DEB exactly. Generated `.SRCINFO` matches the
committed recipe. A missing hicolor icon dependency was corrected; namcap reports
no errors. Existing upstream binary-hardening and privately bundled Qt module
warnings remain. ARM64 is declared in the recipe but was not runtime-tested on Arch.

AppImage stores a private content-addressed runtime cache so background and
autostart paths survive the temporary mount. Online-drive mounts require the
native helper. No new real-account test was performed for these packaging changes.
AUR publication is pending an AUR account and SSH access; GitHub provides the
recipe, generated metadata and tested Arch binary without claiming an AUR listing.
The supplemental release files use `PORTABLE_SHA256SUMS`; existing alpha6 files
and the release tag are retained unchanged.
