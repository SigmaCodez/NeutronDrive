# Contributing

Use Python 3.10+; core functionality uses only the standard library. GTK is an
OS dependency. Keep network and filesystem transfers off the GTK main thread.
Do not introduce custom cryptography, password collection, shell interpolation,
silent overwrite policies or broad periodic remote-tree scans.

```sh
python3 -m unittest discover -s tests -v
python3 -m compileall -q src packaging scripts
python3 bin/neutrondrive --demo
```

Unit tests never access an account. For an authorized disposable test account,
log in through the browser and explicitly opt in to the real contract test:

```sh
python3 scripts/smoke-live --live --cli /absolute/path/to/proton-drive
```

It creates a uniquely named synthetic folder, tests transfer/rename/move/sharing/
trash/restore/manual sync, and moves that folder to trash afterward. It never
permanently deletes account data or prints credentials/share URLs. Do not run it
against an account without its owner's consent.

Every backend update must refresh `packaging/backend-lock.json`, architecture
checksums, included license notices, the Flatpak manifest and the compatibility
record. Test actual released binaries, not only an unreleased GitHub branch.
Validate packaged launchers, browser login and keyring access in each sandbox.
Build success alone is not installation or account-integration validation.

Changes involving file content must include regression tests for conflicts,
concurrent updates and interrupted operations. UI wording must distinguish demo
data, verified success, skipped items and incomplete operations. Public reports
must contain synthetic data only.
