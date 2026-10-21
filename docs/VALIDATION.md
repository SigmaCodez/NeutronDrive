# Validation record — 0.6.0 alpha6

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
