# Native Qt controller migration

This crate implements the Rust application behind the existing Qt Quick/QML
interface, including file operations, background services, online drives,
preferences, updates and headless commands. Current package and source launchers
use this executable. Previously installed and published versions are unchanged.
The Python application modules have been removed; native-core compatibility
adapters are test-only. The final audit, CI and fresh package/architecture checks
are still in progress.

CXX-Qt generates Qt's QObject/meta-object integration from Rust declarations;
application behavior remains Rust and the interface remains QML. Qt API glue
provides native window, locale, palette and settings integration. Browser and
background portals are owned by the Rust core.

The native dependency graph retains every package identity/checksum from
`core/Cargo.lock`. `desktop/Cargo.lock` adds the pinned CXX-Qt 0.10.0 and CXX
1.0.205 dependencies, vendored alongside the existing Rust dependencies. The
headless core does not acquire a Qt build dependency.

## Host development check

A matching Qt 6.10.2 development SDK and C++17 compiler are required. The initial
isolated x86_64 SDK uses checksum-locked Ubuntu development archives from
`packaging/qt-native-dev-lock.json` and the already pinned Qt runtime. It is a
**host development SDK**, not the portable distribution SDK. No system package
installation is performed.

Download the exact package/version pairs from that manifest into
`build/qt-native-sdk/packages`, then prepare a fresh directory on a filesystem
that supports symlinks:

```sh
scripts/prepare-native-qt-dev --output /tmp/neutrondrive-qt-sdk
export QMAKE=/tmp/neutrondrive-qt-sdk/usr/lib/qt6/bin/qmake6
scripts/check-native-qt
```

The check script runs the locked offline build, Rust unit tests, Clippy,
formatting, and an offscreen QML check on a private bus with no service activation.
The native test process has `PATH=/nonexistent`. It does not need Python,
PySide6, Shiboken, a Proton account, or an installed NeutronDrive service.
The Qt libraries currently reside in the existing Qt runtime staging directory;
using those standalone libraries does not load its Python bindings.

`CARGO_TARGET_DIR` may select a local build cache. On a slow network share,
`NEUTRONDRIVE_QT_VENDOR_CACHE` may point to a local copy of the same checksum-
verified vendored packages. `NEUTRONDRIVE_QT_RUNTIME` may override the Qt runtime
prefix (the directory containing `lib`, `plugins`, and `qml`). The script uses
Rust's bundled LLD if a system LLD is not on PATH.

The existing application regression evidence still applies to the unchanged
production launcher. This crate has its own development checks; it does not yet
prove feature parity, ARM64 support, portable GLIBC requirements or installer
coverage for the final native UI. Remaining work includes the main controller,
preferences/localization, update controller, full QML wiring, native CLI launch
paths, portable Qt SDK/builds and removal of the Python runtime from packages.

## Native file controller and preferences

`NativePreferences` reads the same `NeutronDrive/interface` INI settings through
Qt. Language changes persist for the next launch; appearance changes notify
immediately. Failed writes restore the prior cached value. Rust loads the
existing 25 catalogs and applies the core's locale selection/fallback rules.
`platform.h` contains only thin Qt constructor/API bindings missing from
CXX-Qt-lib; all application decisions remain in Rust.

`NativeFiles` owns the file-browser state machine and its cancellable worker.
The native controller covers folder navigation, Unicode search, selection,
file actions, captured dialog targets, sharing, activity and backend failures.
It keeps file-list notifications independent of selection/status changes so
pressed or focused QML delegates are not replaced between clicks. The core
still validates SDK operations and owns encryption via the pinned SDK.

The checker now also runs `ControllerCheck.qml` with isolated settings. It
checks property notifications, dialog delivery, asynchronous listing, demo
mutation rejection, a missing SDK, and cancellation/destruction while a real
synthetic SDK process runs. It verifies both child PIDs are gone. No test writes
the user's settings or accesses an account. Eleven Rust tests cover state,
locale, preferences and full Unicode folding regressions.

These are native component checks, **not full application parity**. They are
not wired into the production main window yet. Main-window/service-page
orchestration, login/logout, update installation, Qt translation installation,
single-instance/window lifecycle and native CLI/packaging integration remain.

## Native account and service orchestration

The file controller now streams validated SDK login events onto the Qt thread,
keeps the sign-in URL out of general state snapshots, rejects late events from
old operations and clears login controls on completion/cancellation. Successful
login refreshes the listing. Logout requires confirmation, stops background sync,
unmounts native drives when necessary, and only then calls SDK logout. Cleanup
failure prevents logout; Flatpak retains its existing mount behavior. Browser
opening uses a direct Qt API binding; cancellation accepts the existing QML
`action("cancel")` contract.

