# Import photo folders and Google Photos from Takeout

NeutronDrive can upload photos and videos from an **extracted Google
Takeout export** into Proton Photos. It keeps original media bytes unchanged and
uses matching Google JSON sidecars for capture time and location when available.

## Folder import (unreleased)

Open **Photos → Photo import → Import a folder** to select ordinary photo and
video folders. Review the scan before uploading. Each media-containing folder
becomes an album named from the selected root and relative folder names, joined
with spaces. Identical local media is uploaded once and assigned to each relevant
album. Existing private albums with an exact, unique name are reused. Shared or
ambiguous album names stop assignment instead of modifying those albums silently.

The same folder-to-album mapping applies to extracted Takeout folders. This
reconstructs folder organization, not Google's separate album metadata. Names
that would collapse different folder paths into the same album name are rejected
before uploading. Limits are 10,000 albums and 100 memberships per unique photo.
Source media is never modified or removed.

## Prepare the export

1. Export Google Photos using Google Takeout.
2. Download **every archive part**. Extract ZIP/TGZ files yourself, combining their
   folders into one export folder. Keep the JSON sidecars beside the media files.
3. Keep the original export until you have checked the imported photos in Proton.
   NeutronDrive does not delete source files.

NeutronDrive does not extract archives. Symbolic links and special files are
ignored. The scanner accepts common photo/video extensions; actual decoding,
preview and upload support also depend on the Proton SDK.

## Review and import

1. Sign in to the intended Proton account.
2. Open **Photos → Photo import → Google Takeout import** and choose the extracted export folder.
3. Review the media/video count, duplicate count, ignored files and metadata
   warnings. Nothing is uploaded before you confirm.
4. Confirm to upload into **Proton Photos**. Keep NeutronDrive running; closing to
   the tray is fine. Accept the refresh prompt afterward to see new uploads.

The scanner reads the source files to calculate hashes, so scanning a large
export can take time. Identical content within the export is grouped. Existing
Proton Photos are checked by name/content through the SDK before upload; this is
not a global duplicate finder for differently named copies.

## Metadata and interruptions

Exact JSON sidecar names are preferred, including `.supplemental-metadata.json`.
A unique title match can associate truncated sidecar filenames. Missing or
ambiguous JSON metadata falls back to the SDK's embedded metadata handling and
may produce a warning when malformed or ambiguous. Ordinary folders need no JSON.
Valid capture dates and GPS coordinates are imported; comments, descriptions and
edited/original relationships are not rebuilt.

You can cancel the import. If a file fails, the batch stops and reports uploaded,
skipped, failed and remaining counts. Item progress covers the complete batch;
file transfer progress remains separate. Album assignment errors are reported
separately from upload failures, retaining any successfully uploaded media. A changed source file or account also stops
the operation. You can select the same export again; matching completed uploads
are checked before another upload. There is no persistent import queue or automatic
resume after quitting/restarting the app.

For large exports, work in batches: the scanner limits a selection to 100,000
unique media files, 500,000 directory entries and 64 directory levels. JSON limits
are 1 MiB per file and 64 MiB per directory.

The release includes synthetic importer and SDK contract tests. A real-account
Takeout upload has not been validated for this release; verify a small batch first.
