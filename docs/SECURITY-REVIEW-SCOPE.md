# External security review scope: community sync controls

Status: **prepared for external review; no independent audit performed**.
Internal implementation/testing by the same development process is not an
independent security assessment. No reviewer has been appointed, no report has
been commissioned and no user/account data has been sent to an external party.

## Proposed review boundaries

Review the source diff that adds `sync_options.rs`, `transfer_limits.rs`, the
scoped filesystem/remote scans, daemon scheduling, settings dialogs and the SDK
`transferLimits.ts` stream wrapper. Review the pinned upstream SDK source and
patch/provenance records when assessing the complete transfer boundary.

1. Exclusion rules must not become deletion evidence. Inspect saved baselines,
   pending deletion recovery, nested paths, remote duplicate names, symlinks,
   moves/renames and changes made on both sides while excluded.
2. Configuration changes must pause and drain active work. Test stale dialog
   identities, job removal/replacement, persisted-state replacement and restart
   between configuration, reload and resume.
3. Schedules must preserve manual pauses, persisted missed deadlines, retry
   backoff and UTC boundaries. Consider wall-clock rollback and forward jumps.
4. Rate settings must be bounded, private, atomically stored and fail closed on
   malformed state or unsupported backend capability. Verify all actual SDK
   file-transfer paths, retries, authentication refreshes, aborts and concurrent
   streams. Measure socket buffering separately from paced payload delivery.
5. The HTTP wrapper must preserve headers, content bytes, status and cancellation
   without logging credentials or content. Review timeout changes and the
   24-hour outer bound for deliberately slow transfers.
6. Diagnostics must not acquire excluded paths, account details or raw errors.
   Confirm new settings do not expand the existing diagnostic allowlist.

## Reproduction and evidence

Use disposable local fixtures and private XDG directories. No live-account test
is needed for the unit, synthetic SDK, local HTTP or Qt-window checks.

- `cargo test --locked --offline --lib --test runtime` in `core/` exercises the
  engine, persistent jobs and compiled native process integration.
- `cargo test --locked --offline -- --test-threads=1` in `desktop/` exercises
  captured job identities, read-only cancellation and pause-before-configure.
- `scripts/build-backend --test-only --bun /path/to/pinned/bun` runs bridge tests
  and TypeScript checking against the pinned SDK and frozen dependency lock.
- `transferLimits.test.ts` uses synthetic bytes and a loopback HTTP server;
  tests cover shared concurrent budgets, byte preservation, cancellation and
  actual upload/download pacing.
- Native Qt window fixtures cover the settings controls; screenshots use only
  synthetic accounts and paths. The broader validation record is in VALIDATION.md.

Published audit findings must identify the exact reviewed commit/artifact hashes,
reviewer, scope, method, unresolved findings and retest results. A build or unit
suite passing must not be described as an audit or proof of end-to-end security.