The executable also accepts `--daemon`, `--background` and `--desktop-service`
with `--cli` and `--state-dir` before Qt initialization. The desktop-client fallback
launches this same native executable, never Python. SIGINT/SIGTERM request owned
runtime cleanup. The integration check starts the desktop service and daemon
with no display and an invalid Qt platform, then checks successful SIGTERM exit
and removal of daemon sockets/PID records. `--background` retains best-effort
mount restoration before daemon startup; that combined startup path is not yet
separately covered by this check.

The expanded suite has 16 Rust tests, two QML checks and the two headless process
checks. Login uses a synthetic trusted-domain URL and is cancelled, without
opening a browser or connecting to an account. Full live authentication is not
claimed. Remaining UI, updates, window lifecycle, other CLI modes and packaging
must still be completed before the production launcher can be replaced.

## Remaining CLI modes migrated

The native entry point now routes sync status/stop, mount/unmount, pin/evict,
service status, foreground mounts, SDK diagnosis, version and help before any
Qt application is constructed. Option parsing preserves remote roots, writable
and offline flags and paths containing spaces. Mode conflicts/missing operands
fail with exit 2. Status/diagnosis output remains JSON; unsuccessful diagnosis
returns 1. Diagnosis cancellation also reaps its SDK child.

Foreground mounting uses the core mount manager directly, always releases its
registry entry, and reports startup/recovery errors. Its runtime result now
retains the cancellation flag, allowing requested signal shutdown to return 0
without suppressing unrelated mount failures. Set `NEUTRONDRIVE_FUSE_TEST=1` when
running the checker to exercise real online/offline mounts using a synthetic
SDK and temporary cache; the mount is read and checked absent after SIGTERM.

The current checks cover 19 desktop Rust tests, 161 core unit tests, both QML
checks, combined background restoration/start/CLI stop, diagnosis cancellation,
and real online/offline foreground mounts. Both crates pass Clippy and formatting.
Production GUI integration, localized CLI text/resource discovery, package builds
and removal of Python are still outstanding. The previously built portable core
artifact predates these core changes and must be rebuilt before final validation.

## Native background controller

The Rust file/main-controller model now also owns the background page fields,
folder selection, activation confirmations, job actions and status presentation.
Captured local/remote roots survive later edits while a confirmation is open.
Computer-folder setup delegates to the existing native computer runtime. Flatpak
portal permission is checked before job/cloud changes. Removing a running job
pauses it before removing its configuration and reloading the daemon.

Foreground actions and polling have independently owned Rust workers. A status
reply is ignored after a new foreground action or a page change, and both
workers are cancelled/joined when the controller is destroyed. Demo mode cannot
start jobs or services. Native daemon startup uses the current executable.

`BackgroundCheck.qml` now exercises a real local job lifecycle through Qt:
choose/confirm, add, start, pause, remove, stop and poll. Its SDK path deliberately
does not exist, so it never connects to an account. The suite currently has 25
Rust desktop tests and three QML checks, plus the headless and FUSE checks.

**Test isolation correction:** service state uses `XDG_STATE_HOME`, not
`XDG_DATA_HOME`. The earlier headless/FUSE scripts set the latter incorrectly.
They now set and verify the former. Two verified synthetic `abc` caches created
by those earlier runs were moved out of the user's foreground cache into
`/tmp/nd-isolation-test-cache-_92vvawy`; existing service-file timestamps predated
the tests. The corrected full run left user-state file lists, sizes and mtimes
unchanged. The scripts now stop any test daemon before removing its temporary
state, retaining that directory if cleanup fails.

## Native mount and local-trash pages

The Rust controller now owns mount-folder selection, remote/offline/write options,
mount/unmount/refresh/login/pin/evict actions, recovery notices and saved mount
configuration display. Mount status polling uses the independently owned status
worker and the same stale-reply protection as background sync. Snap and demo
mount restrictions are retained. Requests use the native desktop-bus launcher.

Local-trash listing and restoration now go directly to the Rust journal runtime.
Only the application's own journal is accepted. A nested unsuccessful/cancelled
restore result is propagated as failure; only a successful restore triggers the
success message and list refresh. Missing/restored entries cannot be restored by
the UI, while prepared/restoring/recovery entries retain the recovery flow.

Thirty desktop Rust tests and four QML checks now pass. A real local-trash test
creates a synthetic receipt, cancels a restore, verifies a colliding destination
and the original payload both survive, then retries successfully after removing
only the test's collision file. A repeated restore is rejected without changing
the recovered content. `MountCheck.qml` drives actual private-bus/FUSE mount,
pin, managed eviction, re-pin, unmount and offline re-mount through the Qt
controller. The fixture records exactly two downloads; the offline pin does not
download again. The test checks retained content and no remaining kernel mount.

