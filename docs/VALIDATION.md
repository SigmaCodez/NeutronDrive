# Validation record


## Post-alpha15 Takeout and UI validation (2026-10-25)

Development changes, not included in published alpha15 packages:

- Rust core: 186 unit tests and 18 existing integration tests passed; desktop:
  83 tests passed. Formatting and Clippy with warnings denied passed.
- The Takeout fixture covers media recognition, JSON date matching, duplicate
  content, skipped symlinks, unchanged source bytes, changed-file refusal,
  cancellation and partial failure counts. The UI model requires a separate
  confirmation after scanning and opens Ko-fi only as an external URL.
- All 23 SDK-bridge contract tests and TypeScript checking passed. The new test
  streams a synthetic video to a mocked SDK uploader, checks exact original bytes,
  capture time and location, skips an existing duplicate, and refuses a changed
  account or hash. No photo was uploaded to a live account.
- The actual Qt window suite passed with explicit checks for the Takeout entry,
  Computer setup card and Ko-fi footer. Current Photos, Computer overview and
  backup-configuration captures were visually inspected.
- All 348 retained regressions passed (18 environment-specific skips), followed
  by actual-window checks for all 25 languages and unsupported-locale English fallback.
- New UI strings were added to all 25 catalogs. German was manually reviewed;
  other new strings were machine-translated from public fixed UI text only.

No real Google export or live-account bulk import has been tested. Source media
and sidecars were synthetic; archive extraction and album reconstruction are not
implemented. Updated bridge packaging is required to use the import.

## Post-alpha15 RPM updater validation (2026-10-25)

Development changes, not included in the published alpha15 binaries:

- Rust core: 184 unit tests and 18 existing integration tests passed; desktop:
  82 tests passed. Formatting and Clippy with warnings denied passed.
- RPM selection tests cover Fedora/openSUSE and both recognized architectures,
  invalid digests, unknown formats and missing architecture matches. Download
  tests verify the RPM suffix, SHA-256 revalidation and corrupt-download cleanup.
- Installer tests cover DNF5/DNF/Zypper selection without crossing distribution
  families. The actual metadata reader and cancellation path passed against a
  synthetic RPM built with RPM 6.0.1 extracted into a temporary directory.
  The fixture was never installed. No global signature-check override is used.
- RPM spec expansion produced the expected Fedora and openSUSE package markers;
  the RPM CI jobs now verify the installed marker against their build matrix.
- The isolated real-service replacement test passed again with the new binary.
- Qt updater and actual offscreen window checks passed, followed by all 348
  retained regressions (18 environment-specific tests skipped).

No privileged RPM transaction or full Fedora/openSUSE installation was performed
on this Ubuntu machine. GitHub CI has not run for these unpublished changes.

## Post-alpha15 update restart validation (2026-10-25)

Development changes, not included in the published alpha15 binaries:

- Rust core: 181 unit tests and 18 existing integration tests passed. Desktop:
  79 tests passed, including a real child-process `exec` handoff with literal
  executable and SDK paths containing spaces.
- The new private-session service integration test passed with the newly built
  executable: the sync daemon PID and online-file service D-Bus owner both
  changed after shutdown/restoration. Stopping an absent service did not activate
  it. No credentials or mounted cloud filesystem were used.
- Update transaction tests cover active-sync refusal, failed stops, cancelled or
  failed installation recovery, preserving stopped services, and retrying failed
  restoration without reinstalling the package.
- Formatting and Clippy with warnings denied passed for both Rust crates.
- The Qt updater, full offscreen window/input suite and separate sharing fixture
  passed. All 25 languages and the unsupported-locale English fallback passed
  their actual-window checks. All 348 retained regression tests completed (18 environment-specific
  tests skipped). The first local invocation omitted the test import path for a
  subprocess; the complete suite passed after correcting the test environment.

No real package-manager transaction, live-account update, PC restart or external
package-manager update was performed. The new automatic handoff applies to the
built-in DEB updater. File-manager processes are not forcibly restarted.

## Alpha15 release validation

