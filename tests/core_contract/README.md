# Test-only native core contract API

These Python adapters are retained only for regression fixtures and differential
checks of the Rust core. They are not an application package, have no console
entrypoint, and are never staged into installation packages. Their C ABI calls
execute the Rust implementation; reference test scheduling/callback objects
allow the existing fault-injection and released-policy comparisons to remain.
The production event loops, sync daemon, FUSE service, Qt controllers and update
workers run in `core/` and `desktop/`, not in these test objects.

The former Python GUI, update controller, appearance watcher, window activation
and demo controller were deleted after native checks replaced their UI tests.
Language/QML resources remain under `src/neutrondrive/` without Python modules.
The opt-in `scripts/smoke*-live` helpers are test tools and import this contract
API from the checkout; they are never installed or run automatically.

Run contract tests with `PYTHONPATH=tests` and `NEUTRONDRIVE_CORE_LIBRARY` set to
the freshly built core shared library. Native UI tests require the matching
Rust frontend executable. Passing only these compatibility tests is insufficient:
`scripts/check-native-qt` and the core executable tests exercise the actual
application process/lifecycle without Python on PATH.