The initial mount-check expectation that eviction of a pinned file would fail
was corrected: the existing mount manager explicitly unpins before evicting.
No production behavior was changed to fit that incorrect expectation. All state
and services remain isolated, and user service/foreground file lists, sizes and
mtimes were checked unchanged after the full run. Main-window wiring, updater,
autostart/window lifecycle, package builds and Python removal remain unfinished.


## Native autostart UI checkpoint

The native file controller now routes autostart enable/disable to Rust. Flatpak
requests the desktop portal and treats denial as failure. Snap uses its stable
launcher without retaining an SDK path from a previous revision. AppImages use
the persistent outer executable; an SDK inside APPDIR is rediscovered at login,
while a custom external SDK is retained. Native runs use the current executable.
Existing Rust managed-entry protection and Desktop Exec escaping remain in use.
The legacy Python launcher fallback is never called by this native controller.

Validation: 34 desktop Rust tests, four QML checks (including native autostart
enable/disable), Clippy with warnings denied, formatting, headless service checks
and online/offline synthetic FUSE checks pass. Autostart writes use an isolated
XDG_CONFIG_HOME, and all daemon/mount state uses isolated XDG_STATE_HOME. User
service/foreground file names, sizes and mtimes match the prior baseline.

The complete application migration remains active. Production Python frontend,
update controller, window lifecycle, main-window wiring, portable packaging and
final parity checks are still outstanding. No release, installation or live
account test was performed at this checkpoint.


## Native update-controller checkpoint

The Rust update model and Qt controller now handle saved automatic/download/preview
preferences, release-channel defaults, the 24-hour check cadence, manual checks,
verified downloads, release notices and explicit installation. The existing Rust
HTTPS/update runtime remains authoritative for selection, downloads and digest
verification; nested errors cannot become UI success. QSettings uses the existing
INI keys. Package format still accepts the installed payload directory from the
future main-window bootstrap.

Download results own their cleanup, including queued results discarded at close.
An explicit install re-verifies the package, reads its Package field with bounded,
cancellable dpkg-deb output and rejects anything other than neutrondrive before
invoking pkexec/apt-get. Once the privileged transaction starts it is not killed;
shutdown returns false while installation is active so the future window binding
can defer close. This must be honored when integrating the actual main window.
A running file operation prevents installation. Failure retains the download for
retry, while success removes it. Automatic download never starts installation.

Validation: 43 Rust desktop tests, five QML checks, Clippy with warnings denied,
format checks and the existing isolated service/online/offline FUSE tests pass.
A synthetic DEB is built and inspected without installing it. The QML test covers
translated status, saved/reloaded options, demo guards and repeated shutdown.
No live release request or privileged installation was performed. User service
and foreground-cache file names, sizes and mtimes remain unchanged.

Two compile issues (Rust pinned-borrow lifetimes and a generated binding type-name
collision) were corrected. A subsequent /tmp user-quota failure was resolved by
removing only regenerable task-owned Bun cache, a duplicate Qt SDK copy and a
failed linker temporary. The successful final log is
/tmp/nd-qt-updates-final-2.log. The Qt SDK still in use is /tmp/nd-qt-sdk.

The overall goal remains active: main-window/controller wiring, update timers,
window/single-instance lifecycle, Qt translations, installed resource discovery,
portable builds/package wiring, full parity tests and production Python removal
are still outstanding. No release or local installation was made.

The shared application view is now embedded through `ui.qrc` and exercised with
`--check-window CATALOG_DIRECTORY [SCREENSHOT_DIRECTORY]`. This is still a demo
validation entrypoint, not the final production bootstrap. It drives all six
existing pages with native controllers, changes themes, opens/cancels a folder
dialog and checks actual window-close events. The validation script also captures
light/dark views and requires process exit 0. Current result: 45 Rust tests and
six QML checks pass, with both rendered captures inspected. See the main-window
checkpoint in `docs/RUST-MIGRATION.md` for remaining integration work and the
corrected window-parenting/shutdown issues.


## Native GUI bootstrap and window-instance checkpoint

The native executable now accepts regular GUI startup, --demo and --cli PATH.
The headless modes still dispatch before constructing QGuiApplication. GUI
startup resolves installed-prefix or source-checkout catalogs, preserves explicit
SDK/environment/PATH selection, displays settings when the SDK is unavailable,
sets the existing desktop-file identity and retains Qt translators for qtbase
and qtdeclarative. Missing graphical sessions fail clearly with exit status 1.