The [full source and package workflow](https://github.com/SigmaCodez/NeutronDrive/actions/runs/40219851199)
uses application commit `4a18d57fead2c59428535891104e3f684a9ae598`.
A transient TLS download interruption affected the first openSUSE package attempt;
the job passed on retry without relaxing source or package checks. Both native
architectures and every Linux package job completed successfully.
The unmodified amd64 release DEB passed the isolated Qt window suite and the
separate sharing/clipboard fixture. Eighteen actual English GUI captures were
visually inspected; binary, package and image hashes are recorded in
[GUI-PREVIEWS.json](GUI-PREVIEWS.json). Fifteen local package/version/launcher
checks also passed. Exact final successful workflow results and the metadata
verification run are recorded in release BUILD_INFO.json and TEST_RESULTS.json.

## Alpha15 local source validation (2026-10-25)

Validated from source locally, separately from the published alpha14 artifacts:

- Rust core: 179 unit tests and 18 integration tests passed; desktop: 72 tests passed.
- Rust formatting and Clippy with warnings denied passed for core and desktop.
- Pinned Proton SDK bridge: 22 contract tests and TypeScript checking passed.
- Retained regressions: 348 tests executed, 18 environment-specific tests skipped.
- Actual offscreen Qt windows: all 25 language catalogs plus English fallback passed.
  The separate sharing window was opened with the main browser hidden; copying the
  synthetic link preserved its exact bytes. The new Computer backup page and
  Settings sync tab/restart control were found and exercised.
- Photo model: 2,049 synthetic entries, stable scroll position/selection, appending
  during a deterministically driven scrolling state, automatic next-page requests
  and missing-preview prefetch passed. This is not a live-account FPS benchmark.
- Disk-cache tests cover reopening, account/revision separation, expiry, corrupt
  entries, eviction and denied access. Restart tests cover active synchronization
  and a failed stop without starting a replacement daemon.
- Light/dark settings, Computer backup and the separate sharing window were
  captured from the tested executable and visually inspected using synthetic data.

The local checks did not send real invitations, mutate cloud files or install the
application. Existing alpha14 release evidence below remains
historical and does not cover alpha15. Release CI and package evidence are recorded
separately in the published BUILD_INFO.json and TEST_RESULTS.json.

## Alpha14 validation

The [source and package CI](https://github.com/SigmaCodez/NeutronDrive/actions/runs/40213218108)
records alpha14 native architecture, SDK and installed-package checks. Release
BUILD_INFO.json and TEST_RESULTS.json identify the final successful runs and
checksums. Local checks passed 71 desktop Rust tests, 14 package checks and the
isolated Qt interface suite. The 2,048-entry photo regression checks incremental
updates, preserved selection and deferred updates during the scrolling state, including inventory reset without stale tiles.
That state is driven deterministically because offscreen Qt does not reliably
advance a synthetic flick; this is not a hardware FPS or live-cloud benchmark.
Current English light/dark release captures are recorded in
[GUI-PREVIEWS.json](GUI-PREVIEWS.json).


## Alpha13 release validation

[Full source, SDK and Linux package CI](https://github.com/SigmaCodez/NeutronDrive/actions/runs/40206351901)
uses source commit `7902b20611f8738caa7cd6b0de716bedb8055c41`. The native jobs
run 176 core unit tests, 71 desktop tests, 348 retained regressions, 22 SDK tests,
TypeScript checking and actual GUI/service checks on x86_64 and ARM64.


The initial full build caught four new account-menu keys missing from 23
non-English/German catalogs. All 25 catalogs were completed; the dedicated
localization regression tests then passed. The final full CI run, package
provenance and actual release GUI captures are recorded in the release
`BUILD_INFO.json`, `TEST_RESULTS.json` and [GUI-PREVIEWS.json](GUI-PREVIEWS.json).
No live cloud mutation or live-photo network benchmark is claimed.

## Local development checks for alpha13 (2026-10-25)

Local validation passed 71 desktop Rust tests, Clippy for all targets with warnings
denied, the native application build, and nine SDK library-command tests (23
expectations). The Qt smoke test passed in an isolated offscreen session,
including selection, photo viewer, account-menu settings navigation, themes,
keyboard/mouse input and tray behavior.

A synthetic photo model grew from 2,000 to 2,048 entries while preserving the
visible delegate and scroll position. This checks incremental rendering, not
real-account network throughput or a full thumbnail-decoding benchmark. The
dark account menu was also visually inspected from an application-generated
capture with synthetic identity. No live logout or subscription change was
performed. These local checks preceded packaging and do not describe the older alpha12
local1 package below. The release checks are recorded separately.

## Historical alpha11 release and alpha12 local development

The preceding published release was [alpha11](https://github.com/SigmaCodez/NeutronDrive/releases/tag/v0.6.0-alpha11).
Its release assets include BUILD_INFO.json, TEST_RESULTS.json, provenance and
SHA256SUMS. The records below describe their stated versions, not every later build.

Alpha12 development work was tracked in [PR #11](https://github.com/SigmaCodez/NeutronDrive/pull/11).
The initial alpha12 [full CI run](https://github.com/SigmaCodez/NeutronDrive/actions/runs/40193814097)
passed both native architectures and all Linux package jobs. A subsequent live
Nautilus shutdown test found a provider-lifetime bug; its correction adds a
regression that fails before the fix and passes afterward for Nautilus, Nemo,
Caja and Thunar. Follow-up CI is recorded separately in
[run 40199746441](https://github.com/SigmaCodez/NeutronDrive/actions/runs/40199746441).

The corrected local package is `0.6.0~alpha12-1+local1`: the CI-built application
and SDK with locally rebuilt, tested file-manager libraries. Cache and pinned
selections survived installation. The user confirmed the Neutron favorite icon
in Nautilus. Repeated shutdown completed without a new crash report. Final live
checks listed 11 directories in 0.06–0.11 seconds and rendered three folder
windows in 0.14–0.15 seconds. One earlier listing exceeded eight seconds; the
final checks did not reproduce it, and its cause is not conclusively identified.
These observations do not certify every folder or file manager. Those alpha12 development captures have been superseded by the current
release previews; see [their provenance](GUI-PREVIEWS.json).

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

## 0.6.0 alpha10 release validation

Native application and DEB source commit: `bba4545cdf728ddec2da026e781fa3acc2844033`. Package corrections and final validation are recorded
in the [release checks](https://github.com/SigmaCodez/NeutronDrive/pull/9/checks). Feature changes and release metadata are
tracked in [PR #9](https://github.com/SigmaCodez/NeutronDrive/pull/9).

The build validates both native architectures, Rust tests and Clippy, retained
regression contracts, SDK bridge tests and eight Linux installation packages.
Native menu providers are tested against real host libraries with synthetic
file information and intercepted process spawning, including the two Nautilus
ABIs. Isolated actual Qt input tests cover the themed sign-in/folder steps,
photo selection, viewer, albums and navigation. Fresh English previews come
from the unchanged release DEB, with executable hashes in GUI-PREVIEWS.json.

No new live cloud mutation test was performed for this UI/menu release. Earlier
alpha9 live-test evidence remains separately dated above. PCManFM/PCManFM-Qt
full desktop GUI testing remains pending. This does not certify every distro
or establish absence of all bugs. See release BUILD_INFO.json, TEST_RESULTS.json,
SHA256SUMS and native/SDK provenance for exact artifacts and test jobs.

## 0.6.0 alpha11 file-manager/cache/tray validation — 2026-10-25

Local feature validation for alpha11 (isolated synthetic fixtures):

- Rust core: 176 unit tests, 18 runtime/service integration tests; Clippy passes.
- Rust/Qt frontend: 69 unit tests and Clippy pass. Isolated actual-window checks
  cover setup, file/grid/photo interaction, hide-to-tray/reopen and normal exit.
- Real Nautilus 4, Nemo, Caja and Thunar libraries accept the native menu provider.
  Nautilus/Nemo/Caja InfoProvider tests verify adding a completed-file emblem and
  invalidating it when availability changes. The legacy Nautilus 3 artifact
  builds; a fresh live Nautilus 42 runtime test was not performed here.
- A separate X11/StatusNotifier host verifies actual `--background` tray startup,
  hidden initial window, activation, close-to-tray and a silent duplicate start.
- Isolated GVfs tests verify adding/removing the custom emblem while preserving
  a user-set favorite emblem. No test-account content is changed.
- The KDE 6 adapter compiles and loads with a real KIO plugin loader; its test
  observes overlay removal and the Proton Drive bookmark. KDE 5 remains untested.
- A separate GIO test process hides only the configured existing mount and
  restores visibility when its temporary configuration changes. The user's
  running file manager and mount remain untouched.
- The real FUSE synthetic-SDK test injects a five-second list stall: cached root
  listing, stat and missing-name lookup complete in approximately 1 ms. Nested
  metadata prefetch completes without downloading file contents.
- Foreground mounts, desktop services, guided setup, mount cleanup, application
  activation/crash recovery, AppImage cache and native loader checks pass.
- All 14 package-version and native packaging/provenance tests pass. Native host packages stage the
  emblem and KDE addon source; the addon is compiled against the host KDE ABI.

A full PC restart with these new binaries, every file-manager GUI and every
distribution has not been tested. See [the support matrix](FILE-MANAGER-INTEGRATION.md) for
optional dependencies and the remaining PCManFM duplicate-volume limitation.

The 25 GUI languages and unsupported-locale English fallback passed their actual
window/input checks. Tests using the local network-hosted Qt runtime intermittently exceeded
the normal 25-second process deadline; the successful local rerun allowed 60
seconds for process startup with identical UI assertions and local catalog copies.
The repository's normal CI deadline was not relaxed.

Both AMD64 and ARM64 native build/test jobs passed in
[run 40186253319](https://github.com/SigmaCodez/NeutronDrive/actions/runs/40186253319),
including the real isolated tray and GVfs checks. Both DEBs passed installation
checks in that run; these exact DEBs are retained for the AUR hashes and release
previews. The first RPM assembly reported missing file-list entries for the new
emblem and Dolphin sources; the package manifest was corrected. Final package
checks and asset origins are recorded in the release's `BUILD_INFO.json` and
`TEST_RESULTS.json`. No new live cloud mutation test was performed.

All published previews were freshly captured from the alpha11 AMD64 DEB
executable in an isolated English profile and visually inspected.
[GUI-PREVIEWS.json](GUI-PREVIEWS.json) records binary and image hashes.
