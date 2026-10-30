# Desktop update checks

Settings → Updates offers a manual check and an optional automatic check at
startup (after a short delay) and every two hours while the GUI is running.
Automatic checks are enabled by default; disabling them stops scheduled requests.
No separate service is installed. A persisted last-attempt time does not suppress
the first check in a new app session. Checks never overlap downloads or installation.
Only a newer available release produces a notification, once per version per app
session. No-update results and network failures stay silent outside Update settings.
Manual checks can be retried at any time. Demo mode never checks.

The checker contacts the public GitHub releases endpoint for
`SigmaCodez/NeutronDrive`, without authentication or account/file data. GitHub
receives the normal connection metadata, including the client's IP address.
It examines the most recent 100 releases; errors, rate limits and offline
conditions are reported instead of being interpreted as an up-to-date result.

Alpha builds include prereleases by default. A checkbox selects stable-only
releases instead. Versions are compared numerically, including alpha/beta/rc
ordering; local alpha6 packaging revisions do not count as a new app version.
Draft releases, unknown version formats, older releases and foreign release
URLs are ignored.

## Native DEB installations

When running the native DEB installation, a matching amd64/arm64 release asset
can be downloaded manually or automatically (automatic downloads default off).
Downloads have size/time limits and must match GitHub's SHA-256 digest and
expected size. Partial or corrupt files are removed. The digest is checked
again before installation. This protects against corruption, but is not an
independent publisher signature: GitHub/repository access remains the trust root.
API reference: https://docs.github.com/en/rest/releases/releases

**Install update** invokes `pkexec apt-get install --no-remove` with an argument
list and the verified local file. The desktop asks for administrator approval;
no password is handled by NeutronDrive. Installation errors or cancellation
leave the package available to retry. The GUI is not automatically restarted.
Finish running synchronizations before restarting. Downloaded packages are
session-local and cleaned up on a normal exit; after a crash, leftover files
may remain in the application's updates cache. A later session downloads anew.

## Other installation formats

Flatpak, Snap, AppImage, RPM, Arch and source installations receive release
notifications and a link to the release page. This version does **not** replace
these installations automatically. Use their package manager or the appropriate
release artifact; it never installs a DEB over a sandboxed/portable installation.
GitHub-hosted Flatpak/Snap bundles are not a configured store update channel.

The new updater labels have German and English text; other UI languages
currently use English for these additional labels. Existing translations remain.

These controls are included in alpha7 native application packages.
