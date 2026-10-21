# Validation record — 0.6.0 alpha2

Previous release evidence: [0.6.0 alpha1](VALIDATION-0.6-alpha1.md).

This is a small prerelease increment, not a new minor release. The interface
retains Qt Quick/QML, with a light palette, grouped navigation, original line
icons and the existing seven pages and actions.

Rust now plans remote metadata-cache invalidation: old/new parents, descendants,
root aliases, unknown-event fallback and resets. Python retains locking, cache
storage and generation checks. A core error discards stale listings before it
propagates. The pure policy is compared against the previous implementation on
500 seeded graphs, including cycles, aliases and unrelated subtrees.

The native-library build now replaces the output inode atomically, preserving
libraries already mapped by running applications. A mapping regression test
covers this after a build/test overlap caused an initial local test abort.

Release verification is in progress. Final counts and package evidence will be
recorded here before publication. Existing alpha/platform limitations remain.
