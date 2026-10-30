# Photo folder import and gallery fixes — unreleased

Validated 2026-10-30 against the pinned Proton SDK and Bun 1.4.2.

## Changes

- Ordinary photo/video folder import, with review before upload and folder-derived
  albums; extracted Takeout imports use the same album mapping.
- Identical local media retains all folder album memberships without uploading
  each local copy. Remote duplicate detection remains SDK name/content based.
- Existing private albums can be reused; shared, renamed or ambiguous targets
  fail safely. Partial album failures preserve successful uploads and are visible.
  Missing membership confirmation is also a failure, even after upload succeeds.
- Whole-import item progress, result counts, and a gallery refresh prompt.
- The photo viewer requests the next page when moving beyond loaded photos.
- Removing selected photos from an album targets the open album, independently
  of the destination album selected for other operations.
- Album labels tolerate temporarily missing display names without a QML warning.

## Validation

- Rust core: 233 tests passed (212 unit tests and 21 integration tests).
- Rust desktop: 129 tests passed.
- SDK bridge: 28 tests, 107 assertions passed across six files; TypeScript check passed.
- Core and desktop Clippy passed with warnings denied; Rust formatting checked.
- Actual offscreen Qt window interactions passed, including continuous photo
  pagination, preview retry cooldown, selection identity, viewer page-boundary
  continuation and the album-removal target. Synthetic screenshots were inspected.
- UI tests used isolated XDG directories and a private D-Bus session; no personal
  library, session, or sync service was modified.

These are isolated automated checks, not a real-account import validation or a
claim of complete Windows feature parity. No release or local package installation
was performed for this change. New UI messages have German and English text;
other language catalogs currently use English for these new messages.
