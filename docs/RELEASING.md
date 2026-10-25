# Release maintenance

For every release, update the GitHub-facing text and GUI previews together.
This is a project publishing requirement, not an optional cosmetic follow-up.

1. Use a small version increment and update package metadata consistently.
2. Run the native architecture tests and installed-package checks. Preserve the
   exact build commit, source/binary provenance and artifact checksums.
3. Update the English README, feature guides, installation commands, package
   coverage, known limitations and release notes to describe the shipped version.
   Keep historical validation evidence clearly identified as historical.
4. Capture new previews from the actual release executable with English selected
   in Settings. Include the light and dark file views, automatic sync/computer
   backup and online drive. Refresh the README gallery and its light/dark settings and setup previews on every release;
   do not reuse older screenshots just because the layout appears similar.
5. Use an isolated profile and clearly disclosed synthetic data. Never expose
   account details, personal filenames or live synchronization data. Use English
   sample filenames as well as English UI labels. Capture the rendered GUI
   directly; do not redraw controls or edit translated labels into an old image.
6. Visually inspect every image for the correct language, current version,
   current navigation, readable text and absence of private data. Update
   `GUI-PREVIEWS.json` with the executable version/hash, build commit, capture
   context and image hashes, and keep README captions accurate.
7. Update release notes with the current English preview and links to other
   current views. Pin published SDK and Arch recipe downloads to actual hashes.
   Do not publish an AUR listing without an explicitly configured account.
8. Verify the uploaded assets before publishing. For local upgrades, finish
   transfers first, use the system authorization dialog, preserve existing
   configuration and verify the installed native version and isolated GUI.

The screenshot files are `docs/demo.png`, `docs/demo-dark.png`,
`docs/background-demo.png` and `docs/ondemand-demo.png`. These stable filenames
are replaced by new captures; the metadata records which release they show.

## Repository presentation

Keep the README, About description and topics in English. Show badges for the
actual license, runtime, interface and CI status; do not borrow another project's
license or imply upstream affiliation. The release badge includes prereleases.
Document development captures separately from published versions, and keep
installation examples tied to downloadable release assets. GitHub language
statistics exclude third-party license documentation and vendored dependencies
through `.gitattributes`; maintained build/test code remains visible.

## Automatic homepage release bundle

The `Publish release website content` workflow runs on every published GitHub
release (stable or prerelease). It can also be rerun with `workflow_dispatch`
and the existing release tag, or called as a reusable workflow with `tag`.
If a different Actions workflow publishes releases using GITHUB_TOKEN, it must
call this reusable workflow explicitly: GitHub does not start another workflow
from that token’s release event. It checks out the exact release source, verifies
its published amd64 DEB against SHA256SUMS, checks the executable version, and
captures the actual packaged Qt interface in isolated English profiles using
synthetic data. No installed user profile or live Proton account is used.

Before tagging, update `docs/website-content.json` for the exact new release tag.
All five locales (en, de, fr, es, it) must contain description, features, FAQ,
trust notes, gallery captions and releaseNotes. These are maintained translations,
not machine-invented release claims. The fast `Website release manifest checks`
workflow checks completeness on pushes and pull requests. A stale release tag,
missing translation, mismatched package checksum/version, missing PNG or failed
GUI check prevents publication of the homepage bundle.

The generated `neutrondrive-website.json` release asset includes all text, five
base64 PNGs, PNG hashes, package/executable hashes, exact source commit, language
and capture time. It is also retained as a workflow artifact. One asset avoids
partial text/image updates. It is generated after the package release, so a
failed website job does not delete or unpublish packages. Correct the source
before tagging a new release, or rerun transient capture failures for the same tag.
Immutable GitHub releases must have this asset attached before locking the release;
post-publication asset uploads require an editable release.

The server polls every 15 minutes and prefers this bundle over docs in the tag.
It validates the commit, all languages and screenshot hashes before switching its
live snapshot. Until the bundle is available, original release notes update via
the existing fallback and previous screenshots retain their original version.
Changing a release tag or replacing its package just to fix website text is not
part of this workflow.