A Rust window-instance owner retains the existing QLockFile format and per-scope
qt-window-{demo,flatpak,snap,native} socket paths. It rejects symlinked/non-private
directories and foreign socket-path objects; removes only an owned stale socket;
and checks socket identity before cleanup. Activation connects are nonblocking
with a bounded retry, and the Qt timer drains a bounded number of pending
connections to show/raise/activate the existing window. Socket cleanup precedes
lock release. Installed resource paths take precedence over source fallback.

Validation: 51 Rust tests, six QML checks and all existing native service/FUSE
checks pass. A separate process test starts the regular --demo entrypoint,
contacts it from a second process, kills only that synthetic demo owner to
simulate a crash, and verifies stale-lock recovery followed by clean window
shutdown. Symlinks, public directory permissions, regular-file socket collisions
and replacement-path preservation have dedicated tests. No account or installed
package was used. The user service/foreground state file list, sizes and mtimes
remain unchanged. Final log: /tmp/nd-qt-startup-resources-final.log.

Production/package launchers still use Python, and portable frontend/core builds,
package dependency changes, complete integration parity, localized launcher
errors and final Python removal remain outstanding. The overall goal is active.


## Python-free package payload preparation

The Qt extractor now has an explicit native mode: it stages Qt libraries,
plugins, QML and translations without PySide6/Shiboken bindings or Python files.
The original mode remains intact until package migration is complete. A native
shell launcher supplies relocatable Qt/SDK paths, preserves literal arguments
and executes the Rust binary. Native staging requires a fresh destination,
retains the locale payload identity used by update detection, and includes the
Rust/Unicode/Qt notices. Native sandbox staging deliberately fails until the
browser-portal helper is migrated, rather than silently shipping Python.

`packaging/native_frontend.py` verifies the actual ELF architecture, GLIBC and
GLIBCXX requirements, unwanted Python/separate-core dependencies, binary hash and
first-party source provenance before staging. The host development binary requires
GLIBC 2.39 and is correctly rejected for the current portable baseline. No
portable artifact or release has been produced.

Five new packaging tests and five existing AppImage-cache tests pass. A separate
host-only fixture built from the pinned Qt wheels contains no Python runtime
files or Python Qt bindings. Its native launcher successfully ran the full
main-window check with a PATH containing only readlink/dirname and the native
application directory; the process exited 0. The fixture is marked
HOST-VALIDATION-ONLY.json and is not a distributable package. Log:
/tmp/nd-native-payload-window.log.

Next requirements are the portable frontend build, Rust browser-portal and
AppImage cache/bootstrap helpers, architecture verification and the default
package/launcher/dependency switch. Production launchers and the installed app
remain unchanged. The overall migration goal remains active.


## Native browser-portal helper checkpoint

Rust now implements the desktop OpenURI request and the headless --open-uri URL
entrypoint. Only bounded valid HTTP(S) addresses are accepted; whitespace,
controls, backslashes and unsupported schemes are rejected. Fragments are
preserved. The client subscribes before issuing the request, checks the reply's
unique sender and returned request path, ignores unrelated signals, and treats
denial, invalid paths and disconnection as failure. The existing bounded runtime
limits waiting to 120 seconds and supports cancellation.

Native sandbox staging now selects xdg-open-portal-native instead of the Python
Gio helper. Its shell wrapper passes a single literal URL to the Rust executable,
sets the package Qt-library path and rejects wrong argument counts. Legacy
staging remains unchanged until the complete package switch.

Validation: 162 core tests, the isolated D-Bus suite (11 scenarios, including
browser success/spoof/denial/invalid-path/cancellation), 52 desktop Rust tests,
six QML checks, the existing service/FUSE/window-activation checks and six native
packaging tests pass. Core and desktop Clippy deny warnings. The browser helper
rejects a file URL without constructing QGuiApplication, tested with an invalid
Qt platform and no display. No actual browser/account request, release or
installation occurred. User service/foreground file names, sizes and mtimes are
unchanged. Logs: /tmp/nd-portal-rust-tests.log,
/tmp/nd-portal-core-clippy.log and /tmp/nd-native-browser-qt-tests.log.

Remaining requirements include the Rust AppImage stable cache/bootstrap,
portable/architecture builds, and package/launcher/dependency replacement.
Installed autostart must also resolve its Qt libraries without inheriting a
launcher environment (for example a verified relative ELF RUNPATH). The updated
core invalidates earlier portable-core provenance; rebuild it before final
package checks. The continuous goal remains active.

The native executable also owns AppImage cache bootstrap (`--appimage-bootstrap`)
and embeds a relative Qt library RUNPATH for direct service/autostart startup.
`check-native-qt` tests both with isolated state/cache directories and no Python
available to the application subprocess. The build-time fixture scripts use
Python; they are not application dependencies. Production AppImage packaging
still awaits the portable native artifact and builder switch.
