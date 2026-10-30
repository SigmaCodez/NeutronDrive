# Workflow and regression validation — 2026-10-30

Application/workflow source: `d31fb4705a659fa1a38639816a06a22883b5baed`.
These checks do not publish a new release or install a local package.

## Local results

- All seven GitHub Actions YAML files pass Actionlint 1.7.12. Its executable
  download was verified against the GitHub release asset SHA-256 digest.
- Regression discovery: 364 tests, 349 passed and 15 opt-in kernel FUSE tests
  skipped locally. The final run uses `PYTHONPATH=tests`, matching native CI.
  An initial invocation without this setting failed the subprocess import in
  the missing-core-library test; the configured rerun passed.
- Website publisher/consumer: all 16 tests passed; six-language release text
  and existing correction files validated.
- Import scan regression: exactly 100 memberships accepted; 101 rejected before
  upload. All five importer unit tests pass. Core Clippy and vendored source
  completeness checks pass.
- Isolated actual-Qt checks pass for background service, guided setup, mount
  controller cleanup, updates, and window activation/crash recovery.
- Local tray check could not run because Xvfb is absent. Kernel FUSE was not
  enabled locally. Native CI explicitly enables both test groups.

## GitHub results

- [Snap publisher checks](https://github.com/SigmaCodez/NeutronDrive/actions/runs/41081144887): passed.
- [Website manifest checks](https://github.com/SigmaCodez/NeutronDrive/actions/runs/41081145069): passed.
- [Snap release dry run](https://github.com/SigmaCodez/NeutronDrive/actions/runs/41081143566): passed.
  Downloaded the published 1.0.8 Snap and verified version, package metadata and
  checksum. Snapcraft installation/upload and publication-success steps skipped.
- [Website release dry run](https://github.com/SigmaCodez/NeutronDrive/actions/runs/41081148014): passed.
  Generated content from the real published 1.0.8 DEB and retained the workflow
  artifact. Attaching/replacing the release asset was skipped.
- [Full native and Linux package matrix](https://github.com/SigmaCodez/NeutronDrive/actions/runs/41081144951): passed (13 jobs successful).
  The optional artifact-reuse job is intentionally skipped because both native
  builds ran from source. Both architectures passed native Qt, service, kernel
  FUSE, tray and regression tests. DEB (amd64/arm64), Ubuntu 26.10 (amd64/arm64),
  Fedora/openSUSE RPM, strict Snap, Flatpak, AppImage and Arch package checks passed.
  This invokes both reusable workflows (`native-build.yml` and
  `portable-packages.yml`) through `ci.yml`.

The two release workflows now expose a boolean `dry_run` input. Manual tests can
validate existing releases without changing Store channels or GitHub release
assets. Actual release events retain their normal publication behavior.
Dry runs do not test Store credentials, Store approval or channel promotion.
