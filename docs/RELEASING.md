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
   backup and online drive. Replace all four README images on every release;
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
