# Validation record — 0.6.0 alpha4

Previous evidence: [0.6.0 alpha3](VALIDATION-0.6-alpha3.md).

Daemon cloud-event routing now runs in Rust. Decisions include job relevance,
ready/reset/error handling, unknown-parent lookup eligibility, deduplication and
bounded queue overflow. The Python host retains the condition lock, applies the
returned decision, invalidates caches and persists status before returning the
ACK. Policy failure occurs before host state is changed.

The previous routing policy is frozen only as a test oracle. 1,500 seeded cases
compare multiple jobs, paused states, known/unknown node and parent identities,
root matches, reset/error/ready and queued duplicates. Separate tests exercise
the 1,024-entry limit, overflow, duplicate jobs, malformed identities and a Rust
failure before scheduling/persistence. Final test and package results are pending.

No additional C layer was added: no measured performance benefit justified one.
Rust policy, the existing C ABI and guarded Python I/O remain the architecture.
No claim of full Rust migration or measured speed improvement is made.
